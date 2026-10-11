//! Pairs NCZ geometries with the DXF entities that NetCAD wrote for them, by their coordinates.

use crate::dxf::Entity;
use crate::features::NczItem;
use std::collections::HashMap;

/// Candidates looked at per geometry, so that a vertex shared by many polylines stays cheap.
const MAX_CANDIDATES: usize = 64;

pub struct Match {
    pub item: usize,
    pub entity: usize,
    pub score: f64,
    /// Another entity written for a geometry that already has its main counterpart, e.g. a hatch fill.
    pub secondary: bool,
}

/// Score of an entity that has the same points as the geometry, which a secondary match needs.
const SECONDARY_SCORE: f64 = 2.0;

pub struct Matching {
    /// Whether the NCZ X and Y had to be swapped to meet the DXF coordinates.
    pub swapped: bool,
    pub matches: Vec<Match>,
    /// Entities that are not the counterpart of any geometry.
    pub unmatched_entities: Vec<usize>,
    /// Geometries that have points but no counterpart.
    pub unmatched_items: Vec<usize>,
}

/// A spatial hash of points with a cell the size of the tolerance.
struct Grid {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(tolerance: f64) -> Self {
        Self {
            cell: tolerance,
            cells: HashMap::new(),
        }
    }

    fn key(&self, (x, y): (f64, f64)) -> (i64, i64) {
        (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        )
    }

    fn insert(&mut self, point: (f64, f64), value: usize) {
        let key = self.key(point);
        self.cells.entry(key).or_default().push(value);
    }

    /// Values stored at points in the cells around `point`, which includes all within the tolerance.
    fn near(&self, point: (f64, f64)) -> impl Iterator<Item = usize> + '_ {
        let (kx, ky) = self.key(point);
        (-1..=1)
            .flat_map(move |dx| (-1..=1).map(move |dy| (kx + dx, ky + dy)))
            .filter_map(|key| self.cells.get(&key))
            .flatten()
            .copied()
    }
}

fn is_near(a: (f64, f64), b: (f64, f64), tolerance: f64) -> bool {
    (a.0 - b.0).abs() <= tolerance && (a.1 - b.1).abs() <= tolerance
}

/// Share of the points in `from` that have a point of `to` within the tolerance.
fn coverage(from: &[(f64, f64)], to: &[(f64, f64)], tolerance: f64) -> f64 {
    if from.is_empty() {
        return 0.0;
    }

    let mut grid = Grid::new(tolerance);
    for (i, point) in to.iter().enumerate() {
        grid.insert(*point, i);
    }
    let covered = from
        .iter()
        .filter(|p| grid.near(**p).any(|i| is_near(**p, to[i], tolerance)))
        .count();

    covered as f64 / from.len() as f64
}

/// The DXF entity types NetCAD is expected to write for each kind of geometry.
fn expected_kinds(kind: &str) -> &'static [&'static str] {
    match kind {
        "Point" => &["POINT", "INSERT"],
        "Line" => &["LINE"],
        "Circle" => &["CIRCLE"],
        "Arc" => &["ARC"],
        "Text" => &["TEXT", "MTEXT"],
        "Polyline" | "Contour" | "Window" | "Map sheet" => &["LWPOLYLINE", "POLYLINE", "SPLINE"],
        "Symbol" | "Block insert" => &["INSERT"],
        "Triangle" => &["3DFACE"],
        "Image" => &["IMAGE", "MTEXT", "OLE2FRAME"],
        _ => &[],
    }
}

/// Removes the formatting that DXF text may carry, so it compares to the NCZ string.
fn plain_text(text: &str) -> String {
    crate::features::decode_unicode_escapes(text)
        .replace("%%u", "")
        .replace("%%U", "")
        .replace("%%o", "")
        .replace("%%O", "")
        .trim()
        .to_owned()
}

fn score(
    item: &NczItem,
    points: &[(f64, f64)],
    entity: &Entity,
    entity_points: &[(f64, f64)],
    tolerance: f64,
) -> f64 {
    let mut score =
        coverage(points, entity_points, tolerance) + coverage(entity_points, points, tolerance);
    if expected_kinds(item.kind).contains(&entity.kind.as_str()) {
        score += 0.5;
    }
    if let (Some(text), Some(entity_text)) = (&item.text, entity.text())
        && plain_text(text) == plain_text(&entity_text)
    {
        score += 1.0;
    }
    if let (Some(radius), Some(entity_radius)) = (item.features.get("radius"), entity.get(40))
        && let (Ok(a), Ok(b)) = (radius.parse::<f64>(), entity_radius.trim().parse::<f64>())
        && (a - b).abs() <= tolerance
    {
        score += 0.5;
    }

    // Arcs that share a center and radius differ only by their angles, which DXF stores in degrees.
    let angles = [("start_angle", 50), ("end_angle", 51)];
    for (key, code) in angles {
        if let (Some(radians), Some(degrees)) = (item.features.get(key), entity.get(code))
            && let (Ok(radians), Ok(degrees)) =
                (radians.parse::<f64>(), degrees.trim().parse::<f64>())
            && ((radians.to_degrees() - degrees + 180.0).rem_euclid(360.0) - 180.0).abs() < 0.01
        {
            score += 0.25;
        }
    }

    score
}

fn match_oriented(items: &[NczItem], entities: &[Entity], tolerance: f64, swap: bool) -> Matching {
    let orient = |&(x, y): &(f64, f64)| if swap { (y, x) } else { (x, y) };
    let entity_points: Vec<Vec<(f64, f64)>> = entities.iter().map(Entity::points).collect();

    let mut grid = Grid::new(tolerance);
    for (i, points) in entity_points.iter().enumerate() {
        let mut seen = vec![];
        for point in points {
            let key = grid.key(*point);
            if !seen.contains(&key) {
                seen.push(key);
                grid.insert(*point, i);
            }
        }
    }

    let mut candidates = vec![];
    for (item_index, item) in items.iter().enumerate() {
        let points: Vec<_> = item.points.iter().map(orient).collect();
        let Some(anchor) = points.first() else {
            continue;
        };

        let mut near: Vec<usize> = grid
            .near(*anchor)
            .filter(|&e| entities[e].block.is_some() == item.block.is_some())
            .filter(|&e| {
                entity_points[e]
                    .iter()
                    .any(|p| is_near(*p, *anchor, tolerance))
            })
            .collect();
        near.sort_unstable();
        near.dedup();
        // Export keeps the order, so the entity at the same relative position is the likeliest counterpart.
        // This also picks the right candidates where many objects share a point, such as in blocks.
        let order = |entity_index: usize| {
            (item_index as f64 / items.len() as f64 - entity_index as f64 / entities.len() as f64)
                .abs()
        };
        near.sort_by(|a, b| order(*a).total_cmp(&order(*b)));
        near.truncate(MAX_CANDIDATES);

        for entity_index in near {
            let score = score(
                item,
                &points,
                &entities[entity_index],
                &entity_points[entity_index],
                tolerance,
            );
            candidates.push((score, order(entity_index), item_index, entity_index));
        }
    }

    candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.total_cmp(&b.1)));

    let mut item_taken = vec![false; items.len()];
    let mut entity_taken = vec![false; entities.len()];
    let mut matches = vec![];
    for &(score, _, item, entity) in &candidates {
        if item_taken[item] || entity_taken[entity] {
            continue;
        }
        item_taken[item] = true;
        entity_taken[entity] = true;
        matches.push(Match {
            item,
            entity,
            score,
            secondary: false,
        });
    }
    for &(score, _, item, entity) in &candidates {
        if entity_taken[entity] || score < SECONDARY_SCORE {
            continue;
        }
        entity_taken[entity] = true;
        matches.push(Match {
            item,
            entity,
            score,
            secondary: true,
        });
    }
    matches.sort_by_key(|m| (m.item, m.secondary));

    Matching {
        swapped: swap,
        matches,
        unmatched_entities: (0..entities.len()).filter(|&e| !entity_taken[e]).collect(),
        unmatched_items: (0..items.len())
            .filter(|&i| !item_taken[i] && !items[i].points.is_empty())
            .collect(),
    }
}

/// Matches in both axis orders and keeps the one that pairs more geometries well.
pub fn match_items(items: &[NczItem], entities: &[Entity], tolerance: f64) -> Matching {
    let good = |m: &Matching| {
        m.matches
            .iter()
            .filter(|m| !m.secondary && m.score >= 1.5)
            .count()
    };
    let straight = match_oriented(items, entities, tolerance, false);
    let swapped = match_oriented(items, entities, tolerance, true);

    if good(&swapped) > good(&straight) {
        swapped
    } else {
        straight
    }
}
