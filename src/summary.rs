//! A short description of an NCZ file and of its conversion to DXF.

use crate::coordinate::Coordinate;
use crate::dxf_export::is_exported;
use crate::geometry::Geometry;
use crate::object::Object;
use std::collections::BTreeMap;

#[derive(Debug, Default, serde::Serialize)]
pub struct FileSummary {
    pub version: Option<String>,
    pub user: Option<String>,
    /// Map scale divided by 1000.
    pub scale: Option<f64>,
    pub lower_bound: Option<Coordinate>,
    pub upper_bound: Option<Coordinate>,
    /// Number of geometries of each kind.
    pub geometries: BTreeMap<&'static str, usize>,
    pub layers: usize,
    pub line_types: usize,
    pub blocks: usize,
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

pub struct Conversion {
    pub summary: Summary,
    pub dxf: Vec<u8>,
}

/// Converts an NCZ file to DXF and describes both.
pub fn convert(bytes: &[u8]) -> Result<Conversion, dxf::DxfError> {
    let objects = crate::parse_objects(bytes);

    let mut file = FileSummary::default();
    let mut skipped = BTreeMap::new();
    for object in &objects {
        match object {
            Object::Version { name } => file.version = Some(name.clone()),
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
                file.line_types = types.iter().filter(|line_type| !line_type.name.is_empty()).count();
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
    }

    let drawing = crate::objects_to_dxf(objects);
    let mut dxf = vec![];
    drawing.save(&mut dxf)?;

    let mut entities = BTreeMap::new();
    let block_entities = drawing.blocks().flat_map(|block| block.entities.iter());
    for entity in drawing.entities().chain(block_entities) {
        *entities.entry(entity_name(&entity.specific)).or_default() += 1;
    }

    let summary = Summary {
        file,
        dxf: DxfSummary {
            size: dxf.len(),
            entities,
            layers: drawing.layers().count(),
            line_types: drawing.line_types().count(),
            blocks: drawing.blocks().count(),
            skipped,
        },
    };

    Ok(Conversion { summary, dxf })
}

fn entity_name(entity: &dxf::entities::EntityType) -> &'static str {
    use dxf::entities::EntityType;

    match entity {
        EntityType::Arc(_) => "ARC",
        EntityType::Circle(_) => "CIRCLE",
        EntityType::Face3D(_) => "3DFACE",
        EntityType::Insert(_) => "INSERT",
        EntityType::Line(_) => "LINE",
        EntityType::ModelPoint(_) => "POINT",
        EntityType::Polyline(_) => "POLYLINE",
        EntityType::Text(_) => "TEXT",
        _ => "OTHER",
    }
}
