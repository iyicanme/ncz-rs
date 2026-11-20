//! The corners of map sheets of the Turkish large-scale sheet system, from their names, as NetCAD 5 draws them.
//!
//! A name such as `F30-A-08-D-1-B` starts with a 1:100,000 sheet: a letter for a 30′ band of latitude, counted south
//! from 44°N (`E` is 41°30′–42°), and a number for a 30′ column of longitude (`30` is 33°–33°30′). Each further part
//! divides the sheet before it: `A`–`D` into quarters clockwise from the north-west (1:50,000), `01`–`25` into a 5×5
//! grid row by row from the north-west (1:10,000), `A`–`D` into quarters (1:5,000), `1`–`4` into quarters in the same
//! order (1:2,000), and `A`–`D` again (1:1,000). These rules match all 2,732 named sheets of the corpus files with a
//! NetCAD 5 export to 0.0001″.

use crate::projection::{GRS80, INTERNATIONAL_1924, TransverseMercator, WGS84};

/// The corners of the sheet with the name, as longitude and latitude in degrees, clockwise from the north-west.
/// `None` when the name does not follow the sheet system.
pub fn geographic_corners(name: &str) -> Option<[(f64, f64); 4]> {
    let mut parts = name.trim().split('-');
    let first = parts.next()?;
    let mut chars = first.chars();
    let band = chars.next()?.to_ascii_uppercase();
    let column: u32 = chars.as_str().parse().ok()?;
    if !band.is_ascii_uppercase() || chars.as_str().is_empty() {
        return None;
    }
    let mut north = 44.0 - 0.5 * f64::from(u32::from(band) - u32::from('A'));
    let mut west = 18.0 + 0.5 * f64::from(column);
    let mut size = 0.5;

    // Quarter by letter or digit: (column from the west, row from the north), clockwise from the north-west.
    let quarter = |part: &str, labels: [&str; 4]| -> Option<(u32, u32)> {
        let index = labels.iter().position(|label| part.eq_ignore_ascii_case(label))?;
        Some([(0, 0), (1, 0), (1, 1), (0, 1)][index])
    };
    for (level, part) in parts.enumerate() {
        let (divisions, (cell_column, cell_row)) = match level {
            0 | 2 | 4 => (2, quarter(part, ["A", "B", "C", "D"])?),
            1 => {
                let number: u32 = part.parse().ok().filter(|n| (1..=25).contains(n))?;
                (5, ((number - 1) % 5, (number - 1) / 5))
            }
            3 => (2, quarter(part, ["1", "2", "3", "4"])?),
            _ => return None,
        };
        size /= f64::from(divisions);
        west += f64::from(cell_column) * size;
        north -= f64::from(cell_row) * size;
    }
    let (east, south) = (west + size, north - size);
    Some([(west, north), (east, north), (east, south), (west, south)])
}

/// The corners of a sheet in drawing coordinates (easting, northing), clockwise from the north-west, projected with
/// the drawing's coordinate system. That is the one `MPROJ` names, or else the Turkish 3° zone on TUREF, ED50 or
/// WGS 84 at the meridian nearest the sheet: files without `MPROJ` in the corpus fit ED50 / TM33. A system is only
/// taken when the corners' box matches `bounds` (easting/northing min and max, which NetCAD stores for the sheet) to
/// within `tolerance`, so a sheet never moves; `None` when no system matches.
pub fn projected_corners(
    name: &str,
    mproj: Option<TransverseMercator>,
    bounds: [f64; 4],
    tolerance: f64,
) -> Option<[(f64, f64); 4]> {
    let corners = geographic_corners(name)?;
    let meridian = (corners[0].0 / 3.0).round() * 3.0;
    let candidates = mproj.into_iter().chain(
        [GRS80, INTERNATIONAL_1924, WGS84]
            .into_iter()
            .map(|ellipsoid| TransverseMercator::turkish_zone(ellipsoid, meridian)),
    );
    for projection in candidates {
        let projected = corners.map(|(longitude, latitude)| projection.forward(longitude, latitude));
        let fits = [
            projected.iter().map(|p| p.0).fold(f64::INFINITY, f64::min) - bounds[0],
            projected.iter().map(|p| p.1).fold(f64::INFINITY, f64::min) - bounds[1],
            projected.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max) - bounds[2],
            projected.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max) - bounds[3],
        ]
        .iter()
        .all(|difference| difference.abs() <= tolerance);
        if fits {
            return Some(projected);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn decodes_sheet_names() {
        // 1:1,000 and 1:5,000 sheets of gökgöz_itrf96 and others, with NetCAD 5's corners.
        let corners = geographic_corners("F30-A-08-D-1-B").unwrap();
        let expected = [(33.10625, 41.425), (33.1125, 41.425), (33.1125, 41.41875), (33.10625, 41.41875)];
        assert!(corners.iter().zip(expected).all(|(&a, b)| close(a, b)), "{corners:?}");
        let [nw, _, se, _] = geographic_corners("F30-A-08-C-1-A").unwrap();
        assert!(close(nw, (33.125, 41.425)) && close(se, (33.13125, 41.41875)));
        // NetCAD's corners of this 1:5,000 sheet in ASAGIGUNEY: 33°–33.025°, 41.15°–41.175°.
        let [nw, _, se, _] = geographic_corners("F30-D-06-D").unwrap();
        assert!(close(nw, (33.0, 41.175)) && close(se, (33.025, 41.15)));
        // Ankara's 1:100,000 sheet.
        assert_eq!(geographic_corners("I29").unwrap()[0], (32.5, 40.0));
        assert_eq!(geographic_corners("G29-A-07-B-3-A").map(|c| c.len()), Some(4));
        // Not sheet names.
        assert_eq!(geographic_corners("23M-IV"), None);
        assert_eq!(geographic_corners("F30-E"), None);
        assert_eq!(geographic_corners("F30-A-26"), None);
        assert_eq!(geographic_corners("F30-A-08-D-1-B-A"), None);
    }

    #[test]
    fn finds_the_coordinate_system() {
        // F30-A-08-D-1-B in gökgöz_itrf96 (TUREF / TM33): its stored bounds.
        let bounds = [508881.696, 4587084.260, 509405.051, 4587779.058];
        let tm33 = TransverseMercator::from_mproj(3, 1, 33);
        let corners = projected_corners("F30-A-08-D-1-B", tm33, bounds, 0.01).unwrap();
        assert!((corners[0].0 - 508881.696).abs() < 0.001 && (corners[0].1 - 4587778.398).abs() < 0.001);
        // Without MPROJ, TUREF / TM33 is found from the bounds.
        assert!(projected_corners("F30-A-08-D-1-B", None, bounds, 0.01).is_some());
        // Bounds that no system gives leave the sheet alone.
        let moved = [bounds[0] + 5.0, bounds[1], bounds[2] + 5.0, bounds[3]];
        assert_eq!(projected_corners("F30-A-08-D-1-B", tm33, moved, 0.01), None);
    }
}
