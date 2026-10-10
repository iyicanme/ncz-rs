use crate::coordinate::Coordinate;
use crate::geometry::{Geometry, POLYLINE_CLOSED};
use crate::line_type::LineType;
use crate::object::Object;

/// Segment values at or above this refer to symbols rather than lengths.
const SYMBOL_SEGMENT: f32 = 1000.0;

impl From<Coordinate> for dxf::Point {
    fn from(value: Coordinate) -> Self {
        dxf::Point::new(value.x, value.y, value.z)
    }
}

fn polyline(drawing: &mut dxf::Drawing, points: Vec<Coordinate>, closed: bool) -> dxf::entities::EntityType {
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
fn rectangle(drawing: &mut dxf::Drawing, corner1: Coordinate, corner2: Coordinate) -> dxf::entities::EntityType {
    let z = corner1.z;
    let points = [(corner1.x, corner1.y), (corner2.x, corner1.y), (corner2.x, corner2.y), (corner1.x, corner2.y)]
        .into_iter()
        .map(|(x, y)| Coordinate { x, y, z })
        .collect();

    polyline(drawing, points, true)
}

/// Maps an alignment code to a DXF justification. Digits are assumed to follow the numeric keypad layout,
/// e.g. 7 is top left and 3 is bottom right. The letters are guesses.
fn text_justification(snap: char) -> (dxf::enums::HorizontalTextJustification, dxf::enums::VerticalTextJustification) {
    use dxf::enums::HorizontalTextJustification as H;
    use dxf::enums::VerticalTextJustification as V;

    match snap {
        '1' => (H::Left, V::Bottom),
        '2' => (H::Center, V::Bottom),
        '3' => (H::Right, V::Bottom),
        '4' => (H::Left, V::Middle),
        '5' => (H::Center, V::Middle),
        '6' => (H::Right, V::Middle),
        '7' => (H::Left, V::Top),
        '8' => (H::Center, V::Top),
        '9' => (H::Right, V::Top),
        'L' => (H::Left, V::Baseline),
        'C' => (H::Center, V::Baseline),
        'R' => (H::Right, V::Baseline),
        _ => (H::Center, V::Middle),
    }
}

/// Converts a line type to a DXF line type. Symbols are dropped, since they take no length in the pattern.
/// Returns `None` for slots that hold no usable line type.
fn dxf_line_type(line_type: &LineType) -> Option<dxf::tables::LineType> {
    let is_valid_name = !line_type.name.is_empty()
        && line_type
            .name
            .chars()
            .all(|c| !c.is_control() && !"<>/\\\":;?*|=,`".contains(c));
    if !is_valid_name {
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

/// Whether the geometry has a DXF counterpart, the others are left out of the drawing.
pub fn is_exported(geometry: &Geometry) -> bool {
    !matches!(
        geometry,
        Geometry::Symbol { .. } | Geometry::Image { .. } | Geometry::Empty { .. } | Geometry::Other { .. }
    )
}

fn entity_type(drawing: &mut dxf::Drawing, geometry: Geometry) -> Option<dxf::entities::EntityType> {
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
            ..
        } => {
            let (horizontal_text_justification, vertical_text_justification) = text_justification(snap);
            let text = dxf::entities::Text {
                location: position.into(),
                text_height: size.into(),
                value: string,
                rotation: f64::from(tilt).to_degrees(),
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
        Geometry::Contour { points, .. } => polyline(drawing, points, false),
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
        Geometry::Window { corner1, corner2, .. } => rectangle(drawing, corner1, corner2),
        Geometry::MapSheet { header, .. } => rectangle(drawing, header.lower_bound, header.upper_bound),
        Geometry::Triangle { vertices: [first, second, third], .. } => {
            let face = dxf::entities::Face3D::new(first.into(), second.into(), third.into(), third.into());
            dxf::entities::EntityType::Face3D(face)
        }
        Geometry::Symbol { .. } | Geometry::Image { .. } | Geometry::Empty { .. } | Geometry::Other { .. } => {
            return None;
        }
    };

    Some(entity)
}

pub fn objects_to_dxf(objects: Vec<Object>) -> dxf::Drawing {
    let mut drawing = dxf::Drawing::new();
    // Layer names by index, geometries refer to layers by their index in the layer table.
    let mut layer_names: Vec<String> = vec![];
    // Line type names by index, `None` where the slot could not be converted.
    let mut line_type_names: Vec<Option<String>> = vec![];
    // Geometries between a block's begin and end belong to that block, not to the model space.
    let mut block: Option<dxf::Block> = None;

    for object in objects {
        match object {
            Object::Layers { layers, .. } => {
                for layer in layers {
                    let exists = drawing.layers().any(|l| l.name == layer.name);
                    if !layer.name.is_empty() && !exists {
                        drawing.add_layer(dxf::tables::Layer {
                            name: layer.name.clone(),
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

                let Some(entity_type) = entity_type(&mut drawing, geometry) else {
                    continue;
                };

                let mut entity = dxf::entities::Entity::new(entity_type);
                if let Some(layer) = layer {
                    entity.common.layer = layer;
                }
                if let Some(line_type) = line_type {
                    entity.common.line_type_name = line_type;
                }

                match &mut block {
                    Some(block) => block.entities.push(entity),
                    None => {
                        drawing.add_entity(entity);
                    }
                }
            }
            _ => {}
        }
    }

    drawing
}
