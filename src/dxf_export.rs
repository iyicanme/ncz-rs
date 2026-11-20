use crate::coordinate::Coordinate;
use crate::geometry::{Geometry, Header, POLYLINE_CLOSED, POLYLINE_CURVE, POLYLINE_SMOOTH_CURVE};
use crate::line_type::LineType;
use crate::colors::{DEFAULT_PALETTE, dxf_color};
use crate::object::{LAYER_POINT_MARKERS, LAYER_VISIBLE, MetadataContent, Object, POINT_MARKERS_SWITCH};
use crate::map_sheet;
use crate::projection::TransverseMercator;
use crate::symbols::{self, Shape};
use crate::{ObjectReader, ParseFailure};
use dxf::stream_writer::handles_needed;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

/// Segment values at or above this refer to symbols rather than lengths.
const SYMBOL_SEGMENT: f32 = 1000.0;

/// Application name of the extended data that holds point names and feature codes.
const APP_ID: &str = "NCZ";

/// How far a map sheet's projected corners may be from its stored bounds, in drawing units, for the projection to be
/// taken. NetCAD's own corners match its bounds to the millimetre.
const MAP_SHEET_TOLERANCE: f64 = 0.01;

/// Prefix of the blocks that draw symbols. The shapes are not part of the NCZ file. Symbols whose shape is embedded
/// (see `symbols/README.md`) are drawn with it, the others with their footprint, and those blocks can be redefined
/// with the real shape.
const SYMBOL_BLOCK_PREFIX: &str = "NCZ_SYMBOL_";

/// Straight segments that draw each span of a curved polyline.
const CURVE_SEGMENTS_PER_SPAN: usize = 8;

/// Slant of italic text in degrees. DXF has no italic flag for text, so it is drawn obliqued.
const ITALIC_OBLIQUE_ANGLE: f64 = 15.0;

/// The elevations kept, in metres. The corpus' real elevations lie between about -200 and 1,700 m (99.9% of the
/// non-zero values); the range adds a margin for the rest of Türkiye, up to Ağrı at 5,137 m.
const ELEVATIONS: std::ops::RangeInclusive<f64> = -500.0..=6000.0;
/// Elevations closer to 0 are noise: survey heights have millimetre precision.
const ELEVATION_NOISE: f64 = 1e-4;

/// An elevation as it is written, 0 when it is not a real elevation. Many files hold leftover memory in elevations
/// they do not use: 64,000 values of the corpus lie between 1,700 m and 1e38 (constant odd values along whole
/// polylines, such as 6750.208 on 614 vertices), 7,500 are not finite, which readers such as AutoCAD reject, and
/// 600,000 are below 1e-6. NetCAD's own export writes them as they are.
fn elevation(z: f64) -> f64 {
    if ELEVATIONS.contains(&z) && z.abs() >= ELEVATION_NOISE {
        z
    } else {
        0.0
    }
}

/// NCZ stores northing as x and easting as y, DXF puts easting on X. Angles are already in the east-north frame.
/// Elevations are cleaned, see [`elevation`], and other values that are not finite become 0.
impl From<Coordinate> for dxf::Point {
    fn from(value: Coordinate) -> Self {
        let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
        dxf::Point::new(finite(value.y), finite(value.x), elevation(value.z))
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
    // Garbage elevations would otherwise blend into the points in between.
    let mut points: Vec<Coordinate> = points
        .into_iter()
        .map(|point| Coordinate {
            z: elevation(point.z),
            ..point
        })
        .collect();
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

/// A polyline through the points: a 2D `LWPOLYLINE` at their elevation when they all share one, which is smaller and
/// more widely supported, else a 3D `POLYLINE`.
fn polyline(
    drawing: &mut dxf::Drawing,
    points: Vec<Coordinate>,
    closed: bool,
) -> dxf::entities::Entity {
    let points: Vec<dxf::Point> = points.into_iter().map(Into::into).collect();
    if points.windows(2).all(|pair| pair[0].z == pair[1].z) {
        let mut polyline = dxf::entities::LwPolyline::default();
        polyline.set_is_closed(closed);
        polyline.vertices = points
            .iter()
            .map(|point| dxf::LwPolylineVertex {
                x: point.x,
                y: point.y,
                ..Default::default()
            })
            .collect();
        let mut entity = dxf::entities::Entity::new(dxf::entities::EntityType::LwPolyline(polyline));
        entity.common.elevation = points.first().map_or(0.0, |point| point.z);
        return entity;
    }

    let mut polyline = dxf::entities::Polyline::default();
    polyline.set_is_3d_polyline(true);
    polyline.set_is_closed(closed);
    for point in points {
        let mut vertex = dxf::entities::Vertex::new(point);
        vertex.set_is_3d_polyline_vertex(true);
        polyline.add_vertex(drawing, vertex);
    }
    dxf::entities::Entity::new(dxf::entities::EntityType::Polyline(polyline))
}

/// An axis aligned rectangle with the given opposite corners, at the elevation of the first.
fn rectangle(
    drawing: &mut dxf::Drawing,
    corner1: Coordinate,
    corner2: Coordinate,
) -> dxf::entities::Entity {
    rotated_rectangle(drawing, corner1, corner2, 0.0)
}

/// A rectangle given by two opposite corners before rotation, turned by `rotation` radians counter-clockwise
/// (in the east-north frame) around `corner1`.
fn rotated_rectangle(
    drawing: &mut dxf::Drawing,
    corner1: Coordinate,
    corner2: Coordinate,
    rotation: f64,
) -> dxf::entities::Entity {
    let z = elevation(corner1.z);
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

/// The block of a symbol, e.g. `NCZ_SYMBOL_F2_126`, or `NCZ_SYMBOL_126` when its library is not known.
fn symbol_block_name(library: Option<&str>, symbol: u16) -> String {
    match library {
        Some(library) => format!("{SYMBOL_BLOCK_PREFIX}{library}_{symbol}"),
        None => format!("{SYMBOL_BLOCK_PREFIX}{symbol}"),
    }
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

/// Draws the entity by block, so it takes the layer, color and line type of each insert.
fn by_block(mut entity: dxf::entities::Entity) -> dxf::entities::Entity {
    entity.common.color = dxf::Color::by_block();
    entity.common.line_type_name = "BYBLOCK".to_owned();
    entity
}

/// A block that draws a symbol with its shape, drawn by block.
fn symbol_shape_block(name: String, shapes: &[Shape]) -> dxf::Block {
    let point = |(x, y): (f64, f64)| dxf::Point::new(x, y, 0.0);
    let entities = shapes
        .iter()
        .map(|&shape| {
            let specific = match shape {
                Shape::Line { start, end } => dxf::entities::EntityType::Line(
                    dxf::entities::Line::new(point(start), point(end)),
                ),
                Shape::Circle { center, radius } => dxf::entities::EntityType::Circle(
                    dxf::entities::Circle::new(point(center), radius),
                ),
            };
            by_block(dxf::entities::Entity::new(specific))
        })
        .collect();

    dxf::Block {
        name,
        entities,
        ..Default::default()
    }
}

/// A block that stands in for a symbol whose shape is not known: its footprint, or a unit square around the insertion
/// point when no instance has usable bounds. It is drawn by block.
fn symbol_footprint_block(
    drawing: &mut dxf::Drawing,
    name: String,
    footprint: Option<SymbolFootprint>,
) -> dxf::Block {
    let (min, max) = footprint
        .map(|footprint| (footprint.min, footprint.max))
        .filter(|(min, max)| max.0 - min.0 > f64::EPSILON || max.1 - min.1 > f64::EPSILON)
        .unwrap_or(((-0.5, -0.5), (0.5, 0.5)));
    // `rectangle` takes NCZ coordinates, which have the axes the other way round.
    let corner = |(x, y)| Coordinate { x: y, y: x, z: 0.0 };

    let entity = by_block(rectangle(drawing, corner(min), corner(max)));

    dxf::Block {
        name,
        entities: vec![entity],
        ..Default::default()
    }
}

/// Maps an alignment code to a DXF justification, as NetCAD 5 exports it (about 320,000 texts of the corpus). The
/// digits do not follow the numeric keypad layout. Other codes (`A`, `E`, `H`, `O`, NUL in the corpus) are left
/// baseline, as NetCAD exports them.
fn text_justification(
    snap: char,
) -> (
    dxf::enums::HorizontalTextJustification,
    dxf::enums::VerticalTextJustification,
) {
    use dxf::enums::HorizontalTextJustification as H;
    use dxf::enums::VerticalTextJustification as V;

    match snap {
        '7' => (H::Center, V::Baseline),
        '8' => (H::Right, V::Baseline),
        'C' => (H::Center, V::Bottom),
        'R' => (H::Right, V::Bottom),
        '1' | 'P' => (H::Left, V::Middle),
        'M' => (H::Center, V::Middle),
        '5' | 'Q' => (H::Right, V::Middle),
        '2' => (H::Left, V::Top),
        '3' | 'U' => (H::Center, V::Top),
        '4' => (H::Right, V::Top),
        // 'L', '6' and the rest.
        _ => (H::Left, V::Baseline),
    }
}

/// Advance widths of NetCAD's stroke font `ARIAL.NCF`, in 24ths of the text height, fitted to where NetCAD 5's DXF
/// export puts the alignment point of about 200,000 centred and right-aligned texts of the corpus (within 0.01 of the
/// height for 99% of them). Characters that are not listed take [`ARIAL_DEFAULT_WIDTH`].
#[rustfmt::skip]
const ARIAL_WIDTHS: &[(char, u8)] = &[
    (' ', 16), ('\'', 6), ('(', 11), (')', 11), ('+', 20), (',', 9), ('-', 11), ('.', 9), ('/', 9), ('0', 19),
    ('1', 19), ('2', 19), ('3', 19), ('4', 19), ('5', 19), ('6', 19), ('7', 19), ('8', 19), ('9', 19), (':', 9),
    ('=', 20), ('?', 19), ('A', 22), ('B', 22), ('C', 24), ('D', 24), ('E', 22), ('F', 21), ('G', 26), ('H', 24),
    ('I', 9), ('J', 17), ('K', 22), ('L', 19), ('M', 28), ('N', 24), ('O', 26), ('P', 22), ('R', 24), ('S', 22),
    ('T', 20), ('U', 24), ('V', 22), ('W', 33), ('X', 22), ('Y', 22), ('Z', 21), ('\\', 9), ('^', 13), ('_', 19),
    ('a', 18), ('b', 18), ('c', 17), ('d', 18), ('e', 18), ('f', 10), ('g', 18), ('h', 19), ('i', 7), ('j', 7),
    ('k', 17), ('l', 7), ('m', 28), ('n', 19), ('o', 18), ('p', 18), ('r', 11), ('s', 17), ('t', 9), ('u', 19),
    ('v', 16), ('y', 16), ('z', 16), ('²', 11), ('Ç', 24), ('Ö', 26), ('Ü', 24), ('ç', 17), ('ö', 18), ('ü', 19),
    ('Ğ', 26), ('ğ', 18), ('İ', 9), ('ı', 9), ('Ş', 22), ('ş', 17),
];
const ARIAL_DEFAULT_WIDTH: u8 = 19;
/// Advance widths of `F1.NCF`, fitted like [`ARIAL_WIDTHS`] to about 19,000 texts. It is nearly monospaced.
#[rustfmt::skip]
const F1_WIDTHS: &[(char, u8)] = &[
    (' ', 16), ('(', 13), (')', 13), ('+', 21), (',', 9), ('-', 10), ('.', 6), ('/', 21), ('0', 21), ('1', 21),
    ('2', 21), ('3', 21), ('4', 21), ('5', 21), ('6', 21), ('7', 21), ('8', 21), ('9', 21), ('?', 17), ('A', 21),
    ('B', 21), ('C', 21), ('D', 21), ('E', 21), ('F', 21), ('G', 21), ('H', 21), ('I', 13), ('K', 21), ('L', 21),
    ('M', 21), ('N', 21), ('O', 21), ('P', 21), ('R', 21), ('S', 21), ('T', 21), ('U', 21), ('V', 21), ('W', 21),
    ('X', 21), ('Y', 21), ('Z', 21), ('a', 21), ('b', 21), ('c', 21), ('d', 21), ('e', 21), ('f', 21), ('g', 21),
    ('h', 21), ('i', 9), ('j', 17), ('k', 21), ('l', 9), ('m', 21), ('n', 17), ('o', 21), ('p', 21), ('r', 21),
    ('s', 21), ('t', 21), ('u', 18), ('v', 21), ('y', 21), ('z', 21), ('Ç', 21), ('Ö', 21), ('Ü', 21), ('ç', 21),
    ('ö', 21), ('ü', 18), ('Ğ', 21), ('ğ', 21), ('İ', 11), ('ı', 5), ('Ş', 21), ('ş', 21),
];
const F1_DEFAULT_WIDTH: u8 = 21;

/// The width of a text as NetCAD measures it, in text heights. The style is the text style the font became (`F1` for
/// `F1.NCF`); fonts other than `F1` are measured as `ARIAL`, which nearly all corpus texts use. A positive `spacing`
/// scales the advance, 0 means 1.
fn text_width(style: Option<&str>, text: &str, spacing: f32) -> f64 {
    let (widths, default) = match style {
        Some(style) if style.eq_ignore_ascii_case("F1") => (F1_WIDTHS, F1_DEFAULT_WIDTH),
        _ => (ARIAL_WIDTHS, ARIAL_DEFAULT_WIDTH),
    };
    let units: u32 = text
        .chars()
        .map(|c| {
            let width = widths
                .iter()
                .find(|&&(known, _)| known == c)
                .map_or(default, |&(_, width)| width);
            u32::from(width)
        })
        .sum();
    let spacing = if spacing.is_finite() && spacing > 0.0 {
        f64::from(spacing)
    } else {
        1.0
    };
    f64::from(units) / 24.0 * spacing
}

/// Where NetCAD 5 puts a text's alignment point (group 11) and group 10, relative to the stored point, by alignment
/// code: group 11 is `a·width + b·height` along the baseline and `c·height` across it (to the left), group 10 is
/// `d·height` along and `e·height` across. The stored point is the lower-left corner of the text for most codes; `P`,
/// `Q` and `U` put the text half a height beside or above it. Matches 99.96% of 238,000 corpus texts to 0.02 of the
/// height.
fn text_offsets(snap: char) -> [f64; 5] {
    match snap {
        '7' => [0.5, 0.0, -0.5, 0.0, 0.0],
        '8' => [1.0, 0.0, -0.5, 0.0, 0.0],
        'C' => [0.5, 0.0, 0.0, 0.0, 0.0],
        'R' => [1.0, 0.0, 0.0, 0.0, 0.0],
        '1' => [0.0, 0.0, 0.5, 0.0, 0.0],
        'M' => [0.5, 0.0, 0.5, 0.0, 0.0],
        '5' => [1.0, 0.0, 0.5, 0.0, 0.0],
        '2' => [0.0, 0.0, 1.0, 0.0, 0.0],
        '3' => [0.5, 0.0, 1.0, 0.0, 0.0],
        '4' => [1.0, 0.0, 1.0, 0.0, 0.0],
        'P' => [0.0, -0.5, 0.5, 0.5, 0.0],
        'Q' => [1.0, 0.5, 0.5, -0.5, 0.0],
        'U' => [0.5, 0.0, 1.5, 0.0, -1.5],
        // 'L', '6' and the rest are left baseline at the stored point.
        _ => [0.0; 5],
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

/// Symbol values of a line type pattern: `k·1000 + n` puts symbol `n` of the first library at the current place, at
/// `k` times its size, turned along the line, and `-(k·1000 + n)` puts it turned by a further half turn. Symbols take
/// no length. The size factor matches NetCAD 5's export of `-2128`/`-4128`, `-2132`, `-2208` and `-4181` in the corpus.
/// Values with `n` of 256 or more are left out.
const PATTERN_SYMBOL_NUMBERS: u16 = 256;
/// Size factors above this are taken as garbage, the corpus has up to 18.
const MAX_PATTERN_SYMBOL_SIZE: f64 = 99.0;

/// Repeats of a pattern along a single line beyond which its symbols are left out, so that a pattern of tiny lengths
/// on a long line cannot flood the drawing.
const MAX_PATTERN_REPEATS: f64 = 10_000.0;

/// Line types in a chain of parallel lines beyond which the chain is cut, which also ends chains that loop.
const MAX_CHAIN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
enum PatternElement {
    /// A dash or a gap, as a length.
    Length(f64),
    Symbol {
        number: u16,
        /// The factor of the symbol's size, `k` of `k·1000 + n`.
        size: f64,
        turned: bool,
    },
}

/// What a line type draws along a line besides its dashes, which the DXF line type draws: symbols, and a further line
/// type drawn parallel to it. NetCAD 5 draws both along lines only, not along polylines or arcs, and so does the DXF
/// export.
#[derive(Clone, Debug, PartialEq)]
struct LinePattern {
    elements: Vec<PatternElement>,
    /// The length of one repeat of the pattern, in pattern units.
    length: f64,
    /// A line type with a negative segment count also draws the line type `unknown1`, at `offset` to the right of it,
    /// which can chain on in turn. This matches every line of NetCAD's export of `DUVAR-1` (4 parallel lines), `DUV3`
    /// and `ISTINAT` in the sample pairs. A chain ends at a line type with a positive count, often `DUZ`.
    parallel: Option<(f64, usize)>,
}

impl LinePattern {
    /// The symbols and parallel line type of a line type, `None` when it has neither. Symbols are only kept when the
    /// pattern has a length, otherwise they would all be at the start.
    fn new(line_type: &LineType) -> Option<Self> {
        let mut elements = vec![];
        for &segment in &line_type.segments {
            let value = f64::from(segment);
            if !value.is_finite() {
                return None;
            }
            if value.abs() < f64::from(SYMBOL_SEGMENT) {
                elements.push(PatternElement::Length(value.abs()));
            } else if value.fract() == 0.0 {
                let size = (value.abs() / 1000.0).floor();
                let number = value.abs() - size * 1000.0;
                if size <= MAX_PATTERN_SYMBOL_SIZE && number < f64::from(PATTERN_SYMBOL_NUMBERS) {
                    elements.push(PatternElement::Symbol {
                        number: number as u16,
                        size,
                        turned: value < 0.0,
                    });
                }
            }
        }
        let length: f64 = elements
            .iter()
            .map(|element| match element {
                PatternElement::Length(length) => *length,
                PatternElement::Symbol { .. } => 0.0,
            })
            .sum();
        if length <= 0.0 {
            elements.clear();
        }
        // Unused slots hold leftover memory, so huge offsets are not taken.
        let offset = f64::from(line_type.offset);
        let parallel = (line_type.segment_count < 0 && offset.is_finite() && offset.abs() < 1e3)
            .then_some((offset, usize::from(line_type.unknown1)));

        let has_symbols = elements
            .iter()
            .any(|element| matches!(element, PatternElement::Symbol { .. }));
        (has_symbols || parallel.is_some()).then_some(Self {
            elements,
            length,
            parallel,
        })
    }

    fn symbols(&self) -> impl Iterator<Item = u16> + '_ {
        self.elements.iter().filter_map(|element| match element {
            PatternElement::Symbol { number, .. } => Some(*number),
            PatternElement::Length(_) => None,
        })
    }

    /// The symbols along a line of the given length, as the distance from its start, the symbol number, its size
    /// factor and whether it is turned. The pattern starts at the start of the line, with lengths multiplied by
    /// `scale`.
    fn symbol_places(&self, scale: f64, length: f64) -> Vec<(f64, u16, f64, bool)> {
        let repeat = self.length * scale;
        if repeat.is_nan() || repeat <= 0.0 || length / repeat > MAX_PATTERN_REPEATS {
            return vec![];
        }
        // Tolerates rounding, so a symbol at the very end of the line is kept.
        let end = length + 1e-9 * length.max(1.0);

        let mut places = vec![];
        let mut along = 0.0;
        while along <= end {
            for element in &self.elements {
                match *element {
                    PatternElement::Length(length) => along += length * scale,
                    PatternElement::Symbol {
                        number,
                        size,
                        turned,
                    } if along <= end => {
                        places.push((along, number, size, turned));
                    }
                    PatternElement::Symbol { .. } => {}
                }
            }
        }
        places
    }
}

/// The line types drawn along a line of the line type with the index, as the distance to the left of the line, in
/// pattern units, and the line type index. The first is the line type itself, at distance 0.
fn line_type_chain(names: &Names, line_type: usize) -> Vec<(f64, usize)> {
    let mut chain = vec![(0.0, line_type)];
    let (mut shift, mut index) = (0.0, line_type);
    while chain.len() < MAX_CHAIN {
        let Some((offset, next)) = names
            .line_pattern(index)
            .and_then(|pattern| pattern.parallel)
        else {
            break;
        };
        if next >= names.line_types.len() {
            break;
        }
        (shift, index) = (shift - offset, next);
        chain.push((shift, index));
    }
    chain
}

/// The symbols and parallel lines that the line type with the index draws along a line from `begin` to `end`, with
/// the line type of each parallel line. Only symbols whose shape is embedded are drawn.
fn line_pattern_entities(
    names: &Names,
    line_type: usize,
    begin: Coordinate,
    end: Coordinate,
) -> Vec<(dxf::entities::EntityType, Option<String>)> {
    let (begin, end): (dxf::Point, dxf::Point) = (begin.into(), end.into());
    let (dx, dy, dz) = (end.x - begin.x, end.y - begin.y, end.z - begin.z);
    let length = dx.hypot(dy);
    if length <= 0.0 {
        return vec![];
    }
    let (ux, uy) = (dx / length, dy / length);
    let scale = names.pattern_scale();
    let library = names.library(0);
    let angle = dy.atan2(dx).to_degrees();
    let mut entities = vec![];

    for (level, (shift, index)) in line_type_chain(names, line_type).into_iter().enumerate() {
        // The left normal is `(-uy, ux)`.
        let (sx, sy) = (-uy * shift * scale, ux * shift * scale);
        let start = dxf::Point::new(begin.x + sx, begin.y + sy, begin.z);
        // The line itself is drawn by its geometry.
        if level > 0 {
            let finish = dxf::Point::new(end.x + sx, end.y + sy, end.z);
            let line = dxf::entities::Line::new(start.clone(), finish);
            let line_type = names.line_types.get(index).cloned().flatten();
            entities.push((dxf::entities::EntityType::Line(line), line_type));
        }

        let Some(pattern) = names.line_pattern(index) else {
            continue;
        };
        for (along, number, size, turned) in pattern.symbol_places(scale, length) {
            if library.is_none_or(|library| symbols::find(library, number).is_none()) {
                continue;
            }
            let insert = dxf::entities::Insert {
                name: symbol_block_name(library, number),
                location: dxf::Point::new(
                    start.x + ux * along,
                    start.y + uy * along,
                    start.z + dz * along / length,
                ),
                x_scale_factor: scale * size,
                y_scale_factor: scale * size,
                rotation: if turned { angle + 180.0 } else { angle },
                ..Default::default()
            };
            // Symbols are drawn solid.
            entities.push((
                dxf::entities::EntityType::Insert(insert),
                Some("CONTINUOUS".to_owned()),
            ));
        }
    }
    entities
}

/// The marker NetCAD draws at a point, besides the point itself. Measured on NetCAD 5's DXF export of the four sample
/// drawings, which all have map scale 1, so how the markers follow the map scale is assumed, as for line patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PointMarker {
    /// A circle with a dot, at every point of a layer with [`LAYER_POINT_MARKERS`].
    Circle,
    /// A larger circle, at traverse points (named `P.…`) of other layers.
    Traverse,
    /// A triangle with a cross, at triangulation points (named `N.…`) of other layers.
    Triangulation,
}

impl PointMarker {
    /// The marker of a point with the name on a layer whose points get markers, or not, see
    /// [`LAYER_POINT_MARKERS`]. Other points get none. The name decides first: in the corpus, 841 traverse points
    /// on such layers get only the traverse circle, and triangulation points only the triangle, whether the layer
    /// and file draw markers or not.
    fn of(name: &str, marked_layer: bool) -> Option<Self> {
        if name.starts_with("P.") {
            Some(Self::Traverse)
        } else if name.starts_with("N.") {
            Some(Self::Triangulation)
        } else if marked_layer {
            Some(Self::Circle)
        } else {
            None
        }
    }

    fn block_name(self) -> &'static str {
        match self {
            Self::Circle => "NCZ_POINT",
            Self::Traverse => "NCZ_POINT_TRAVERSE",
            Self::Triangulation => "NCZ_POINT_TRIANGULATION",
        }
    }

    /// The shapes for map scale 1, around the point. The dot is a short diagonal line, as NetCAD draws it.
    fn shapes(self) -> &'static [Shape] {
        const fn line(x1: f64, y1: f64, x2: f64, y2: f64) -> Shape {
            Shape::Line {
                start: (x1, y1),
                end: (x2, y2),
            }
        }
        const fn circle(radius: f64) -> Shape {
            Shape::Circle {
                center: (0.0, 0.0),
                radius,
            }
        }
        const CIRCLE: &[Shape] = &[circle(0.65), line(-0.041, -0.041, 0.041, 0.041)];
        const TRAVERSE: &[Shape] = &[circle(1.2)];
        const TRIANGULATION: &[Shape] = &[
            line(0.0, 2.0, -1.75, -1.0),
            line(-1.75, -1.0, 1.75, -1.0),
            line(1.75, -1.0, 0.0, 2.0),
            line(-0.125, -0.125, 0.125, 0.125),
            line(-0.125, 0.125, 0.125, -0.125),
        ];
        match self {
            Self::Circle => CIRCLE,
            Self::Traverse => TRAVERSE,
            Self::Triangulation => TRIANGULATION,
        }
    }
}

/// Whether the file draws the markers of layers with [`LAYER_POINT_MARKERS`], from the layer table header.
fn point_markers_on(header: &[u8]) -> bool {
    header.get(POINT_MARKERS_SWITCH) == Some(&1)
}

/// The marker of a point geometry, see [`PointMarker`].
fn point_marker(names: &Names, geometry: &Geometry) -> Option<PointMarker> {
    let Geometry::Point { header, name, .. } = geometry else {
        return None;
    };
    let marked_layer = names
        .point_markers
        .get(usize::from(header.layer_id))
        .copied()
        .unwrap_or(false);
    PointMarker::of(name, marked_layer)
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


/// Whether NetCAD draws a symbol. Symbols with no scale are not drawn, NetCAD stores empty bounds for them. Nor are
/// symbols whose stored bounds are empty (inverted) or a single point: they have no shape. In the corpus these are
/// all symbol 0 of `F2.NCS` or `PLAN.NCS`, which NetCAD draws as nothing.
fn symbol_is_drawn(scale: f32, header: &crate::geometry::Header) -> bool {
    let (lower, upper) = (header.lower_bound, header.upper_bound);
    let has_extent = lower.x <= upper.x && lower.y <= upper.y && (lower.x < upper.x || lower.y < upper.y);
    scale > 0.0 && has_extent
}

/// Whether the geometry has a DXF counterpart, the others are left out of the drawing.
pub fn is_exported(geometry: &Geometry) -> bool {
    match geometry {
        Geometry::Symbol { scale, header, .. } => symbol_is_drawn(*scale, header),
        // Pictures and other type-15 objects are drawn as their frame, when they have one.
        Geometry::Image { header, .. } => header.lower_bound.x <= header.upper_bound.x,
        Geometry::Empty { .. } | Geometry::Other { .. } => false,
        _ => true,
    }
}

/// The entities that draw the geometry, usually one. Contours with gaps are drawn as several polylines.
fn entity_types(
    drawing: &mut dxf::Drawing,
    names: &Names,
    geometry: Geometry,
) -> Vec<dxf::entities::Entity> {
    let entity = match geometry {
        Geometry::Point { position, .. } => {
            let point = dxf::entities::ModelPoint::new(position.into());
            dxf::entities::EntityType::ModelPoint(point)
        }
        Geometry::Line { begin, end, .. } => {
            let line = dxf::entities::Line::new(begin.into(), end.into());
            dxf::entities::EntityType::Line(line)
        }
        // A circle without a radius (16 in the corpus) is not valid in DXF, it marks a point.
        Geometry::Circle { center, radius, .. } if !(radius > 0.0 && radius.is_finite()) => {
            dxf::entities::EntityType::ModelPoint(dxf::entities::ModelPoint::new(center.into()))
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
            spacing,
            font,
            ..
        } => {
            let (horizontal_text_justification, vertical_text_justification) =
                text_justification(snap);
            // NetCAD's alignment point, see `text_offsets`, which CAD programs place the text by.
            let style = names.text_styles.get(usize::from(font)).cloned().flatten();
            let height = f64::from(size);
            let width = text_width(style.as_deref(), &string, spacing) * height;
            let [a, b, c, d, e] = text_offsets(snap);
            let (cos, sin) = (f64::from(tilt).cos(), f64::from(tilt).sin());
            let stored: dxf::Point = position.into();
            let at = |along: f64, across: f64| {
                dxf::Point::new(
                    stored.x + along * cos - across * sin,
                    stored.y + along * sin + across * cos,
                    stored.z,
                )
            };
            let text = dxf::entities::Text {
                location: at(d * height, e * height),
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
                second_alignment_point: at(a * width + b * height, c * height),
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
            return vec![polyline(drawing, points, closed)];
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
        } => return vec![rotated_rectangle(drawing, corner1, corner2, rotation.into())],
        // NetCAD draws a sheet through its true corners, which the name gives (see `map_sheet`). The stored bounds are
        // the box of those corners, and only a projection that reproduces them is taken; otherwise the box is drawn.
        Geometry::MapSheet { header, name } => {
            let (lower, upper): (dxf::Point, dxf::Point) =
                (header.lower_bound.into(), header.upper_bound.into());
            let bounds = [lower.x, lower.y, upper.x, upper.y];
            let corners = map_sheet::projected_corners(&name, names.projection, bounds, MAP_SHEET_TOLERANCE);
            return vec![match corners {
                Some(corners) => {
                    let z = header.lower_bound.z;
                    // NCZ coordinates have northing as x.
                    let points = corners.iter().map(|&(east, north)| Coordinate { x: north, y: east, z }).collect();
                    polyline(drawing, points, true)
                }
                None => rectangle(drawing, header.lower_bound, header.upper_bound),
            }];
        }
        // The frame of a picture, rich text or other type-15 object, whose content is not exported. Its stored bounds
        // are the frame: all 8 in the corpus are unrotated.
        Geometry::Image { header, .. } if header.lower_bound.x <= header.upper_bound.x => {
            return vec![rectangle(drawing, header.lower_bound, header.upper_bound)];
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
            library,
            ref header,
        } if symbol_is_drawn(scale, header) => {
            let insert = dxf::entities::Insert {
                name: symbol_block_name(names.library(library), symbol),
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

    vec![dxf::entities::Entity::new(entity)]
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
        EntityType::LwPolyline(_) => "LWPOLYLINE",
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
    /// Symbol library names by index, e.g. `F2` for `F2.NCS`.
    libraries: Vec<String>,
    /// What line types draw along lines besides dashes, by line type index.
    line_patterns: Vec<Option<LinePattern>>,
    /// The map scale, which scales line type patterns.
    scale: Option<f64>,
    /// The projection `MPROJ` names, which places map sheet corners, see [`map_sheet::projected_corners`].
    projection: Option<TransverseMercator>,
    /// Whether NetCAD draws a marker at the points of the layer, by layer index, see [`LAYER_POINT_MARKERS`] and
    /// [`POINT_MARKERS_SWITCH`].
    point_markers: Vec<bool>,
    /// NetCAD palette numbers of the layers, by layer index.
    layer_colors: Vec<u8>,
    /// The file's own palette entries (from `LEX.ST2`), by palette number. It is only complete once the whole file
    /// was read, so the second pass starts with the first pass's.
    palette: Vec<Option<[u8; 3]>>,
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
    /// The name of the symbol library with the index, `None` when there is none or it cannot be part of a block name.
    fn library(&self, index: u8) -> Option<&str> {
        self.libraries
            .get(usize::from(index))
            .map(String::as_str)
            .filter(|name| is_valid_table_name(name))
    }

    /// The factor of line type pattern lengths and of the symbols along lines, the map scale like `$LTSCALE`.
    /// The DXF colour index and true colour of a NetCAD palette number, see [`crate::colors`]. Numbers whose colour is
    /// not known are written as they are.
    fn color(&self, number: u8) -> (u8, Option<i32>) {
        let rgb = self
            .palette
            .get(usize::from(number))
            .copied()
            .flatten()
            .or(DEFAULT_PALETTE[usize::from(number)]);
        match (rgb, number) {
            (Some(rgb), _) => dxf_color(rgb),
            (None, 0) => (7, None),
            (None, number) => (number, None),
        }
    }

    fn pattern_scale(&self) -> f64 {
        self.scale
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0)
    }

    /// The pattern of the line type with the index, if it draws more than dashes.
    fn line_pattern(&self, index: usize) -> Option<&LinePattern> {
        self.line_patterns.get(index)?.as_ref()
    }

    fn set_libraries(&mut self, names: &[String]) {
        self.libraries = names
            .iter()
            .map(|name| symbols::library_name(name))
            .collect();
    }

    /// Adds the entries of a table object to the drawing and records their names.
    fn add(&mut self, drawing: &mut dxf::Drawing, object: &Object) {
        match object {
            Object::Layers {
                unknown1, layers, ..
            } => {
                let markers_on = point_markers_on(unknown1);
                for layer in layers {
                    let exists = drawing.layers().any(|l| l.name == layer.name);
                    if !layer.name.is_empty() && !exists {
                        // The file's palette comes later, in `LEX.ST2`, which sets the colour again.
                        let (index, true_color) = self.color(layer.color);
                        drawing.add_layer(dxf::tables::Layer {
                            name: layer.name.clone(),
                            color: dxf::Color::from_index(index),
                            color_24_bit: true_color.unwrap_or(0),
                            is_layer_on: layer.flags & LAYER_VISIBLE != 0,
                            ..Default::default()
                        });
                    }
                    self.layers.push(layer.name.clone());
                    self.layer_colors.push(layer.color);
                    self.point_markers
                        .push(markers_on && layer.flags & LAYER_POINT_MARKERS != 0);
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
                self.line_patterns
                    .extend(types.iter().map(LinePattern::new));
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
            Object::Libraries { names } => self.set_libraries(names),
            Object::Scale { scale } => self.scale = Some(*scale),
            Object::Metadata {
                content:
                    MetadataContent::Projection {
                        projection,
                        datum,
                        central_meridian,
                        ..
                    },
                ..
            } => self.projection = TransverseMercator::from_mproj(*projection, *datum, *central_meridian),
            // One entry per layer, in the order of the layer table: the layer's colour, which is also the file's
            // palette entry for the layer's colour number.
            Object::Metadata {
                content: MetadataContent::LayerStyles { layers, .. },
                ..
            } => {
                for ((name, &number), style) in self.layers.iter().zip(&self.layer_colors).zip(layers) {
                    let slot = usize::from(number);
                    if self.palette.len() <= slot {
                        self.palette.resize(slot + 1, None);
                    }
                    self.palette[slot].get_or_insert(style.rgb);
                    let (index, true_color) = dxf_color(style.rgb);
                    if let Some(layer) = drawing.layers_mut().find(|layer| &layer.name == name) {
                        layer.color = dxf::Color::from_index(index);
                        layer.color_24_bit = true_color.unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
    }

    /// Records the names of a table object whose entries [`Names::add`] already added, giving the same names.
    fn find(&mut self, drawing: &dxf::Drawing, object: &Object) {
        match object {
            Object::Layers {
                unknown1, layers, ..
            } => {
                let markers_on = point_markers_on(unknown1);
                self.layers
                    .extend(layers.iter().map(|layer| layer.name.clone()));
                self.layer_colors
                    .extend(layers.iter().map(|layer| layer.color));
                self.point_markers.extend(
                    layers
                        .iter()
                        .map(|layer| markers_on && layer.flags & LAYER_POINT_MARKERS != 0),
                );
            }
            Object::LineTypes { types } => {
                self.line_types.extend(types.iter().map(|line_type| {
                    dxf_line_type(line_type).and_then(|l| find_line_type(drawing, &l.name))
                }));
                self.line_patterns
                    .extend(types.iter().map(LinePattern::new));
            }
            Object::Fonts { names } => {
                self.text_styles.extend(
                    names
                        .iter()
                        .map(|font| text_style(font).and_then(|s| find_style(drawing, &s.name))),
                );
            }
            Object::Libraries { names } => self.set_libraries(names),
            Object::Scale { scale } => self.scale = Some(*scale),
            Object::Metadata {
                content:
                    MetadataContent::Projection {
                        projection,
                        datum,
                        central_meridian,
                        ..
                    },
                ..
            } => self.projection = TransverseMercator::from_mproj(*projection, *datum, *central_meridian),
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
    let mut extras = match geometry {
        Geometry::Line { begin, end, .. } => {
            line_pattern_entities(names, usize::from(geometry.header().line_type), begin, end)
        }
        _ => vec![],
    };
    if let (Some(marker), Geometry::Point { position, .. }) =
        (point_marker(names, &geometry), &geometry)
    {
        let scale = names.pattern_scale();
        let insert = dxf::entities::Insert {
            name: marker.block_name().to_owned(),
            location: (*position).into(),
            x_scale_factor: scale,
            y_scale_factor: scale,
            ..Default::default()
        };
        // Markers are drawn solid.
        extras.push((
            dxf::entities::EntityType::Insert(insert),
            Some("CONTINUOUS".to_owned()),
        ));
    }
    let common = |mut entity: dxf::entities::Entity| {
        if let Some(layer) = &layer {
            entity.common.layer = layer.clone();
        }
        // Zero means the geometry takes the color of its layer, which is the DXF default.
        if color != 0 {
            let (index, true_color) = names.color(color);
            entity.common.color = dxf::Color::from_index(index);
            entity.common.color_24_bit = true_color.unwrap_or(0);
        }
        entity
    };

    let mut entities: Vec<dxf::entities::Entity> = entity_types(drawing, names, geometry)
        .into_iter()
        .map(|mut entity| {
            if let (dxf::entities::EntityType::Text(text), Some(text_style)) =
                (&mut entity.specific, &text_style)
            {
                text.text_style_name = text_style.clone();
            }
            entity.common.x_data.extend(x_data.clone());
            if let Some(line_type) = &line_type {
                entity.common.line_type_name = line_type.clone();
            }
            common(entity)
        })
        .collect();

    entities.extend(extras.into_iter().map(|(entity_type, line_type)| {
        let mut entity = dxf::entities::Entity::new(entity_type);
        if let Some(line_type) = line_type {
            entity.common.line_type_name = line_type;
        }
        common(entity)
    }));
    entities
}

/// A 2D box around points, see [`Extents`].
///
/// The box is 2D, as NetCAD writes it. Elevations are left out since some polylines store garbage there (values up
/// to 1e37 in `ALINCA_ITRF`, which NetCAD exports too), and a box that tall would make a zoom to the extents show
/// nothing.
#[derive(Debug, Default, PartialEq)]
struct Bounds(Option<(dxf::Point, dxf::Point)>);

impl Bounds {
    fn add(&mut self, point: &dxf::Point) {
        if !(point.x.is_finite() && point.y.is_finite()) {
            return;
        }
        let point = dxf::Point::new(point.x, point.y, 0.0);
        let (min, max) = self.0.get_or_insert_with(|| (point.clone(), point.clone()));
        min.x = min.x.min(point.x);
        min.y = min.y.min(point.y);
        max.x = max.x.max(point.x);
        max.y = max.y.max(point.y);
    }

    /// Adds a circle as its bounding square, which also holds any arc of it.
    fn add_circle(&mut self, center: &dxf::Point, radius: f64) {
        let radius = radius.abs();
        for (dx, dy) in [(-radius, -radius), (radius, radius)] {
            self.add(&dxf::Point::new(center.x + dx, center.y + dy, center.z));
        }
    }

    /// Adds the box of an arc: its ends, and the extreme points of its circle that its sweep passes. Angles are in
    /// degrees, counter-clockwise from the start to the end, as in DXF.
    fn add_arc(&mut self, center: &dxf::Point, radius: f64, start: f64, end: f64) {
        let radius = radius.abs();
        let at = |degrees: f64| {
            let radians = degrees.to_radians();
            dxf::Point::new(
                center.x + radius * radians.cos(),
                center.y + radius * radians.sin(),
                center.z,
            )
        };
        if !(start.is_finite() && end.is_finite()) {
            return self.add_circle(center, radius);
        }
        let start = start.rem_euclid(360.0);
        let mut sweep = (end - start).rem_euclid(360.0);
        if sweep == 0.0 {
            sweep = 360.0;
        }
        self.add(&at(start));
        self.add(&at(start + sweep));
        for quadrant in [0.0, 90.0, 180.0, 270.0, 360.0, 450.0, 540.0, 630.0] {
            if quadrant > start && quadrant < start + sweep {
                self.add(&at(quadrant));
            }
        }
    }

    /// Adds the box of a text: NetCAD's width (see [`text_width`]) and the height, placed by the alignment point and
    /// justification as CAD programs place it, turned with the text.
    fn add_text(&mut self, text: &dxf::entities::Text) {
        use dxf::enums::HorizontalTextJustification as H;
        use dxf::enums::VerticalTextJustification as V;

        let height = text.text_height.abs();
        let value = text.value.strip_prefix("%%u").unwrap_or(&text.value);
        let width = text_width(Some(&text.text_style_name), value, 0.0) * height;
        let justified = (text.horizontal_text_justification, text.vertical_text_justification)
            != (H::Left, V::Baseline);
        // The corner the text starts at (left end of the baseline), in the text's frame relative to `origin`.
        let origin = if justified {
            &text.second_alignment_point
        } else {
            &text.location
        };
        let along = match text.horizontal_text_justification {
            H::Center | H::Middle => -width / 2.0,
            H::Right => -width,
            _ => 0.0,
        };
        let across = match text.vertical_text_justification {
            V::Middle => -height / 2.0,
            V::Top => -height,
            _ => 0.0,
        };
        let (sin, cos) = text.rotation.to_radians().sin_cos();
        for (dx, dy) in [(0.0, 0.0), (width, 0.0), (0.0, height), (width, height)] {
            let (x, y) = (along + dx, across + dy);
            self.add(&dxf::Point::new(
                origin.x + x * cos - y * sin,
                origin.y + x * sin + y * cos,
                0.0,
            ));
        }
    }

    /// Adds the points that define an entity. Inserts count with their insertion point only, so the box can miss the
    /// far side of a block's shape.
    fn add_entity(&mut self, entity: &dxf::entities::Entity) {
        use dxf::entities::EntityType;

        match &entity.specific {
            EntityType::Arc(arc) => {
                self.add_arc(&arc.center, arc.radius, arc.start_angle, arc.end_angle)
            }
            EntityType::Circle(circle) => self.add_circle(&circle.center, circle.radius),
            EntityType::Face3D(face) => {
                for point in [
                    &face.first_corner,
                    &face.second_corner,
                    &face.third_corner,
                    &face.fourth_corner,
                ] {
                    self.add(point);
                }
            }
            EntityType::Insert(insert) => self.add(&insert.location),
            EntityType::Line(line) => {
                self.add(&line.p1);
                self.add(&line.p2);
            }
            EntityType::ModelPoint(point) => self.add(&point.location),
            EntityType::Polyline(polyline) => {
                for vertex in polyline.vertices() {
                    self.add(&vertex.location);
                }
            }
            EntityType::LwPolyline(polyline) => {
                for vertex in &polyline.vertices {
                    self.add(&dxf::Point::new(vertex.x, vertex.y, 0.0));
                }
            }
            EntityType::Text(text) => self.add_text(text),
            _ => {}
        }
    }

    fn union(&mut self, other: &Bounds) {
        if let Some((min, max)) = &other.0 {
            self.add(min);
            self.add(max);
        }
    }
}

/// Size of the grid cells that [`Extents`] counts entities in, in drawing units.
const EXTENTS_CELL: f64 = 1000.0;
/// A gap in the distances from the drawing's centre that starts stray geometry must be at least this long…
const STRAY_GAP: f64 = 20_000.0;
/// …and at least this many times the distance before it…
const STRAY_GAP_RATIO: f64 = 2.0;
/// …and what lies beyond it at most this share of the entities.
const STRAY_SHARE: f64 = 0.01;

/// The box around the entities of the model space, for the extents and limits in the header and the opening view. The
/// extents an NCZ file stores are often stale: in 3 of the 6 sample files they cover only part of the drawing, so
/// viewers that zoom to the header extents showed little or nothing.
///
/// A few files hold a few stray entities far from the rest, such as 6 near (0, 0) in `436-5,6,7(yeşil)(imar)`, whose
/// drawing lies around (106,000, 106,000). They stay in the drawing but are left out of the box, which would otherwise
/// be mostly empty. Entities count by the centre of their box, in a grid. Walking out from the weighted median cell,
/// the box stops at the first gap of at least 20 km that also doubles the distance, provided at most 1% of the entities
/// lie beyond it. Real data spreads continuously, even over 160 km in multi-site files, and a real second site holds
/// more than 1%, so only clearly stray geometry is left out.
#[derive(Debug, Default)]
struct Extents {
    /// Entity count and box by grid cell.
    cells: BTreeMap<(i64, i64), (usize, Bounds)>,
    /// The inserts, by grid cell, which count with their insertion point until [`Extents::add_blocks`] adds the box
    /// of their block.
    inserts: Vec<((i64, i64), dxf::entities::Insert)>,
}

impl Extents {
    fn add_entity(&mut self, entity: &dxf::entities::Entity) {
        let mut bounds = Bounds::default();
        bounds.add_entity(entity);
        let Some((min, max)) = &bounds.0 else {
            return;
        };
        let cell = |a: f64, b: f64| ((a + b) / 2.0 / EXTENTS_CELL).floor() as i64;
        let key = (cell(min.x, max.x), cell(min.y, max.y));
        let (count, cell_bounds) = self.cells.entry(key).or_default();
        *count += 1;
        cell_bounds.union(&bounds);
        if let dxf::entities::EntityType::Insert(insert) = &entity.specific {
            self.inserts.push((key, insert.clone()));
        }
    }

    /// Adds the box of each insert's block, scaled, turned and moved like the insert, to the insert's cell. Inserts
    /// inside blocks count with their insertion point only.
    fn add_blocks(&mut self, drawing: &dxf::Drawing) {
        let mut block_bounds: BTreeMap<&str, (f64, f64, f64, f64)> = BTreeMap::new();
        for block in drawing.blocks() {
            let mut bounds = Bounds::default();
            for entity in &block.entities {
                bounds.add_entity(entity);
            }
            if let Some((min, max)) = bounds.0 {
                let base = &block.base_point;
                block_bounds.insert(&block.name, (min.x - base.x, min.y - base.y, max.x - base.x, max.y - base.y));
            }
        }
        for (key, insert) in std::mem::take(&mut self.inserts) {
            let Some(&(x0, y0, x1, y1)) = block_bounds.get(insert.name.as_str()) else {
                continue;
            };
            let (sin, cos) = insert.rotation.to_radians().sin_cos();
            let (sx, sy) = (insert.x_scale_factor, insert.y_scale_factor);
            let Some((_, cell_bounds)) = self.cells.get_mut(&key) else {
                continue;
            };
            for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                let (x, y) = (x * sx, y * sy);
                cell_bounds.add(&dxf::Point::new(
                    insert.location.x + x * cos - y * sin,
                    insert.location.y + x * sin + y * cos,
                    0.0,
                ));
            }
        }
    }

    /// The box, and how many entities were left out of it.
    fn bounds(&self) -> (Bounds, usize) {
        let total: usize = self.cells.values().map(|(count, _)| count).sum();
        if total == 0 {
            return (Bounds::default(), 0);
        }
        let median = |axis: fn(&(i64, i64)) -> i64| {
            let mut keys: Vec<(i64, usize)> = self
                .cells
                .iter()
                .map(|(key, (count, _))| (axis(key), *count))
                .collect();
            keys.sort_unstable();
            let mut seen = 0;
            for (value, count) in keys {
                seen += count;
                if seen * 2 >= total {
                    return (value as f64 + 0.5) * EXTENTS_CELL;
                }
            }
            unreachable!("the counts add up to the total")
        };
        let center = (median(|key| key.0), median(|key| key.1));

        let mut cells: Vec<(f64, usize, &Bounds)> = self
            .cells
            .iter()
            .map(|(&(x, y), (count, bounds))| {
                let (cx, cy) = ((x as f64 + 0.5) * EXTENTS_CELL, (y as f64 + 0.5) * EXTENTS_CELL);
                ((cx - center.0).hypot(cy - center.1), *count, bounds)
            })
            .collect();
        cells.sort_by(|a, b| a.0.total_cmp(&b.0));

        let mut kept = cells.len();
        let mut beyond = total;
        for index in 0..cells.len() {
            if index > 0 {
                let (distance, previous) = (cells[index].0, cells[index - 1].0.max(EXTENTS_CELL));
                if distance >= STRAY_GAP
                    && distance >= STRAY_GAP_RATIO * previous
                    && beyond as f64 <= STRAY_SHARE * total as f64
                {
                    kept = index;
                    break;
                }
            }
            beyond -= cells[index].1;
        }

        let mut bounds = Bounds::default();
        for (_, _, cell_bounds) in &cells[..kept] {
            bounds.union(cell_bounds);
        }
        let left_out = cells[kept..].iter().map(|(_, count, _)| count).sum();
        (bounds, left_out)
    }
}

/// What the DXF output holds, for the summary.
#[derive(Debug, Default)]
pub struct DxfContents {
    /// Number of entities of each DXF type, in the model space and in blocks.
    pub entities: BTreeMap<&'static str, usize>,
    pub layers: usize,
    pub line_types: usize,
    pub blocks: usize,
    /// Symbols whose shape is not known, drawn as a rectangle, by symbol (e.g. `F3 139`): the number of geometries.
    pub unknown_symbols: BTreeMap<String, usize>,
    /// Symbols of line type patterns whose shape is not known, left out, by symbol: the number of lines.
    pub dropped_pattern_symbols: BTreeMap<String, usize>,
}

/// A symbol as the summary names it, e.g. `F3 139`, or `139` when its library is not known.
fn symbol_label(library: Option<&str>, symbol: u16) -> String {
    match library {
        Some(library) => format!("{library} {symbol}"),
        None => symbol.to_string(),
    }
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
    // R12 has neither extended data nor a thumbnail, and true colours (group 420) need R2004.
    drawing.header.version = dxf::enums::AcadVersion::R2004;
    // Text before R2007 is in the code page `$DWGCODEPAGE` names. Turkish Windows' code page holds the Turkish letters,
    // which NCZ files store in it too. Without it, names and texts carried `\U+` escapes (`HAT_B\U+0130NA`), which some
    // programs show as they are.
    drawing.header.drawing_code_page = "ANSI_1254".to_owned();
    // NCZ coordinates are metres in a national or local grid.
    drawing.header.default_drawing_units = dxf::enums::Units::Meters;
    drawing.header.drawing_units = dxf::enums::DrawingUnits::Metric;
    drawing.text_encoding = Some(encoding_rs::WINDOWS_1254);
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
    // The symbols in use by block name, with their library, number, and footprint measured on the instance closest to
    // a right angle.
    let mut symbols_in_use: BTreeMap<String, (Option<String>, u16, Option<SymbolFootprint>)> =
        BTreeMap::new();
    let mut point_markers: BTreeSet<PointMarker> = BTreeSet::new();
    // Block entities with their own colour, by block name, entity range and palette number. They are coloured again
    // once the whole palette is known.
    let mut block_colors: BTreeMap<String, Vec<(std::ops::Range<usize>, u8)>> = BTreeMap::new();
    let mut reserved_handles = 0;
    let mut extents = Extents::default();
    // The union of the bounds stored in the headers of the model space geometries, which NetCAD 5 writes as the
    // limits (all 242 corpus files checked): it holds geometry that is not drawn, such as symbols with scale 0, and
    // some files store an arc's whole circle.
    let mut stored_bounds = Bounds::default();
    for item in ObjectReader::new(bytes)? {
        inspect(&item);
        let Ok(object) = item else { continue };
        names.add(&mut drawing, &object);
        match object {
            Object::Extents {
                lower_bound,
                upper_bound,
            } => {
                // Only used when the model space is empty, see `Bounds`.
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
                if block.is_none() && !matches!(geometry, Geometry::Empty { .. } | Geometry::Other { .. }) {
                    let header = geometry.header();
                    let (lower, upper): (dxf::Point, dxf::Point) =
                        (header.lower_bound.into(), header.upper_bound.into());
                    // Empty bounds are stored inverted, and unset ones as ±1e30.
                    let sane = |point: &dxf::Point| point.x.abs() < 1e20 && point.y.abs() < 1e20;
                    if lower.x <= upper.x && lower.y <= upper.y && sane(&lower) && sane(&upper) {
                        stored_bounds.add(&lower);
                        stored_bounds.add(&upper);
                    }
                }
                if let Geometry::Symbol {
                    position,
                    scale,
                    rotation,
                    symbol,
                    library,
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
                    let library = names.library(*library);
                    if library.is_none_or(|library| symbols::find(library, *symbol).is_none()) {
                        *contents
                            .unknown_symbols
                            .entry(symbol_label(library, *symbol))
                            .or_default() += 1;
                    }
                    let (_, _, footprint) = symbols_in_use
                        .entry(symbol_block_name(library, *symbol))
                        .or_insert_with(|| (library.map(str::to_owned), *symbol, None));
                    if let Some(measured) = measured
                        && footprint.is_none_or(|f| measured.deviation < f.deviation)
                    {
                        *footprint = Some(measured);
                    }
                }

                point_markers.extend(point_marker(&names, &geometry));

                if let Geometry::Line { header, .. } = &geometry {
                    let library = names.library(0);
                    let chain = line_type_chain(&names, usize::from(header.line_type));
                    let symbols: BTreeSet<u16> = chain
                        .iter()
                        .filter_map(|&(_, index)| names.line_pattern(index))
                        .flat_map(LinePattern::symbols)
                        .collect();
                    for symbol in symbols {
                        match library {
                            Some(library) if symbols::find(library, symbol).is_some() => {
                                symbols_in_use
                                    .entry(symbol_block_name(Some(library), symbol))
                                    .or_insert_with(|| (Some(library.to_owned()), symbol, None));
                            }
                            _ => {
                                *contents
                                    .dropped_pattern_symbols
                                    .entry(symbol_label(library, symbol))
                                    .or_default() += 1;
                            }
                        }
                    }
                }

                match &mut block {
                    Some(block) => {
                        let number = geometry.header().color;
                        let first = block.entities.len();
                        for entity in geometry_entities(&mut drawing, &names, geometry) {
                            count(&entity);
                            block.entities.push(entity);
                        }
                        if number != 0 {
                            block_colors
                                .entry(block.name.clone())
                                .or_default()
                                .push((first..block.entities.len(), number));
                        }
                    }
                    None => {
                        for entity in geometry_entities(&mut scratch, &names, geometry) {
                            count(&entity);
                            extents.add_entity(&entity);
                            drawing.prepare_entity(&entity);
                            reserved_handles += handles_needed(&entity);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    for block in drawing.blocks_mut() {
        for (range, number) in block_colors.get(&block.name).into_iter().flatten() {
            let (index, true_color) = names.color(*number);
            for entity in &mut block.entities[range.clone()] {
                entity.common.color = dxf::Color::from_index(index);
                entity.common.color_24_bit = true_color.unwrap_or(0);
            }
        }
    }

    for marker in point_markers {
        let block = symbol_shape_block(marker.block_name().to_owned(), marker.shapes());
        block.entities.iter().for_each(&mut count);
        drawing.add_block(block);
    }
    for (name, (library, symbol, footprint)) in symbols_in_use {
        let shapes = library.and_then(|library| symbols::find(&library, symbol));
        let block = match shapes {
            Some(shapes) => symbol_shape_block(name, shapes),
            None => symbol_footprint_block(&mut drawing, name, footprint),
        };
        block.entities.iter().for_each(&mut count);
        drawing.add_block(block);
    }

    // Inserts count with the box of their block, which is only known now that every block exists.
    extents.add_blocks(&drawing);
    let (bounds, _) = extents.bounds();
    // Without stored bounds (no geometry stores any), the limits are the extents.
    let limits = if stored_bounds.0.is_some() { &stored_bounds } else { &bounds };
    if let Bounds(Some((min, max))) = limits {
        // The limits as NetCAD writes them (2D), see `stored_bounds`; the extents below are what is drawn.
        drawing.header.minimum_drawing_limits = min.clone();
        drawing.header.maximum_drawing_limits = max.clone();
    }
    if let Bounds(Some((min, max))) = bounds {
        // The view a reader opens with, for readers that do not zoom to the extents: the bounds with a margin, tall
        // enough for a window of 3:2.
        let (width, height) = (max.x - min.x, max.y - min.y);
        let center = dxf::Point::new((min.x + max.x) / 2.0, (min.y + max.y) / 2.0, 0.0);
        let view_height = (height.max(width / 1.5) * 1.05).max(1.0);
        for view_port in drawing.view_ports_mut().filter(|v| v.name.eq_ignore_ascii_case("*ACTIVE")) {
            view_port.view_center = center.clone();
            view_port.view_height = view_height;
            view_port.view_port_aspect_ratio = 1.5;
        }
        drawing.header.minimum_drawing_extents = min;
        drawing.header.maximum_drawing_extents = max;
    }

    contents.layers = drawing.layers().count();
    contents.line_types = drawing.line_types().count();
    contents.blocks = drawing.blocks().count();

    // Second pass: the entities of the model space.
    let mut stream = drawing.save_streaming(writer, reserved_handles)?;
    let mut names = Names {
        palette: names.palette,
        ..Names::default()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn line_type(segment_count: i8, offset: f32, unknown1: u16, segments: &[f32]) -> LineType {
        LineType {
            description: String::new(),
            name: "TEST".to_owned(),
            segment_count,
            pattern_length: 0.0,
            offset,
            unknown1,
            segments: segments.to_vec(),
            unknown2: vec![],
        }
    }

    #[test]
    fn extents_leave_out_stray_geometry() {
        use dxf::entities::{Entity, EntityType, ModelPoint};
        use dxf::Point;

        let point = |x: f64, y: f64| Entity::new(EntityType::ModelPoint(ModelPoint::new(Point::new(x, y, 0.0))));
        let mut extents = Extents::default();
        // A drawing of 200 points spread over 50 km, and 1 point 150 km away.
        for i in 0..200 {
            extents.add_entity(&point(106_000.0 + f64::from(i) * 250.0, 106_000.0));
        }
        extents.add_entity(&point(0.0, 0.0));
        let (bounds, left_out) = extents.bounds();
        assert_eq!(left_out, 1);
        assert_eq!(
            bounds,
            Bounds(Some((Point::new(106_000.0, 106_000.0, 0.0), Point::new(155_750.0, 106_000.0, 0.0))))
        );

        // A second site of more than 1% stays in.
        for i in 0..10 {
            extents.add_entity(&point(f64::from(i), 0.0));
        }
        assert_eq!(extents.bounds().1, 0);
    }

    #[test]
    fn cleans_elevations() {
        assert_eq!(elevation(788.533), 788.533);
        assert_eq!(elevation(-12.5), -12.5);
        assert_eq!(elevation(5137.0), 5137.0);
        assert_eq!(elevation(6750.208), 0.0);
        assert_eq!(elevation(-1.2e34), 0.0);
        assert_eq!(elevation(f64::NAN), 0.0);
        assert_eq!(elevation(3.19e-27), 0.0);
    }

    #[test]
    fn measures_text_like_netcad() {
        // `ALİ OĞLU` is 161/24 heights wide in ARIAL.NCF.
        assert!((text_width(Some("ARIAL"), "ALİ OĞLU", 0.0) - 161.0 / 24.0).abs() < 1e-12);
        assert!((text_width(None, "26", 1.0) - 38.0 / 24.0).abs() < 1e-12);
        assert!((text_width(Some("F1"), "11", 0.0) - 42.0 / 24.0).abs() < 1e-12);
        // Spacing scales the advance; unknown characters take the default.
        assert!((text_width(Some("ARIAL"), "\u{2603}", 1.2) - 19.0 / 24.0 * 1.2).abs() < 1e-6);
    }

    #[test]
    fn places_text_at_netcads_alignment_point() {
        // abakolu_itrf96_20112009_kesin: `ALİ OĞLU`, snap `M`, height 3, rotation 323.13°, stored at
        // (509327.391, 4581553.912); NetCAD 5 writes group 11 (509336.341, 4581549.074).
        let [a, b, c, d, e] = text_offsets('M');
        let (height, tilt) = (3.0, 323.13_f64.to_radians());
        let width = text_width(Some("ARIAL"), "ALİ OĞLU", 0.0) * height;
        let (along, across) = (a * width + b * height, c * height);
        let x = 509327.391 + along * tilt.cos() - across * tilt.sin();
        let y = 4581553.912 + along * tilt.sin() + across * tilt.cos();
        assert!((x - 509336.341).abs() < 0.01 && (y - 4581549.074).abs() < 0.01, "{x} {y}");
        assert_eq!((d, e), (0.0, 0.0));
        // `U` puts the text above the point, with group 10 below it, as NetCAD does.
        assert_eq!(text_offsets('U'), [0.5, 0.0, 1.5, 0.0, -1.5]);
        assert_eq!(text_offsets('L'), [0.0; 5]);
    }

    #[test]
    fn maps_text_alignments() {
        use dxf::enums::HorizontalTextJustification as H;
        use dxf::enums::VerticalTextJustification as V;
        // As NetCAD 5 exports them.
        assert_eq!(text_justification('M'), (H::Center, V::Middle));
        assert_eq!(text_justification('P'), (H::Left, V::Middle));
        assert_eq!(text_justification('Q'), (H::Right, V::Middle));
        assert_eq!(text_justification('U'), (H::Center, V::Top));
        assert_eq!(text_justification('8'), (H::Right, V::Baseline));
        assert_eq!(text_justification('L'), (H::Left, V::Baseline));
        assert_eq!(text_justification('\0'), (H::Left, V::Baseline));
    }

    #[test]
    fn picks_point_markers() {
        // As NetCAD 5 drew them in the corpus.
        assert!(point_markers_on(&[0, 0, 0, 0, 1, 1, 0, 1, 0]));
        assert!(!point_markers_on(&[0, 0, 0, 0, 1, 0, 0, 1, 0]));
        assert!(!point_markers_on(&[]));
        assert_eq!(PointMarker::of("118/1", true), Some(PointMarker::Circle));
        assert_eq!(PointMarker::of("P.35", true), Some(PointMarker::Traverse));
        assert_eq!(
            PointMarker::of("N.F2930181", true),
            Some(PointMarker::Triangulation)
        );
        assert_eq!(PointMarker::of("P.20", false), Some(PointMarker::Traverse));
        assert_eq!(
            PointMarker::of("N.F2930203", false),
            Some(PointMarker::Triangulation)
        );
        assert_eq!(PointMarker::of("118/535", false), None);
        assert_eq!(PointMarker::of("", false), None);
    }

    #[test]
    fn extents_cover_the_blocks_of_inserts() {
        use dxf::entities::{Entity, EntityType, Insert, Line};
        use dxf::Point;

        let mut drawing = dxf::Drawing::new();
        drawing.add_block(dxf::Block {
            name: "B".to_owned(),
            entities: vec![Entity::new(EntityType::Line(Line::new(
                Point::new(0.0, 0.0, 0.0),
                Point::new(10.0, 5.0, 0.0),
            )))],
            ..Default::default()
        });
        // Turned a quarter, at twice the size: the block's (10, 5) lands at (100 - 10, 100 + 20).
        let insert = Insert {
            name: "B".to_owned(),
            location: Point::new(100.0, 100.0, 0.0),
            x_scale_factor: 2.0,
            y_scale_factor: 2.0,
            rotation: 90.0,
            ..Default::default()
        };
        let mut extents = Extents::default();
        extents.add_entity(&Entity::new(EntityType::Insert(insert)));
        extents.add_blocks(&drawing);
        let (Bounds(Some((min, max))), left_out) = extents.bounds() else {
            panic!("no bounds");
        };
        assert_eq!(left_out, 0);
        assert!((min.x - 90.0).abs() < 1e-9 && (max.x - 100.0).abs() < 1e-9, "{min:?} {max:?}");
        assert!((min.y - 100.0).abs() < 1e-9 && (max.y - 120.0).abs() < 1e-9, "{min:?} {max:?}");
    }

    #[test]
    fn bounds_cover_the_text_box() {
        use dxf::entities::{Entity, EntityType, Text};
        use dxf::enums::{HorizontalTextJustification as H, VerticalTextJustification as V};
        use dxf::Point;

        // `11` in ARIAL is 38/24 heights wide; centred-middle at (100, 50), height 2.4, unturned.
        let text = Text {
            value: "11".to_owned(),
            text_height: 2.4,
            text_style_name: "ARIAL".to_owned(),
            location: Point::new(98.0, 49.0, 0.0),
            second_alignment_point: Point::new(100.0, 50.0, 0.0),
            horizontal_text_justification: H::Center,
            vertical_text_justification: V::Middle,
            ..Default::default()
        };
        let mut bounds = Bounds::default();
        bounds.add_entity(&Entity::new(EntityType::Text(text)));
        let (min, max) = bounds.0.unwrap();
        let half = 38.0 / 24.0 * 2.4 / 2.0;
        assert!((min.x - (100.0 - half)).abs() < 1e-9 && (max.x - (100.0 + half)).abs() < 1e-9);
        assert!((min.y - 48.8).abs() < 1e-9 && (max.y - 51.2).abs() < 1e-9);
    }

    #[test]
    fn bounds_cover_only_the_arc() {
        use dxf::entities::{Arc, Entity, EntityType};
        use dxf::Point;

        // A quarter arc from 0° to 90° around the origin, radius 10: only the first quadrant.
        let mut bounds = Bounds::default();
        bounds.add_entity(&Entity::new(EntityType::Arc(Arc::new(
            Point::new(0.0, 0.0, 0.0),
            10.0,
            0.0,
            90.0,
        ))));
        let (min, max) = bounds.0.unwrap();
        assert!(min.x.abs() < 1e-9 && min.y.abs() < 1e-9);
        assert!((max.x - 10.0).abs() < 1e-9 && (max.y - 10.0).abs() < 1e-9);
        // From 350° to 10° it passes 0°, but not 90°, 180° or 270°.
        let mut bounds = Bounds::default();
        bounds.add_entity(&Entity::new(EntityType::Arc(Arc::new(
            Point::new(0.0, 0.0, 0.0),
            10.0,
            350.0,
            10.0,
        ))));
        let (min, max) = bounds.0.unwrap();
        assert!((max.x - 10.0).abs() < 1e-9);
        assert!((min.x - 10.0 * 10_f64.to_radians().cos()).abs() < 1e-9);
        assert!((max.y - 10.0 * 10_f64.to_radians().sin()).abs() < 1e-9);
    }

    #[test]
    fn bounds_cover_entities() {
        use dxf::entities::{Circle, Entity, EntityType, Line};
        use dxf::Point;

        let mut bounds = Bounds::default();
        bounds.add_entity(&Entity::new(EntityType::Line(Line::new(
            Point::new(10.0, 20.0, 0.0),
            Point::new(30.0, 5.0, 2.0),
        ))));
        bounds.add_entity(&Entity::new(EntityType::Circle(Circle::new(
            Point::new(0.0, 0.0, 0.0),
            1.5,
        ))));
        bounds.add(&Point::new(f64::NAN, 100.0, 0.0));
        assert_eq!(
            bounds,
            Bounds(Some((
                Point::new(-1.5, -1.5, 0.0),
                Point::new(30.0, 20.0, 0.0)
            )))
        );
    }

    #[test]
    fn places_symbols_along_a_line() {
        // ÇİT-1 as NetCAD exports it: a symbol every 10 units, the first at 5.
        let pattern = LinePattern::new(&line_type(3, 0.0, 0, &[5.0, 1144.0, 5.0])).unwrap();
        assert_eq!(pattern.parallel, None);
        assert_eq!(
            pattern.symbol_places(1.0, 17.285),
            [(5.0, 144, 1.0, false), (15.0, 144, 1.0, false)]
        );
        // The pattern scales with the map scale.
        assert_eq!(pattern.symbol_places(2.0, 17.285), [(10.0, 144, 1.0, false)]);
    }

    #[test]
    fn keeps_symbols_at_the_ends() {
        // ISTINAT: a symbol at the start and every 7 units.
        let pattern = LinePattern::new(&line_type(-2, 1.0, 64, &[1140.0, 7.0])).unwrap();
        assert_eq!(pattern.parallel, Some((1.0, 64)));
        assert_eq!(
            pattern.symbol_places(1.0, 14.0),
            [
                (0.0, 140, 1.0, false),
                (7.0, 140, 1.0, false),
                (14.0, 140, 1.0, false)
            ]
        );
    }

    #[test]
    fn turns_negative_symbols() {
        let pattern = LinePattern::new(&line_type(
            -5,
            -1.0,
            107,
            &[3.5, -1140.0, 7.0, -1140.0, 3.5],
        ))
        .unwrap();
        assert_eq!(
            pattern.symbol_places(1.0, 15.0),
            [(3.5, 140, 1.0, true), (10.5, 140, 1.0, true)]
        );
    }

    #[test]
    fn sizes_symbols_by_the_thousands_digit() {
        // ŞEV-1 as NetCAD exports it: symbol 128 at twice and at four times its size, turned.
        let pattern =
            LinePattern::new(&line_type(5, 0.0, 0, &[2.0, -2128.0, 2.5, -4128.0, 0.5])).unwrap();
        assert_eq!(
            pattern.symbol_places(1.0, 5.0),
            [(2.0, 128, 2.0, true), (4.5, 128, 4.0, true)]
        );
        // Symbol numbers beyond a library's 256 are left out.
        assert_eq!(
            LinePattern::new(&line_type(3, 0.0, 0, &[2.0, 1300.0, 2.0])),
            None
        );
    }

    #[test]
    fn leaves_out_unknown_symbols_and_plain_patterns() {
        assert_eq!(LinePattern::new(&line_type(2, 0.0, 0, &[2.0, -1.0])), None);
        // Symbols without any length would all be at the start.
        assert_eq!(LinePattern::new(&line_type(1, 0.0, 0, &[1140.0])), None);
    }

    #[test]
    fn follows_parallel_line_types() {
        // DUVAR-1 → DUV2 → DUV3 → DUV1 → DUZ, as in the sample files.
        let mut names = Names::default();
        let mut types: Vec<LineType> = (0..110).map(|_| line_type(1, 0.0, 0, &[500.0])).collect();
        types[10] = line_type(-2, 0.1, 108, &[400.0, 100.0]);
        types[107] = line_type(-2, 1.0, 0, &[400.0, 100.0]);
        types[108] = line_type(-2, 0.8, 109, &[400.0, 100.0]);
        types[109] = line_type(-5, -1.0, 107, &[3.5, -1140.0, 7.0, -1140.0, 3.5]);
        names.line_types = vec![None; types.len()];
        names.line_patterns = types.iter().map(LinePattern::new).collect();

        let chain: Vec<(f64, usize)> = line_type_chain(&names, 10)
            .into_iter()
            .map(|(shift, index)| ((shift * 1e6_f64).round() / 1e6, index))
            .collect();
        assert_eq!(
            chain,
            [(0.0, 10), (-0.1, 108), (-0.9, 109), (0.1, 107), (-0.9, 0)]
        );

        // A loop is cut.
        names.line_patterns[0] = LinePattern::new(&line_type(-1, 1.0, 0, &[500.0]));
        assert_eq!(line_type_chain(&names, 0).len(), MAX_CHAIN);
    }
}
