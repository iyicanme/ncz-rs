//! A short description of an NCZ file and of its conversion to DXF.

use crate::coordinate::Coordinate;
use crate::dxf_export::{is_exported, write_dxf};
use crate::geometry::Geometry;
use crate::object::{MetadataContent, Object};
use std::collections::BTreeMap;
use std::io::Write;

#[derive(Debug, Default, serde::Serialize)]
pub struct FileSummary {
    pub version: Option<String>,
    pub user: Option<String>,
    /// Map scale divided by 1000.
    pub scale: Option<f64>,
    /// The coordinate system from the `MPROJ` entry, with its EPSG code when one is known.
    pub coordinate_system: Option<String>,
    pub lower_bound: Option<Coordinate>,
    pub upper_bound: Option<Coordinate>,
    /// Number of geometries of each kind.
    pub geometries: BTreeMap<&'static str, usize>,
    pub layers: usize,
    pub line_types: usize,
    pub blocks: usize,
    /// Number of objects that could not be decoded.
    pub unreadable: usize,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct DxfSummary {
    /// Size of the DXF file in bytes.
    pub size: usize,
    /// Number of entities of each DXF type, in the model space and in blocks.
    pub entities: BTreeMap<&'static str, usize>,
    pub layers: usize,
    pub line_types: usize,
    pub blocks: usize,
    /// Number of geometries of each kind that have no DXF counterpart.
    pub skipped: BTreeMap<&'static str, usize>,
}

#[derive(Debug, serde::Serialize)]
pub struct Summary {
    pub file: FileSummary,
    pub dxf: DxfSummary,
}

/// Counts the bytes written through it.
struct CountingWriter<'a, W: Write> {
    inner: &'a mut W,
    count: usize,
}

impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.count += written;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Converts an NCZ file to DXF, writes it and describes both.
pub fn convert(
    bytes: &[u8],
    writer: &mut impl Write,
) -> Result<Summary, Box<dyn std::error::Error>> {
    let mut file = FileSummary::default();
    let mut skipped = BTreeMap::new();
    let mut counting = CountingWriter {
        inner: writer,
        count: 0,
    };

    let contents = write_dxf(bytes, &mut counting, |item| {
        let Ok(object) = item else {
            file.unreadable += 1;
            return;
        };
        match object {
            Object::Version { name } => file.version = Some(name.clone()),
            Object::Metadata {
                content:
                    MetadataContent::Projection {
                        projection,
                        datum,
                        central_meridian,
                        ..
                    },
                ..
            } => file.coordinate_system = coordinate_system(*projection, *datum, *central_meridian),
            Object::UserInformation { name } => file.user = Some(name.clone()),
            Object::Scale { scale } => file.scale = Some(*scale),
            Object::Extents {
                lower_bound,
                upper_bound,
            } => {
                file.lower_bound = Some(*lower_bound);
                file.upper_bound = Some(*upper_bound);
            }
            Object::Layers { layers, .. } => {
                file.layers = layers.iter().filter(|layer| !layer.name.is_empty()).count();
            }
            Object::LineTypes { types } => {
                file.line_types = types
                    .iter()
                    .filter(|line_type| !line_type.name.is_empty())
                    .count();
            }
            Object::BlockBegin { .. } => file.blocks += 1,
            // Empty geometries are placeholders without content.
            Object::Geometry {
                geometry: Geometry::Empty { .. },
            } => {}
            Object::Geometry { geometry } => {
                *file.geometries.entry(geometry.kind()).or_default() += 1;
                if !is_exported(geometry) {
                    *skipped.entry(geometry.kind()).or_default() += 1;
                }
            }
            _ => {}
        }
    })?;
    counting.flush()?;

    Ok(Summary {
        file,
        dxf: DxfSummary {
            size: counting.count,
            entities: contents.entities,
            layers: contents.layers,
            line_types: contents.line_types,
            blocks: contents.blocks,
            skipped,
        },
    })
}

/// Describes the coordinate system of an `MPROJ` entry. EPSG codes are given for the Turkish 3° zones (TUREF and
/// ED50 / TM27 to TM45), UTM zones and geographic coordinates. Datum 0 is documented as WGS 84, but some files with it
/// are named as ITRF, so it gets no code. `None` when the projection is not set.
fn coordinate_system(projection: u8, datum: u8, central_meridian: u8) -> Option<String> {
    let datum_name = match datum {
        0 => "WGS 84".to_owned(),
        1 => "TUREF (ITRF)".to_owned(),
        4 | 254 => "ED50".to_owned(),
        other => format!("datum {other}"),
    };
    let meridian = u16::from(central_meridian);
    let turkish_zone = (27..=45).contains(&meridian) && meridian % 3 == 0;
    let utm_zone = (meridian + 183) % 6 == 0 && meridian <= 177;

    let (name, epsg) = match projection {
        0 => return None,
        1 => {
            let epsg = match datum {
                0 => Some(4326),
                1 => Some(5252),
                4 | 254 => Some(4230),
                _ => None,
            };
            (format!("{datum_name}, geographic"), epsg)
        }
        2 => {
            let zone = (meridian + 183) / 6;
            let epsg = match datum {
                0 if utm_zone => Some(32600 + zone),
                4 | 254 if utm_zone => Some(23000 + zone),
                _ => None,
            };
            (format!("{datum_name} / UTM zone {zone}N"), epsg)
        }
        3 => {
            let epsg = match datum {
                1 if turkish_zone => Some(5253 + (meridian - 27) / 3),
                4 | 254 if turkish_zone => Some(2319 + (meridian - 27) / 3),
                _ => None,
            };
            (format!("{datum_name} / TM{meridian}"), epsg)
        }
        other => (
            format!("{datum_name}, projection {other}, central meridian {meridian}"),
            None,
        ),
    };

    Some(match epsg {
        Some(epsg) => format!("{name} (EPSG:{epsg})"),
        None => name,
    })
}

#[cfg(test)]
mod tests {
    use super::coordinate_system;

    #[test]
    fn names_coordinate_systems() {
        assert_eq!(coordinate_system(0, 0, 0), None);
        assert_eq!(
            coordinate_system(3, 1, 33).as_deref(),
            Some("TUREF (ITRF) / TM33 (EPSG:5255)")
        );
        assert_eq!(
            coordinate_system(3, 4, 27).as_deref(),
            Some("ED50 / TM27 (EPSG:2319)")
        );
        assert_eq!(
            coordinate_system(3, 254, 45).as_deref(),
            Some("ED50 / TM45 (EPSG:2325)")
        );
        assert_eq!(
            coordinate_system(3, 0, 33).as_deref(),
            Some("WGS 84 / TM33")
        );
        assert_eq!(
            coordinate_system(3, 255, 33).as_deref(),
            Some("datum 255 / TM33")
        );
        assert_eq!(
            coordinate_system(2, 0, 33).as_deref(),
            Some("WGS 84 / UTM zone 36N (EPSG:32636)")
        );
        assert_eq!(
            coordinate_system(2, 4, 39).as_deref(),
            Some("ED50 / UTM zone 37N (EPSG:23037)")
        );
        assert_eq!(
            coordinate_system(1, 1, 0).as_deref(),
            Some("TUREF (ITRF), geographic (EPSG:5252)")
        );
    }
}
