// ncz-rs patch: writes the entities of a drawing as they are made, so they need not be held in memory at once.

use std::io::Write;

use crate::code_pair_writer::CodePairWriter;
use crate::entities::{Entity, EntityType};
use crate::enums::AcadVersion;
use crate::header::Header;
use crate::tables::TABLE_COUNT;
use crate::{CodePair, Drawing, DxfError, DxfResult, Handle};

/// Number of handles that [`EntityStreamWriter::write_entity`] gives the entity: its own, and those of the vertices,
/// attributes and `SEQEND` that follow a polyline or an insert.
pub fn handles_needed(entity: &Entity) -> u64 {
    1 + match entity.specific {
        EntityType::Polyline(ref poly) => 1 + poly.__vertices_and_handles.len() as u64,
        EntityType::Insert(ref ins) => 1 + ins.__attributes_and_handles.len() as u64,
        _ => 0,
    }
}

/// Writes the entities section of a drawing one entity at a time. Made by [`Drawing::save_streaming`].
pub struct EntityStreamWriter<'d, 'w, W: Write + ?Sized> {
    drawing: &'d Drawing,
    writer: CodePairWriter<'w, W>,
    write_handles: bool,
    next_handle: u64,
    end_handle: u64,
    pairs: Vec<CodePair>,
}

impl Drawing {
    /// Adds the layer, line type, text style and other table entries that the entity refers to, as
    /// [`Drawing::add_entity`] does, without adding the entity. Every entity passed to
    /// [`EntityStreamWriter::write_entity`] must have been prepared, since the tables are written before it.
    pub fn prepare_entity(&mut self, entity: &Entity) {
        self.ensure_entity_tables(entity);
    }

    /// Writes the drawing as ASCII up to and including its own entities, and returns a writer for further entities.
    /// They take the handles after those of the drawing and its tables, and `reserved_handles` of them are set aside,
    /// as [`handles_needed`] counts them.
    pub fn save_streaming<'d, 'w, W: Write + ?Sized>(
        &'d self,
        writer: &'w mut W,
        reserved_handles: u64,
    ) -> DxfResult<EntityStreamWriter<'d, 'w, W>> {
        let version = self.header.version;
        let write_handles = version >= AcadVersion::R13 || self.header.handles_enabled;
        let first_table_handle = self.header.next_available_handle;
        let first_entity_handle = first_table_handle.0 + TABLE_COUNT;
        let end_handle = first_entity_handle + reserved_handles;

        let mut pairs = vec![];
        let mut header: Header = self.header.clone();
        header.next_available_handle = Handle(end_handle);
        header.add_code_pairs(&mut pairs);
        self.add_classes_pairs(&mut pairs);
        self.add_tables_pairs(&mut pairs, write_handles, first_table_handle);
        self.add_blocks_pairs(&mut pairs, write_handles);
        pairs.push(CodePair::new_str(0, "SECTION"));
        pairs.push(CodePair::new_str(2, "ENTITIES"));
        for e in self.entities() {
            e.add_code_pairs(&mut pairs, version, write_handles);
        }

        let text_as_ascii = version <= AcadVersion::R2004;
        let mut writer = CodePairWriter::new(writer, true, text_as_ascii, version);
        writer.write_prelude()?;
        for pair in &pairs {
            writer.write_code_pair(pair)?;
        }
        pairs.clear();

        Ok(EntityStreamWriter {
            drawing: self,
            writer,
            write_handles,
            next_handle: first_entity_handle,
            end_handle,
            pairs,
        })
    }
}

impl<W: Write + ?Sized> EntityStreamWriter<'_, '_, W> {
    fn next_handle(&mut self) -> DxfResult<Handle> {
        if self.next_handle >= self.end_handle {
            return Err(DxfError::IoError(std::io::Error::other(
                "more entity handles were needed than were reserved",
            )));
        }
        let handle = Handle(self.next_handle);
        self.next_handle += 1;
        Ok(handle)
    }

    /// Gives the entity its handles and writes it. Handles it already has are replaced.
    pub fn write_entity(&mut self, mut entity: Entity) -> DxfResult<()> {
        entity.common.handle = self.next_handle()?;
        match entity.specific {
            EntityType::Insert(ref mut ins) => {
                ins.__seqend_handle = self.next_handle()?;
                for a in ins.__attributes_and_handles.iter_mut() {
                    a.1 = self.next_handle()?;
                }
            }
            EntityType::Polyline(ref mut poly) => {
                poly.__seqend_handle = self.next_handle()?;
                for v in poly.__vertices_and_handles.iter_mut() {
                    v.1 = self.next_handle()?;
                }
            }
            _ => (),
        }

        entity.add_code_pairs(
            &mut self.pairs,
            self.drawing.header.version,
            self.write_handles,
        );
        for pair in &self.pairs {
            self.writer.write_code_pair(pair)?;
        }
        self.pairs.clear();
        Ok(())
    }

    /// Writes the rest of the drawing after the entities.
    pub fn finish(mut self) -> DxfResult<()> {
        let mut pairs = vec![CodePair::new_str(0, "ENDSEC")];
        self.drawing.add_objects_pairs(&mut pairs);
        self.drawing.add_thumbnail_pairs(&mut pairs)?;
        pairs.push(CodePair::new_str(0, "EOF"));
        for pair in &pairs {
            self.writer.write_code_pair(pair)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::entities::*;
    use crate::enums::AcadVersion;
    use crate::tables::Layer;
    use crate::*;
    use std::collections::HashSet;

    fn entities(drawing: &mut Drawing) -> Vec<Entity> {
        let mut line = Entity::new(EntityType::Line(Line::new(
            Point::origin(),
            Point::new(1.0, 1.0, 0.0),
        )));
        line.common.layer = "streamed".to_string();
        line.common.x_data.push(XData {
            application_name: "ACAD".to_string(),
            items: vec![XDataItem::Str("value".to_string())],
        });
        let mut polyline = Polyline::default();
        polyline.add_vertex(drawing, Vertex::new(Point::origin()));
        polyline.add_vertex(drawing, Vertex::new(Point::new(2.0, 0.0, 0.0)));
        let polyline = Entity::new(EntityType::Polyline(polyline));
        vec![line, polyline]
    }

    fn handles(text: &str) -> (Vec<u64>, u64) {
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        let pairs: Vec<(&str, &str)> = lines.chunks(2).map(|c| (c[0], c[1])).collect();
        let parse = |h: &str| u64::from_str_radix(h, 16).unwrap();
        let seed_at = pairs.iter().position(|&p| p == ("9", "$HANDSEED")).unwrap();
        let handles = pairs
            .iter()
            .enumerate()
            .filter(|&(i, p)| (p.0 == "5" || p.0 == "105") && i != seed_at + 1)
            .map(|(_, p)| parse(p.1))
            .collect();
        (handles, parse(pairs[seed_at + 1].1))
    }

    #[test]
    fn streamed_entities_match_saved_ones() {
        let mut saved = Drawing::new();
        saved.header.version = AcadVersion::R2000;
        saved.add_layer(Layer {
            name: "streamed".to_string(),
            ..Default::default()
        });
        for entity in entities(&mut saved) {
            saved.add_entity(entity);
        }
        let mut saved_text = vec![];
        saved.save(&mut saved_text).unwrap();

        let mut streamed = Drawing::new();
        streamed.header.version = AcadVersion::R2000;
        let mut scratch = Drawing::new();
        let to_stream = entities(&mut scratch);
        for entity in &to_stream {
            streamed.prepare_entity(entity);
        }
        let reserved = to_stream.iter().map(stream_writer::handles_needed).sum();
        let mut streamed_text = vec![];
        let mut writer = streamed.save_streaming(&mut streamed_text, reserved).unwrap();
        for entity in to_stream {
            writer.write_entity(entity).unwrap();
        }
        writer.finish().unwrap();

        // The same entities and tables come back; only the handles differ.
        let saved = Drawing::load(&mut saved_text.as_slice()).unwrap();
        let reloaded = Drawing::load(&mut streamed_text.as_slice()).unwrap();
        let summary = |d: &Drawing| {
            let mut entities: Vec<String> = d
                .entities()
                .map(|e| format!("{:?} {:?} {}", e.specific, e.common.x_data, e.common.layer))
                .collect();
            entities.sort();
            let mut layers: Vec<String> = d.layers().map(|l| l.name.clone()).collect();
            layers.sort();
            (entities, layers)
        };
        assert_eq!(summary(&saved), summary(&reloaded));

        let (handles, seed) = handles(std::str::from_utf8(&streamed_text).unwrap());
        assert!(handles.iter().all(|&h| h != 0 && h < seed));
        assert_eq!(handles.len(), handles.iter().collect::<HashSet<_>>().len());
    }

    #[test]
    fn stops_when_the_reserved_handles_run_out() {
        let drawing = Drawing::new();
        let mut text = vec![];
        let mut writer = drawing.save_streaming(&mut text, 1).unwrap();
        let line = || Entity::new(EntityType::Line(Line::new(Point::origin(), Point::origin())));
        assert!(writer.write_entity(line()).is_ok());
        assert!(writer.write_entity(line()).is_err());
    }
}
