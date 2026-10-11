use crate::coordinate::Coordinate;
use crate::geometry::{Geometry, Header, POLYLINE_CLOSED};
use std::collections::BTreeMap;
use crate::line_type::LineType;
use crate::object::{LAYER_VISIBLE, Object};

/// Segment values at or above this refer to symbols rather than lengths.
const SYMBOL_SEGMENT: f32 = 1000.0;

/// Application name of the extended data that holds point names and feature codes.
const APP_ID: &str = "NCZ";

/// Prefix of the blocks that stand in for symbols. The shapes are in NetCAD's symbol libraries (`.NCS`), which are
/// not part of the NCZ file, so each block holds only the symbol's footprint and can be redefined with the real shape.
const SYMBOL_BLOCK_PREFIX: &str = "NCZ_SYMBOL_";

/// Slant of italic text in degrees. DXF has no italic flag for text, so it is drawn obliqued.
const ITALIC_OBLIQUE_ANGLE: f64 = 15.0;

/// NCZ stores northing as x and easting as y, DXF puts easting on X. Angles are already in the east-north frame.
impl From<Coordinate> for dxf::Point {
    fn from(value: Coordinate) -> Self {
        dxf::Point::new(value.y, value.x, value.z)
    }
}

/// Splits a contour into the runs that are drawn, matching NetCAD's DXF export. Flagged points mark where the
/// elevation label goes. A segment between two flagged points is hidden, and flagged points at the start of the
/// contour are not drawn at all.
fn contour_runs(points: Vec<Coordinate>, point_flags: &[u8]) -> Vec<Vec<Coordinate>> {
    let leading = point_flags.iter().take_while(|&&flag| flag != 0).count();
    let mut runs = vec![];
    let mut run = vec![];
    let mut previous_flagged = false;

    for (point, &flag) in points.into_iter().zip(point_flags).skip(leading) {
        let flagged = flag != 0;
        if flagged && previous_flagged {
            runs.push(std::mem::take(&mut run));
        }
        run.push(point);
        previous_flagged = flagged;
    }
    runs.push(run);

    runs.retain(|run| run.len() >= 2);
    runs
}

fn polyline(
    drawing: &mut dxf::Drawing,
    points: Vec<Coordinate>,
    closed: bool,
) -> dxf::entities::EntityType {
    let mut polyline = dxf::entities::Polyline::default();
    polyline.set_is_3d_polyline(true);
    polyline.set_is_closed(closed);

    for point in points {
        let mut vertex = dxf::entities::Vertex::new(point.into());
        vertex.set_is_3d_polyline_vertex(true);
        polyline.add_vertex(drawing, vertex);
    }

    dxf::entities::EntityType::Polyline(polyline)
}

/// An axis aligned rectangle with the given opposite corners, at the elevation of the first.
fn rectangle(
    drawing: &mut dxf::Drawing,
    corner1: Coordinate,
    corner2: Coordinate,
) -> dxf::entities::EntityType {
    rotated_rectangle(drawing, corner1, corner2, 0.0)
}

/// A rectangle given by two opposite corners before rotation, turned by `rotation` radians counter-clockwise
/// (in the east-north frame) around `corner1`.
fn rotated_rectangle(
    drawing: &mut dxf::Drawing,
    corner1: Coordinate,
    corner2: Coordinate,
    rotation: f64,
) -> dxf::entities::EntityType {
    let z = corner1.z;
    let (sin, cos) = rotation.sin_cos();
    // NCZ stores northing as x and easting as y, so a counter-clockwise turn is clockwise in x/y.
    let (width, height) = (corner2.x - corner1.x, corner2.y - corner1.y);
    let points = [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        .into_iter()
        .map(|(dx, dy)| Coordinate {
            x: corner1.x + dx * cos + dy * sin,
            y: corner1.y - dx * sin + dy * cos,
            z,
        })
        .collect();

    polyline(drawing, points, true)
}

fn symbol_block_name(symbol: u16) -> String {
    format!("{SYMBOL_BLOCK_PREFIX}{symbol}")
}

/// The extent of a symbol in its own frame, before scaling and rotation, relative to its insertion point. In DXF
/// axes, so X is easting.
#[derive(Clone, Copy)]
struct SymbolFootprint {
    min: (f64, f64),
    max: (f64, f64),
    /// How far the instance it was measured on is turned from a right angle. Only an instance turned by a multiple
    /// of a right angle gives the exact extent, others give a larger one.
    deviation: f64,
}

/// Measures the footprint of a symbol from the bounds of one of its instances, which NetCAD stores in world
/// coordinates. Symbol positions, scales and rotations match a DXF insert of the symbol drawn in its own frame.
fn symbol_footprint(
    header: &Header,
    position: Coordinate,
    scale: f64,
    rotation: f64,
) -> Option<SymbolFootprint> {
    let (lower, upper) = (header.lower_bound, header.upper_bound);
    // Instances without bounds store 1e30 as the lower and -1e30 as the upper bound.
    if lower.x > upper.x || lower.y > upper.y || scale <= 0.0 {
        return None;
    }

    let (sin, cos) = rotation.sin_cos();
    let corners = [(lower.y, lower.x), (upper.y, lower.x), (upper.y, upper.x), (lower.y, upper.x)];
    let local = corners.map(|(x, y)| {
        let (dx, dy) = (x - position.y, y - position.x);
        ((dx * cos + dy * sin) / scale, (-dx * sin + dy * cos) / scale)
    });
    let quarter = std::f64::consts::FRAC_PI_2;

    Some(SymbolFootprint {
        min: local
            .iter()
            .fold((f64::INFINITY, f64::INFINITY), |(x, y), p| (x.min(p.0), y.min(p.1))),
        max: local.iter().fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |(x, y), p| {
            (x.max(p.0), y.max(p.1))
        }),
        deviation: (rotation - (rotation / quarter).round() * quarter).abs(),
    })
}

/// Gives an entity its handles, including those of a polyline's vertices and end marker. The DXF crate assigns them
/// only to entities added to the model space, and programs such as ezdxf cannot read a block whose polyline ends in
/// a marker without a handle.
fn with_handles(drawing: &mut dxf::Drawing, entity: dxf::entities::Entity) -> dxf::entities::Entity {
    drawing.add_entity(entity);
    let last = drawing.entities().count() - 1;
    drawing
        .remove_entity(last)
        .expect("the entity was just added")
}

/// A block that stands in for a symbol: its footprint, or a unit square around the insertion point when no
/// instance has usable bounds. It is drawn by block, so it takes the layer, color and line type of each insert.
fn symbol_block(
    drawing: &mut dxf::Drawing,
    symbol: u16,
    footprint: Option<SymbolFootprint>,
) -> dxf::Block {
    let (min, max) = footprint
        .map(|footprint| (footprint.min, footprint.max))
        .filter(|(min, max)| max.0 - min.0 > f64::EPSILON || max.1 - min.1 > f64::EPSILON)
        .unwrap_or(((-0.5, -0.5), (0.5, 0.5)));
    // `rectangle` takes NCZ coordinates, which have the axes the other way round.
    let corner = |(x, y)| Coordinate { x: y, y: x, z: 0.0 };

    let mut entity = dxf::entities::Entity::new(rectangle(drawing, corner(min), corner(max)));
    entity.common.color = dxf::Color::by_block();
    entity.common.line_type_name = "BYBLOCK".to_owned();

    dxf::Block {
        name: symbol_block_name(symbol),
        entities: vec![with_handles(drawing, entity)],
        ..Default::default()
    }
}

/// Maps an alignment code to a DXF justification, as NetCAD 5 exports it (4,082 texts). The digits do not follow the
/// numeric keypad layout. Codes that NetCAD's export never showed (`P`, `U`, `Q`, …) fall back to centre middle.
fn text_justification(
    snap: char,
) -> (
    dxf::enums::HorizontalTextJustification,
    dxf::enums::VerticalTextJustification,
) {
    use dxf::enums::HorizontalTextJustification as H;
    use dxf::enums::VerticalTextJustification as V;

    match snap {
        'L' | '6' => (H::Left, V::Baseline),
        '7' => (H::Center, V::Baseline),
        'C' => (H::Center, V::Bottom),
        'R' => (H::Right, V::Bottom),
        '1' => (H::Left, V::Middle),
        'M' => (H::Center, V::Middle),
        '5' => (H::Right, V::Middle),
        '2' => (H::Left, V::Top),
        '3' => (H::Center, V::Top),
        '4' => (H::Right, V::Top),
        _ => (H::Center, V::Middle),
    }
}

/// Whether the name can be used for a DXF table entry, such as a line type or a text style.
fn is_valid_table_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| !c.is_control() && !"<>/\\\":;?*|=,`".contains(c))
}

/// Converts a line type to a DXF line type. Symbols are dropped, since they take no length in the pattern.
/// Returns `None` for slots that hold no usable line type.
fn dxf_line_type(line_type: &LineType) -> Option<dxf::tables::LineType> {
    if !is_valid_table_name(&line_type.name) {
        return None;
    }

    let segments: Vec<f64> = line_type
        .segments
        .iter()
        .filter(|segment| segment.abs() < SYMBOL_SEGMENT)
        .map(|&segment| f64::from(segment))
        .collect();
    let total_pattern_length: f64 = segments.iter().map(|segment| segment.abs()).sum();
    if segments.is_empty() || !total_pattern_length.is_finite() || total_pattern_length <= 0.0 {
        return None;
    }

    Some(dxf::tables::LineType {
        name: line_type.name.clone(),
        description: line_type.description.clone(),
        element_count: segments.len() as i32,
        total_pattern_length,
        dash_dot_space_lengths: segments,
        ..Default::default()
    })
}

/// Converts a font file name to a text style. The style is named after the file stem, e.g. `ARIAL` for
/// `ARIAL.NCF`, and is assumed to use the TrueType font of the same name.
fn text_style(font: &str) -> Option<dxf::tables::Style> {
    let name = font.rsplit_once('.').map_or(font, |(stem, _)| stem).trim();
    if !is_valid_table_name(name) {
        return None;
    }

    Some(dxf::tables::Style {
        name: name.to_owned(),
        primary_font_file_name: format!("{}.ttf", name.to_lowercase()),
        ..Default::default()
    })
}

/// Point names, feature codes and links as extended data, `None` when the geometry has neither.
fn x_data(geometry: &Geometry) -> Option<dxf::XData> {
    let name = match geometry {
        Geometry::Point { name, .. } => Some(name),
        _ => None,
    };
    let code = geometry.header().code.as_ref();
    let link = geometry.header().link.as_ref();

    let items: Vec<dxf::XDataItem> = [("NAME", name), ("CODE", code), ("LINK", link)]
        .into_iter()
        .filter_map(|(key, value)| {
            value
                .filter(|value| !value.is_empty())
                .map(|value| (key, value))
        })
        .flat_map(|(key, value)| {
            [
                dxf::XDataItem::Str(key.to_owned()),
                dxf::XDataItem::Str(value.clone()),
            ]
        })
        .collect();

    (!items.is_empty()).then(|| dxf::XData {
        application_name: APP_ID.to_owned(),
        items,
    })
}

/// Layer colors are assumed to share the AutoCAD color index palette. Zero is not a valid layer color in DXF,
/// so it falls back to white.
fn layer_color(color: u8) -> dxf::Color {
    match color {
        0 => dxf::Color::from_index(7),
        color => dxf::Color::from_index(color),
    }
}

/// Whether the geometry has a DXF counterpart, the others are left out of the drawing.
pub fn is_exported(geometry: &Geometry) -> bool {
    match geometry {
        // Symbols with no scale are not drawn, NetCAD stores empty bounds for them.
        Geometry::Symbol { scale, .. } => *scale > 0.0,
        Geometry::Image { .. } | Geometry::Empty { .. } | Geometry::Other { .. } => false,
        _ => true,
    }
}

/// The entities that draw the geometry, usually one. Contours with gaps are drawn as several polylines.
fn entity_types(drawing: &mut dxf::Drawing, geometry: Geometry) -> Vec<dxf::entities::EntityType> {
    let entity = match geometry {
        Geometry::Point { position, .. } => {
            let point = dxf::entities::ModelPoint::new(position.into());
            dxf::entities::EntityType::ModelPoint(point)
        }
        Geometry::Line { begin, end, .. } => {
            let line = dxf::entities::Line::new(begin.into(), end.into());
            dxf::entities::EntityType::Line(line)
        }
        Geometry::Circle { center, radius, .. } => {
            let circle = dxf::entities::Circle::new(center.into(), radius);
            dxf::entities::EntityType::Circle(circle)
        }
        Geometry::Arc {
            center,
            radius,
            start_angle,
            end_angle,
            ..
        } => {
            let arc = dxf::entities::Arc::new(
                center.into(),
                radius,
                start_angle.to_degrees(),
                end_angle.to_degrees(),
            );
            dxf::entities::EntityType::Arc(arc)
        }
        Geometry::Text {
            position,
            string,
            size,
            tilt,
            snap,
            italic,
            underline,
            ..
        } => {
            let (horizontal_text_justification, vertical_text_justification) =
                text_justification(snap);
            let text = dxf::entities::Text {
                location: position.into(),
                text_height: size.into(),
                value: if underline {
                    format!("%%u{string}")
                } else {
                    string
                },
                rotation: f64::from(tilt).to_degrees(),
                oblique_angle: if italic { ITALIC_OBLIQUE_ANGLE } else { 0.0 },
                relative_x_scale_factor: 1.0,
                // Every justification but the default one positions the text at this point.
                second_alignment_point: position.into(),
                horizontal_text_justification,
                vertical_text_justification,
                ..Default::default()
            };
            dxf::entities::EntityType::Text(text)
        }
        Geometry::PolyLine { points, flags, .. } => {
            polyline(drawing, points, flags & POLYLINE_CLOSED != 0)
        }
        Geometry::Contour {
            points,
            point_flags,
            ..
        } => {
            return contour_runs(points, &point_flags)
                .into_iter()
                .map(|run| polyline(drawing, run, false))
                .collect();
        }
        Geometry::BlockInsert {
            position,
            name,
            rotation,
            scale_x,
            scale_y,
            ..
        } => {
            let insert = dxf::entities::Insert {
                name,
                location: position.into(),
                x_scale_factor: scale_x,
                y_scale_factor: scale_y,
                rotation: rotation.to_degrees(),
                ..Default::default()
            };
            dxf::entities::EntityType::Insert(insert)
        }
        Geometry::Window {
            corner1,
            corner2,
            rotation,
            ..
        } => rotated_rectangle(drawing, corner1, corner2, rotation.into()),
        Geometry::MapSheet { header, .. } => {
            rectangle(drawing, header.lower_bound, header.upper_bound)
        }
        Geometry::Triangle {
            vertices: [first, second, third],
            ..
        } => {
            let face =
                dxf::entities::Face3D::new(first.into(), second.into(), third.into(), third.into());
            dxf::entities::EntityType::Face3D(face)
        }
        Geometry::Symbol {
            position,
            scale,
            rotation,
            symbol,
            ..
        } if scale > 0.0 => {
            let insert = dxf::entities::Insert {
                name: symbol_block_name(symbol),
                location: position.into(),
                x_scale_factor: scale.into(),
                y_scale_factor: scale.into(),
                rotation: f64::from(rotation).to_degrees(),
                ..Default::default()
            };
            dxf::entities::EntityType::Insert(insert)
        }
        Geometry::Symbol { .. }
        | Geometry::Image { .. }
        | Geometry::Empty { .. }
        | Geometry::Other { .. } => {
            return vec![];
        }
    };

    vec![entity]
}

pub fn objects_to_dxf(objects: Vec<Object>) -> dxf::Drawing {
    let mut drawing = dxf::Drawing::new();
    // R12 has neither extended data nor a thumbnail.
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.add_app_id(dxf::tables::AppId {
        name: APP_ID.to_owned(),
        ..Default::default()
    });
    // Layer names by index, geometries refer to layers by their index in the layer table.
    let mut layer_names: Vec<String> = vec![];
    // Line type names by index, `None` where the slot could not be converted.
    let mut line_type_names: Vec<Option<String>> = vec![];
    // Text style names by font index, `None` where the font could not be converted.
    let mut text_style_names: Vec<Option<String>> = vec![];
    // Geometries between a block's begin and end belong to that block, not to the model space.
    let mut block: Option<dxf::Block> = None;
    // Footprints of the symbols in use, measured on the instance closest to a right angle.
    let mut symbol_footprints: BTreeMap<u16, Option<SymbolFootprint>> = BTreeMap::new();

    for object in objects {
        match object {
            Object::Layers { layers, .. } => {
                for layer in layers {
                    let exists = drawing.layers().any(|l| l.name == layer.name);
                    if !layer.name.is_empty() && !exists {
                        drawing.add_layer(dxf::tables::Layer {
                            name: layer.name.clone(),
                            color: layer_color(layer.color),
                            is_layer_on: layer.flags & LAYER_VISIBLE != 0,
                            ..Default::default()
                        });
                    }
                    layer_names.push(layer.name);
                }
            }
            Object::LineTypes { types } => {
                for line_type in &types {
                    let name = dxf_line_type(line_type).map(|dxf_line_type| {
                        // DXF names are case insensitive, a repeated name reuses the first line type.
                        let existing = drawing
                            .line_types()
                            .find(|l| l.name.eq_ignore_ascii_case(&dxf_line_type.name))
                            .map(|l| l.name.clone());
                        existing.unwrap_or_else(|| {
                            let name = dxf_line_type.name.clone();
                            drawing.add_line_type(dxf_line_type);
                            name
                        })
                    });
                    line_type_names.push(name);
                }
            }
            Object::Fonts { names } => {
                for font in &names {
                    let name = text_style(font).map(|style| {
                        // Fonts that differ only in extension, e.g. `ARIAL.NCF` and `ARIAL.NCI`, share a style.
                        let existing = drawing
                            .styles()
                            .find(|s| s.name.eq_ignore_ascii_case(&style.name))
                            .map(|s| s.name.clone());
                        existing.unwrap_or_else(|| {
                            let name = style.name.clone();
                            drawing.add_style(style);
                            name
                        })
                    });
                    text_style_names.push(name);
                }
            }
            Object::Extents {
                lower_bound,
                upper_bound,
            } => {
                drawing.header.minimum_drawing_extents = lower_bound.into();
                drawing.header.maximum_drawing_extents = upper_bound.into();
            }
            Object::Thumbnail { bytes } => {
                drawing.thumbnail =
                    image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp).ok();
            }
            // Line type patterns are assumed to be in paper millimetres, which the map scale turns into metres.
            Object::Scale { scale } => drawing.header.line_type_scale = scale,
            Object::BlockBegin { name } => {
                block = Some(dxf::Block {
                    name,
                    ..Default::default()
                });
            }
            Object::BlockEnd => {
                if let Some(block) = block.take() {
                    drawing.add_block(block);
                }
            }
            Object::Geometry { geometry } => {
                let layer = layer_names
                    .get(usize::from(geometry.header().layer_id))
                    .filter(|name| !name.is_empty())
                    .cloned();
                // Text has a line type index too, but DXF text is not drawn with one.
                let line_type = match geometry {
                    Geometry::Text { .. } => None,
                    _ => line_type_names
                        .get(usize::from(geometry.header().line_type))
                        .cloned()
                        .flatten(),
                };

                let text_style = match geometry {
                    Geometry::Text { font, .. } => {
                        text_style_names.get(usize::from(font)).cloned().flatten()
                    }
                    _ => None,
                };
                let x_data = x_data(&geometry);

                let color = geometry.header().color;

                if let Geometry::Symbol {
                    position,
                    scale,
                    rotation,
                    symbol,
                    ..
                } = &geometry
                    && is_exported(&geometry)
                {
                    let measured = symbol_footprint(
                        geometry.header(),
                        *position,
                        f64::from(*scale),
                        f64::from(*rotation),
                    );
                    let footprint = symbol_footprints.entry(*symbol).or_default();
                    if let Some(measured) = measured
                        && footprint.is_none_or(|f| measured.deviation < f.deviation)
                    {
                        *footprint = Some(measured);
                    }
                }

                for entity_type in entity_types(&mut drawing, geometry) {
                    let mut entity = dxf::entities::Entity::new(entity_type);
                    if let (dxf::entities::EntityType::Text(text), Some(text_style)) =
                        (&mut entity.specific, &text_style)
                    {
                        text.text_style_name = text_style.clone();
                    }
                    entity.common.x_data.extend(x_data.clone());
                    if let Some(layer) = &layer {
                        entity.common.layer = layer.clone();
                    }
                    if let Some(line_type) = &line_type {
                        entity.common.line_type_name = line_type.clone();
                    }
                    // Zero means the geometry takes the color of its layer, which is the DXF default.
                    if color != 0 {
                        entity.common.color = dxf::Color::from_index(color);
                    }

                    match &mut block {
                        Some(block) => block.entities.push(with_handles(&mut drawing, entity)),
                        None => {
                            drawing.add_entity(entity);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for (symbol, footprint) in symbol_footprints {
        let block = symbol_block(&mut drawing, symbol, footprint);
        drawing.add_block(block);
    }

    drawing
}
