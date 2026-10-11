//! Flattens NCZ geometries and DXF entities into `key -> value` maps that can be compared.

use crate::dxf::{Entity, Group};
use nczrs::geometry::Geometry;
use nczrs::object::{Layer, Object};
use serde_json::Value;
use std::collections::BTreeMap;

pub type Features = BTreeMap<String, String>;

/// Bytes of an area record or geometry style body that are expanded into one feature each.
const MAX_CONTEXT_BYTES: usize = 64;
/// Byte arrays longer than this are summarised by their length.
const MAX_ARRAY: usize = 64;

/// Formats a number so that the same value read as `f32` or `f64` (or printed by a DXF writer) compares equal.
pub fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }

    let rounded: f64 = format!("{value:.6e}").parse().unwrap_or(value);
    format!("{rounded}")
}

/// A geometry together with what is around it in the stream.
pub struct NczItem {
    /// Position among the geometries of the file.
    pub index: usize,
    pub kind: &'static str,
    /// Name of the block that holds the geometry, `None` in the model space.
    pub block: Option<String>,
    pub points: Vec<(f64, f64)>,
    pub text: Option<String>,
    pub features: Features,
}

pub fn ncz_items(objects: &[Object]) -> Vec<NczItem> {
    let mut items = vec![];
    let mut layers: &[Layer] = &[];
    let mut line_type_names: Vec<String> = vec![];
    let mut block = None;
    let mut style: Option<(u8, &[u8])> = None;
    let mut area: Option<&[u8]> = None;

    for object in objects {
        match object {
            Object::Layers { layers: l, .. } => layers = l,
            Object::LineTypes { types } => {
                line_type_names = types.iter().map(|t| t.name.clone()).collect()
            }
            Object::BlockBegin { name } => block = Some(name.clone()),
            Object::BlockEnd => block = None,
            Object::GeometryStyle {
                geometry_type,
                bytes,
            } => style = Some((*geometry_type, bytes)),
            Object::AreaBegin { bytes } => area = Some(bytes),
            Object::AreaEnd { .. } => area = None,
            Object::Geometry { geometry } => {
                let mut features = geometry_features(geometry);
                let header = geometry.header();
                if let Some(layer) = layers.get(usize::from(header.layer_id)) {
                    features.insert("ctx.layer_name".to_owned(), layer.name.clone());
                }
                if let Some(name) = line_type_names.get(usize::from(header.line_type)) {
                    features.insert("ctx.line_type_name".to_owned(), name.clone());
                }
                if let Some((geometry_type, bytes)) = style.take() {
                    features.insert("ctx.style.type".to_owned(), geometry_type.to_string());
                    insert_bytes(&mut features, "ctx.style", bytes);
                }
                if let Some(bytes) = area {
                    insert_bytes(&mut features, "ctx.area", bytes);
                }

                items.push(NczItem {
                    index: items.len(),
                    kind: geometry.kind(),
                    block: block.clone(),
                    points: geometry_points(geometry),
                    text: match geometry {
                        Geometry::Text { string, .. } => Some(string.clone()),
                        _ => None,
                    },
                    features,
                });
            }
            _ => {}
        }
    }

    items
}

fn insert_bytes(features: &mut Features, prefix: &str, bytes: &[u8]) {
    features.insert(format!("{prefix}.len"), bytes.len().to_string());
    for (i, byte) in bytes.iter().take(MAX_CONTEXT_BYTES).enumerate() {
        features.insert(format!("{prefix}[{i}]"), byte.to_string());
    }
}

/// The points used to find the DXF counterpart, anchor first.
fn geometry_points(geometry: &Geometry) -> Vec<(f64, f64)> {
    let xy = |c: &nczrs::coordinate::Coordinate| (c.x, c.y);
    match geometry {
        Geometry::Point { position, .. }
        | Geometry::Text { position, .. }
        | Geometry::Symbol { position, .. }
        | Geometry::BlockInsert { position, .. }
        | Geometry::Image { position, .. } => vec![xy(position)],
        Geometry::Line { begin, end, .. } => vec![xy(begin), xy(end)],
        Geometry::Circle { center, .. } | Geometry::Arc { center, .. } => vec![xy(center)],
        Geometry::PolyLine { points, .. } | Geometry::Contour { points, .. } => {
            points.iter().map(xy).collect()
        }
        Geometry::Window {
            corner1, corner2, ..
        } => vec![xy(corner1), xy(corner2)],
        Geometry::Triangle { vertices, .. } => vertices.iter().map(xy).collect(),
        Geometry::MapSheet { header, .. } => vec![xy(&header.lower_bound), xy(&header.upper_bound)],
        Geometry::Empty { .. } | Geometry::Other { .. } => vec![],
    }
}

fn geometry_features(geometry: &Geometry) -> Features {
    let mut features = Features::new();
    let value = serde_json::to_value(geometry).unwrap_or(Value::Null);
    // Enums serialize as `{"Variant": {fields}}`.
    if let Some(fields) = value
        .as_object()
        .and_then(|o| o.values().next())
        .and_then(Value::as_object)
    {
        for (key, value) in fields {
            flatten(&mut features, key, value);
        }
    }

    let header = geometry.header();
    let attributes = &header.attributes;
    if let Ok(bytes) = attributes[4..12].try_into() {
        features.insert(
            "attributes[4..12].f64".to_owned(),
            format_number(f64::from_le_bytes(bytes)),
        );
    }
    if let Ok(bytes) = attributes[12..16].try_into() {
        features.insert(
            "attributes[12..16].f32".to_owned(),
            format_number(f32::from_le_bytes(bytes).into()),
        );
    }
    insert_bits(&mut features, "line_type2", header.line_type2.into(), 8);
    if let Geometry::PolyLine { flags, .. } = geometry {
        insert_bits(&mut features, "flags", *flags, 32);
    }
    if let Geometry::Contour { point_flags, .. } = geometry {
        let ones = point_flags.iter().filter(|&&f| f != 0).count();
        features.insert("point_flags.ones".to_owned(), ones.to_string());
    }
    if let Geometry::PolyLine { points, .. } | Geometry::Contour { points, .. } = geometry {
        features.insert("points.len".to_owned(), points.len().to_string());
    }

    features
}

fn insert_bits(features: &mut Features, key: &str, value: u32, bits: u32) {
    for bit in 0..bits {
        features.insert(format!("{key}.bit{bit}"), ((value >> bit) & 1).to_string());
    }
}

/// Adds scalars under their key, byte arrays per index and the elevation of coordinates.
fn flatten(features: &mut Features, key: &str, value: &Value) {
    match value {
        Value::Bool(b) => {
            features.insert(key.to_owned(), u8::from(*b).to_string());
        }
        Value::Number(n) => {
            features.insert(
                key.to_owned(),
                format_number(n.as_f64().unwrap_or(f64::NAN)),
            );
        }
        Value::String(s) => {
            features.insert(key.to_owned(), s.clone());
        }
        Value::Object(o) if o.contains_key("x") && o.contains_key("z") => {
            flatten(features, &format!("{key}.z"), &o["z"]);
        }
        Value::Array(a) if a.iter().all(Value::is_number) => {
            if a.len() <= MAX_ARRAY {
                for (i, v) in a.iter().enumerate() {
                    flatten(features, &format!("{key}[{i}]"), v);
                }
            } else {
                features.insert(format!("{key}.len"), a.len().to_string());
            }
        }
        _ => {}
    }
}

/// Group codes that identify an entity rather than describe it, or repeat its coordinates.
fn is_ignored_code(code: i32) -> bool {
    matches!(code, 0 | 5 | 100 | 102 | 330 | 360)
        || (10..=39).contains(&code) && code != 38 && code != 39
}

fn normalize(value: &str) -> String {
    match value.trim().parse::<f64>() {
        Ok(number) => format_number(number),
        Err(_) => decode_unicode_escapes(value),
    }
}

/// Replaces the `\U+XXXX` escapes that DXF uses for characters outside the code page.
pub fn decode_unicode_escapes(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("\\U+") {
        out.push_str(&rest[..start]);
        let hex = rest.get(start + 3..start + 7);
        match hex
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .and_then(char::from_u32)
        {
            Some(c) => {
                out.push(c);
                rest = &rest[start + 7..];
            }
            None => {
                out.push_str("\\U+");
                rest = &rest[start + 3..];
            }
        }
    }
    out.push_str(rest);

    out
}

/// Adds the groups of an entity or table entry under `prefix`. Repeated codes keep their first value and
/// also get a count and, when few, the set of distinct values.
pub fn insert_groups(features: &mut Features, prefix: &str, groups: &[Group]) {
    let mut by_code: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut xdata_app: Option<String> = None;
    for (code, value) in groups {
        if *code == 1001 {
            xdata_app = Some(value.clone());
            continue;
        }
        if xdata_app.is_none() && is_ignored_code(*code) {
            continue;
        }
        let key = match &xdata_app {
            Some(app) => format!("{prefix}x:{app}:{code}"),
            None => format!("{prefix}{code}"),
        };
        by_code.entry(key).or_default().push(normalize(value));
    }

    for (key, values) in by_code {
        if values.len() > 1 {
            let mut distinct = values.clone();
            distinct.sort();
            distinct.dedup();
            features.insert(format!("{key}#count"), values.len().to_string());
            if distinct.len() <= 4 {
                features.insert(format!("{key}#set"), distinct.join(","));
            }
        }
        features.insert(key, values[0].clone());
    }
}

pub fn entity_features(entity: &Entity) -> Features {
    let mut features = Features::new();
    insert_groups(&mut features, "", &entity.groups);
    if let Some(first) = entity.children.first() {
        let prefix = format!("{}.", first.kind);
        let groups: Vec<Group> = entity
            .children
            .iter()
            .flat_map(|c| c.groups.clone())
            .collect();
        insert_groups(&mut features, &prefix, &groups);
        features.insert(
            format!("{prefix}#children"),
            entity.children.len().to_string(),
        );
    }
    if let Some(Ok(code)) = entity.get(70).map(|v| v.trim().parse::<u32>()) {
        insert_bits(&mut features, "70", code, 16);
    }

    features
}
