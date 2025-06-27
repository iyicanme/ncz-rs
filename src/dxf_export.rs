use crate::coordinate::Coordinate;
use crate::geometry::Geometry;
use crate::object::Object;

impl From<Coordinate> for dxf::Point {
    fn from(value: Coordinate) -> Self {
        dxf::Point::new(value.x, value.y, value.z)
    }
}
impl TryFrom<Geometry> for dxf::entities::EntityType {
    type Error = ();

    fn try_from(geometry: Geometry) -> Result<Self, Self::Error> {
        let entity = match geometry {
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
                    start_angle / 0.9,
                    end_angle / 0.9,
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
            Geometry::Curve { .. } => {
                return Err(());
            }
            Geometry::PolyLine { .. } => {
                return Err(());
            }
            Geometry::SmoothCurve { .. } => {
                return Err(());
            }
            Geometry::Point { .. } => {
                return Err(());
            }
            Geometry::Other { .. } => {
                return Err(());
            }
        };

        Ok(entity)
    }
}

pub fn objects_to_dxf(objects: Vec<Object>) -> dxf::Drawing {
    let mut drawing = dxf::Drawing::new();
    for object in objects {
        let Object::Geometry { geometry } = object else {
            continue;
        };

        let Ok(entity_type) = dxf::entities::EntityType::try_from(geometry) else {
            continue;
        };
        
        let entity = dxf::entities::Entity::new(entity_type);

        drawing.add_entity(entity);
    }

    drawing
}
