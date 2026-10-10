use crate::coordinate::Coordinate;
use crate::geometry::{Geometry, POLYLINE_CLOSED};
use crate::object::Object;

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
            tilt,
            ..
        } => {
            let text = dxf::entities::Text {
                thickness: 1.0,
                location: position.into(),
                text_height: 1.0,
                value: string.clone(),
                rotation: 0.0,
                relative_x_scale_factor: 1.0,
                oblique_angle: tilt.into(),
                text_style_name: "".to_string(),
                text_generation_flags: 0,
                second_alignment_point: Default::default(),
                normal: Default::default(),
                horizontal_text_justification: dxf::enums::HorizontalTextJustification::Center,
                vertical_text_justification: dxf::enums::VerticalTextJustification::Middle,
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
        Geometry::Symbol { .. } | Geometry::Image { .. } | Geometry::Other { .. } => {
            return None;
        }
    };

    Some(entity)
}

pub fn objects_to_dxf(objects: Vec<Object>) -> dxf::Drawing {
    let mut drawing = dxf::Drawing::new();
    // Geometries between a block's begin and end belong to that block, not to the model space.
    let mut block: Option<dxf::Block> = None;

    for object in objects {
        match object {
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
                let Some(entity_type) = entity_type(&mut drawing, geometry) else {
                    continue;
                };

                let entity = dxf::entities::Entity::new(entity_type);

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
