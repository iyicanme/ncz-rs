use crate::coordinate::Coordinate;
use crate::geometry::{Geometry, Header, POLYLINE_CLOSED, POLYLINE_CURVE, POLYLINE_SMOOTH_CURVE};
use crate::line_type::LineType;
use crate::object::{LAYER_VISIBLE, Object};
use crate::{ObjectReader, ParseFailure};
use dxf::stream_writer::handles_needed;
use std::collections::BTreeMap;
use std::io::Write;

/// Segment values at or above this refer to symbols rather than lengths.
const SYMBOL_SEGMENT: f32 = 1000.0;

/// Application name of the extended data that holds point names and feature codes.
const APP_ID: &str = "NCZ";

/// Prefix of the blocks that stand in for symbols. The shapes are in NetCAD's symbol libraries (`.NCS`), which are
/// not part of the NCZ file, so each block holds only the symbol's footprint and can be redefined with the real shape.
const SYMBOL_BLOCK_PREFIX: &str = "NCZ_SYMBOL_";

/// Straight segments that draw each span of a curved polyline.
const CURVE_SEGMENTS_PER_SPAN: usize = 8;

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

/// A point on the centripetal Catmull-Rom span from `p1` to `p2`, with `p0` and `p3` as the neighbours, at `t`
/// between 0 and 1. The centripetal form passes through every point without cusps or loops. Elevations are
/// interpolated linearly, so the curve does not invent heights.
fn catmull_rom(
    p0: Coordinate,
    p1: Coordinate,
    p2: Coordinate,
    p3: Coordinate,
    t: f64,
) -> Coordinate {
    // Knot spacing is the square root of the distance. Coincident neighbours get a tiny spacing instead of zero.
    let knot = |a: Coordinate, b: Coordinate| (a.x - b.x).hypot(a.y - b.y).sqrt().max(1e-9);
    let t1 = knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let t = t1 + t * (t2 - t1);

    let lerp = |a: (f64, f64), b: (f64, f64), ta: f64, tb: f64| {
        let (wa, wb) = ((tb - t) / (tb - ta), (t - ta) / (tb - ta));
        (a.0 * wa + b.0 * wb, a.1 * wa + b.1 * wb)
    };
    let (q0, q1, q2, q3) = ((p0.x, p0.y), (p1.x, p1.y), (p2.x, p2.y), (p3.x, p3.y));
    let a1 = lerp(q0, q1, 0.0, t1);
    let a2 = lerp(q1, q2, t1, t2);
    let a3 = lerp(q2, q3, t2, t3);
    let b1 = lerp(a1, a2, 0.0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    let (x, y) = lerp(b1, b2, t1, t2);

    let s = (t - t1) / (t2 - t1);
    Coordinate {
        x,
        y,
        z: p1.z + (p2.z - p1.z) * s,
    }
}

/// Replaces the points of a curved polyline with points along a curve through them. NetCAD 5 leaves curves out of
/// its DXF export, so its own curve is not known. A centripetal Catmull-Rom spline is assumed, which keeps every
/// stored point. The ends of an open curve continue straight, as if mirrored.
fn curve_points(points: Vec<Coordinate>, closed: bool) -> Vec<Coordinate> {
    let mut points = points;
    points.dedup_by(|a, b| a.x == b.x && a.y == b.y);
    // A closed polyline may repeat its first point at the end.
    if closed
        && points.len() > 1
        && points.first().map(|p| (p.x, p.y)) == points.last().map(|p| (p.x, p.y))
    {
        points.pop();
    }
    let n = points.len();
    if n < 3 {
        return points;
    }

    let mirror = |a: Coordinate, b: Coordinate| Coordinate {
        x: 2.0 * a.x - b.x,
        y: 2.0 * a.y - b.y,
        z: a.z,
    };
    let at = |i: isize| -> Coordinate {
        if closed {
            points[i.rem_euclid(n as isize) as usize]
        } else if i < 0 {
            mirror(points[0], points[1])
        } else if i as usize >= n {
            mirror(points[n - 1], points[n - 2])
        } else {
            points[i as usize]
        }
    };

    let spans = if closed { n } else { n - 1 };
    let mut curve = Vec::with_capacity(spans * CURVE_SEGMENTS_PER_SPAN + 1);
    for span in 0..spans as isize {
        let (p0, p1, p2, p3) = (at(span - 1), at(span), at(span + 1), at(span + 2));
        curve.push(p1);
        for step in 1..CURVE_SEGMENTS_PER_SPAN {
            let t = step as f64 / CURVE_SEGMENTS_PER_SPAN as f64;
            curve.push(catmull_rom(p0, p1, p2, p3, t));
        }
    }
    // A closed polyline is closed by its flag, an open one ends at its last point.
    if !closed {
        curve.push(points[n - 1]);
    }
    curve
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
    let corners = [
        (lower.y, lower.x),
        (upper.y, lower.x),
        (upper.y, upper.x),
        (lower.y, upper.x),
    ];
    let local = corners.map(|(x, y)| {
        let (dx, dy) = (x - position.y, y - position.x);
        (
            (dx * cos + dy * sin) / scale,
            (-dx * sin + dy * cos) / scale,
        )
    });
    let quarter = std::f64::consts::FRAC_PI_2;

    Some(SymbolFootprint {
        min: local
            .iter()
            .fold((f64::INFINITY, f64::INFINITY), |(x, y), p| {
                (x.min(p.0), y.min(p.1))
            }),
        max: local
            .iter()
            .fold((f64::NEG_INFINITY, f64::NEG_INFINITY), |(x, y), p| {
                (x.max(p.0), y.max(p.1))
            }),
        deviation: (rotation - (rotation / quarter).round() * quarter).abs(),
    })
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
        entities: vec![entity],
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

/// Point and polyline names, polyline ids and areas, feature codes and links as extended data, as key-value pairs.
/// `None` when the geometry has none of them.
fn x_data(geometry: &Geometry) -> Option<dxf::XData> {
    let text = |value: &String| (!value.is_empty()).then(|| dxf::XDataItem::Str(value.clone()));
    let header = geometry.header();
    let mut values = vec![];

    match geometry {
        Geometry::Point { name, .. } => values.push(("NAME", text(name))),
        // The area is the stored one, which on parcels is close to but not the area of the points, probably the
        // registered area. Zero means none is stored.
        Geometry::PolyLine { name, id, area, .. } => values.extend([
            ("NAME", text(name)),
            ("ID", i32::try_from(*id).ok().map(dxf::XDataItem::Long)),
            (
                "AREA",
                (*area != 0.0).then_some(dxf::XDataItem::Real(*area)),
            ),
        ]),
        _ => {}
    }
    values.push(("CODE", header.code.as_ref().and_then(text)));
    values.push(("LINK", header.link.as_ref().and_then(text)));

    let items: Vec<dxf::XDataItem> = values
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| [dxf::XDataItem::Str(key.to_owned()), value]))
        .flatten()
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
            let closed = flags & POLYLINE_CLOSED != 0;
            let points = if flags & (POLYLINE_CURVE | POLYLINE_SMOOTH_CURVE) != 0 {
                curve_points(points, closed)
            } else {
                points
            };
            polyline(drawing, points, closed)
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

/// The DXF type name of an entity, as the summary counts it.
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

/// Names that geometries refer to by index, from the table objects read so far.
#[derive(Default)]
struct Names {
    /// Layer names by index, geometries refer to layers by their index in the layer table.
    layers: Vec<String>,
    /// Line type names by index, `None` where the slot could not be converted.
    line_types: Vec<Option<String>>,
    /// Text style names by font index, `None` where the font could not be converted.
    text_styles: Vec<Option<String>>,
}

/// The line type of the drawing with the name, ignoring case as DXF does.
fn find_line_type(drawing: &dxf::Drawing, name: &str) -> Option<String> {
    drawing
        .line_types()
        .find(|l| l.name.eq_ignore_ascii_case(name))
        .map(|l| l.name.clone())
}

/// The text style of the drawing with the name, ignoring case.
fn find_style(drawing: &dxf::Drawing, name: &str) -> Option<String> {
    drawing
        .styles()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .map(|s| s.name.clone())
}

impl Names {
    /// Adds the entries of a table object to the drawing and records their names.
    fn add(&mut self, drawing: &mut dxf::Drawing, object: &Object) {
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
                    self.layers.push(layer.name.clone());
                }
            }
            Object::LineTypes { types } => {
                for line_type in types {
                    // A repeated name reuses the first line type.
                    let name = dxf_line_type(line_type).map(|dxf_line_type| {
                        find_line_type(drawing, &dxf_line_type.name).unwrap_or_else(|| {
                            let name = dxf_line_type.name.clone();
                            drawing.add_line_type(dxf_line_type);
                            name
                        })
                    });
                    self.line_types.push(name);
                }
            }
            Object::Fonts { names } => {
                for font in names {
                    // Fonts that differ only in extension, e.g. `ARIAL.NCF` and `ARIAL.NCI`, share a style.
                    let name = text_style(font).map(|style| {
                        find_style(drawing, &style.name).unwrap_or_else(|| {
                            let name = style.name.clone();
                            drawing.add_style(style);
                            name
                        })
                    });
                    self.text_styles.push(name);
                }
            }
            _ => {}
        }
    }

    /// Records the names of a table object whose entries [`Names::add`] already added, giving the same names.
    fn find(&mut self, drawing: &dxf::Drawing, object: &Object) {
        match object {
            Object::Layers { layers, .. } => {
                self.layers
                    .extend(layers.iter().map(|layer| layer.name.clone()));
            }
            Object::LineTypes { types } => {
                self.line_types.extend(types.iter().map(|line_type| {
                    dxf_line_type(line_type).and_then(|l| find_line_type(drawing, &l.name))
                }));
            }
            Object::Fonts { names } => {
                self.text_styles.extend(
                    names
                        .iter()
                        .map(|font| text_style(font).and_then(|s| find_style(drawing, &s.name))),
                );
            }
            _ => {}
        }
    }
}

/// The entities that draw the geometry, with its layer, line type, text style, color and extended data. Polyline
/// vertices take handles from `drawing`.
fn geometry_entities(
    drawing: &mut dxf::Drawing,
    names: &Names,
    geometry: Geometry,
) -> Vec<dxf::entities::Entity> {
    let layer = names
        .layers
        .get(usize::from(geometry.header().layer_id))
        .filter(|name| !name.is_empty())
        .cloned();
    // Text has a line type index too, but DXF text is not drawn with one.
    let line_type = match geometry {
        Geometry::Text { .. } => None,
        _ => names
            .line_types
            .get(usize::from(geometry.header().line_type))
            .cloned()
            .flatten(),
    };
    let text_style = match geometry {
        Geometry::Text { font, .. } => names.text_styles.get(usize::from(font)).cloned().flatten(),
        _ => None,
    };
    let x_data = x_data(&geometry);
    let color = geometry.header().color;

    entity_types(drawing, geometry)
        .into_iter()
        .map(|entity_type| {
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
            entity
        })
        .collect()
}

/// What the DXF output holds, for the summary.
#[derive(Debug, Default)]
pub struct DxfContents {
    /// Number of entities of each DXF type, in the model space and in blocks.
    pub entities: BTreeMap<&'static str, usize>,
    pub layers: usize,
    pub line_types: usize,
    pub blocks: usize,
}

/// Converts an NCZ file to DXF and writes it. Everything but the entities of the model space is made in a first
/// pass over the file, and those entities in a second pass, written as they are made, so neither the objects of the
/// file nor the entities are held at once. `inspect` sees every object and failure of the first pass.
pub fn write_dxf(
    bytes: &[u8],
    writer: &mut impl Write,
    mut inspect: impl FnMut(&Result<Object, ParseFailure>),
) -> Result<DxfContents, Box<dyn std::error::Error>> {
    let mut drawing = dxf::Drawing::new();
    // R12 has neither extended data nor a thumbnail.
    drawing.header.version = dxf::enums::AcadVersion::R2000;
    drawing.add_app_id(dxf::tables::AppId {
        name: APP_ID.to_owned(),
        ..Default::default()
    });
    // Polyline vertices of the model space take their handles from here, since the stream writer replaces them.
    let mut scratch = dxf::Drawing::new();
    let mut contents = DxfContents::default();
    let mut count = |entity: &dxf::entities::Entity| {
        *contents
            .entities
            .entry(entity_name(&entity.specific))
            .or_default() += 1;
    };

    // First pass: tables, blocks, header values, and what the entities of the model space need.
    let mut names = Names::default();
    // Geometries between a block's begin and end belong to that block, not to the model space.
    let mut block: Option<dxf::Block> = None;
    // Footprints of the symbols in use, measured on the instance closest to a right angle.
    let mut symbol_footprints: BTreeMap<u16, Option<SymbolFootprint>> = BTreeMap::new();
    let mut reserved_handles = 0;
    for item in ObjectReader::new(bytes)? {
        inspect(&item);
        let Ok(object) = item else { continue };
        names.add(&mut drawing, &object);
        match object {
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

                match &mut block {
                    Some(block) => {
                        for entity in geometry_entities(&mut drawing, &names, geometry) {
                            count(&entity);
                            block.entities.push(entity);
                        }
                    }
                    None => {
                        for entity in geometry_entities(&mut scratch, &names, geometry) {
                            count(&entity);
                            drawing.prepare_entity(&entity);
                            reserved_handles += handles_needed(&entity);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for (symbol, footprint) in symbol_footprints {
        let block = symbol_block(&mut drawing, symbol, footprint);
        block.entities.iter().for_each(&mut count);
        drawing.add_block(block);
    }
    contents.layers = drawing.layers().count();
    contents.line_types = drawing.line_types().count();
    contents.blocks = drawing.blocks().count();

    // Second pass: the entities of the model space.
    let mut stream = drawing.save_streaming(writer, reserved_handles)?;
    let mut names = Names::default();
    let mut in_block = false;
    for item in ObjectReader::new(bytes)? {
        let Ok(object) = item else { continue };
        names.find(&drawing, &object);
        match object {
            Object::BlockBegin { .. } => in_block = true,
            Object::BlockEnd => in_block = false,
            Object::Geometry { geometry } if !in_block => {
                for entity in geometry_entities(&mut scratch, &names, geometry) {
                    stream.write_entity(entity)?;
                }
            }
            _ => {}
        }
    }
    stream.finish()?;

    Ok(contents)
}
