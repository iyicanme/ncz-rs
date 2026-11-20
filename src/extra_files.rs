//! Files that go next to the DXF: the coordinate system as a `.prj` file, and the pictures and rich texts that
//! NCZ files embed (type-15 objects), whose content a DXF frame cannot hold.
//!
//! The coordinate system is the one `MPROJ` names. When that gives none (no `MPROJ`, projection 0 or an unknown
//! datum), the file's map sheets can still tell it: the system that places most sheets at their stored bounds, see
//! [`map_sheet::fitting_projection`]. All 6 corpus files without a usable `MPROJ` that have sheets fit ED50 / TM33.

use crate::geometry::Geometry;
use crate::map_sheet;
use crate::object::{MetadataContent, Object};
use std::collections::BTreeMap;

/// How far a sheet's projected corners may be from its stored bounds for its system to count, in drawing units.
const SHEET_TOLERANCE: f64 = 0.01;

/// A file to write next to the DXF, named by the DXF's name without extension followed by `suffix`.
#[derive(Debug, serde::Serialize)]
pub struct ExtraFile {
    /// E.g. `.prj`, `-image-1.png` or `-text-1.rtf`.
    pub suffix: String,
    pub size: usize,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

/// Collects the extra files from the objects of an NCZ file, which it is shown one by one.
#[derive(Debug, Default)]
pub struct ExtraFiles {
    prj: Option<String>,
    /// How many map sheets each coordinate system places at their stored bounds, by `MPROJ` codes.
    sheet_systems: BTreeMap<(u8, u8, u8), usize>,
    /// Embedded pictures, as their file extension and bytes.
    images: Vec<(&'static str, Vec<u8>)>,
    /// Embedded rich texts, as RTF.
    texts: Vec<Vec<u8>>,
}

impl ExtraFiles {
    pub fn inspect(&mut self, object: &Object) {
        match object {
            Object::Metadata {
                content:
                    MetadataContent::Projection {
                        projection,
                        datum,
                        central_meridian,
                        ..
                    },
                ..
            } => self.prj = prj(*projection, *datum, *central_meridian),
            Object::Geometry {
                geometry: Geometry::MapSheet { header, name },
            } => {
                // NCZ coordinates have northing as x.
                let (lower, upper) = (header.lower_bound, header.upper_bound);
                let bounds = [lower.y, lower.x, upper.y, upper.x];
                if let Some(codes) = map_sheet::fitting_projection(name, None, bounds, SHEET_TOLERANCE)
                    .and_then(|projection| projection.mproj_codes())
                {
                    *self.sheet_systems.entry(codes).or_default() += 1;
                }
            }
            Object::Geometry {
                geometry: Geometry::Image { properties, .. },
            } => {
                for property in properties {
                    let Some(bytes) = decode_base64(&property.value) else {
                        continue;
                    };
                    match property.key.as_str() {
                        "imageContent" => {
                            if let Some(extension) = image_extension(&bytes) {
                                self.images.push((extension, bytes));
                            }
                        }
                        "rtfData" if bytes.starts_with(b"{\\rtf") => self.texts.push(bytes),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// The `MPROJ` codes of the coordinate system that the map sheets show, when `MPROJ` itself gives no `.prj`.
    pub fn inferred_system(&self) -> Option<(u8, u8, u8)> {
        if self.prj.is_some() {
            return None;
        }
        self.sheet_systems
            .iter()
            .max_by_key(|&(_, count)| *count)
            .map(|(&codes, _)| codes)
    }

    pub fn into_files(self) -> Vec<ExtraFile> {
        let inferred = self.inferred_system();
        let prj = self.prj.or_else(|| {
            let (projection, datum, central_meridian) = inferred?;
            prj(projection, datum, central_meridian)
        });
        let mut files = vec![];
        let mut add = |suffix: String, bytes: Vec<u8>| {
            files.push(ExtraFile {
                suffix,
                size: bytes.len(),
                bytes,
            });
        };
        if let Some(prj) = prj {
            add(".prj".to_owned(), prj.into_bytes());
        }
        for (index, (extension, bytes)) in self.images.into_iter().enumerate() {
            add(format!("-image-{}.{extension}", index + 1), bytes);
        }
        for (index, bytes) in self.texts.into_iter().enumerate() {
            add(format!("-text-{}.rtf", index + 1), bytes);
        }
        files
    }
}

/// The file extension of an image by its first bytes, `None` for formats that are not recognised.
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("jpg")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// Decodes standard base64, ignoring whitespace. `None` when the text is not base64.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let digits: Vec<u8> = text
        .bytes()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let digits = digits.strip_suffix(b"==").or_else(|| digits.strip_suffix(b"=")).unwrap_or(&digits);
    if digits.is_empty() || digits.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(digits.len() * 3 / 4);
    for chunk in digits.chunks(4) {
        let mut word = 0u32;
        for (index, &c) in chunk.iter().enumerate() {
            word |= value(c)? << (18 - 6 * index);
        }
        bytes.extend_from_slice(&word.to_be_bytes()[1..chunk.len()]);
    }
    Some(bytes)
}

/// The coordinate system of an `MPROJ` entry as an ESRI-style WKT `.prj` file, `None` when the projection is not set
/// or the datum is not known. Turkish 3° zones are transverse Mercator with scale 1 and a false easting of 500 km, as
/// TUREF / TM27–TM45 (EPSG:5253–5259) and ED50 / TM27–TM45 (EPSG:2319–2325) define them. Datum 0 is taken as WGS 84,
/// as documented, though some files with it are named as ITRF; at map scale the two differ by less than a metre.
pub fn prj(projection: u8, datum: u8, central_meridian: u8) -> Option<String> {
    const DEGREE: &str = r#"UNIT["Degree",0.0174532925199433]"#;
    let (datum_name, geographic) = match datum {
        0 => (
            "WGS_1984",
            r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]]"#,
        ),
        1 => (
            "TUREF",
            r#"GEOGCS["GCS_TUREF",DATUM["D_Turkish_National_Reference_Frame",SPHEROID["GRS_1980",6378137.0,298.257222101]]"#,
        ),
        4 | 254 => (
            "ED_1950",
            r#"GEOGCS["GCS_European_1950",DATUM["D_European_1950",SPHEROID["International_1924",6378388.0,297.0]]"#,
        ),
        _ => return None,
    };
    let geographic = format!(r#"{geographic},PRIMEM["Greenwich",0.0],{DEGREE}]"#);
    let meridian = f64::from(central_meridian);
    let transverse_mercator = |name: String, scale: f64| {
        format!(
            r#"PROJCS["{name}",{geographic},PROJECTION["Transverse_Mercator"],PARAMETER["False_Easting",500000.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",{meridian:.1}],PARAMETER["Scale_Factor",{scale:?}],PARAMETER["Latitude_Of_Origin",0.0],UNIT["Meter",1.0]]"#
        )
    };
    match projection {
        1 => Some(geographic),
        2 if (u16::from(central_meridian) + 183) % 6 == 0 && central_meridian <= 177 => {
            let zone = (u16::from(central_meridian) + 183) / 6;
            Some(transverse_mercator(
                format!("{datum_name}_UTM_Zone_{zone}N"),
                0.9996,
            ))
        }
        3 => Some(transverse_mercator(format!("{datum_name}_TM{central_meridian}"), 1.0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_projection_files() {
        assert_eq!(
            prj(3, 1, 33).unwrap(),
            r#"PROJCS["TUREF_TM33",GEOGCS["GCS_TUREF",DATUM["D_Turkish_National_Reference_Frame",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Transverse_Mercator"],PARAMETER["False_Easting",500000.0],PARAMETER["False_Northing",0.0],PARAMETER["Central_Meridian",33.0],PARAMETER["Scale_Factor",1.0],PARAMETER["Latitude_Of_Origin",0.0],UNIT["Meter",1.0]]"#
        );
        assert!(prj(3, 4, 30).unwrap().starts_with(r#"PROJCS["ED_1950_TM30",GEOGCS["GCS_European_1950""#));
        assert!(prj(2, 0, 33).unwrap().contains(r#"PROJCS["WGS_1984_UTM_Zone_36N""#));
        assert!(prj(2, 0, 33).unwrap().contains(r#"PARAMETER["Scale_Factor",0.9996]"#));
        assert!(prj(1, 1, 0).unwrap().starts_with(r#"GEOGCS["GCS_TUREF""#));
        // Projection not set, unknown datum, or a central meridian that is no UTM zone's.
        assert_eq!(prj(0, 0, 0), None);
        assert_eq!(prj(3, 255, 33), None);
        assert_eq!(prj(2, 0, 34), None);
    }

    /// A map sheet record at the stored bounds that `projection` gives the sheet.
    fn map_sheet(name: &str, projection: crate::projection::TransverseMercator) -> Object {
        use crate::coordinate::Coordinate;
        use crate::geometry::Header;
        let corners = crate::map_sheet::geographic_corners(name).unwrap().map(|(lon, lat)| projection.forward(lon, lat));
        let (east, north): (Vec<f64>, Vec<f64>) = corners.into_iter().unzip();
        let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        // NCZ coordinates have northing as x.
        let header = Header {
            unknown1: 0,
            layer_id: 0,
            line_type: 0,
            line_type2: 0,
            color: 0,
            attributes: vec![],
            lower_bound: Coordinate { x: min(&north), y: min(&east), z: 0.0 },
            upper_bound: Coordinate { x: max(&north), y: max(&east), z: 0.0 },
            unknown2: 0,
            code: None,
            link: None,
        };
        Object::Geometry {
            geometry: Geometry::MapSheet {
                header,
                name: name.to_owned(),
            },
        }
    }

    #[test]
    fn infers_the_coordinate_system_from_map_sheets() {
        use crate::projection::{INTERNATIONAL_1924, TransverseMercator};
        let ed50_tm33 = TransverseMercator::turkish_zone(INTERNATIONAL_1924, 33.0);

        // No MPROJ: the sheets show ED50 / TM33, which becomes the .prj.
        let mut extra = ExtraFiles::default();
        extra.inspect(&map_sheet("F29-D-14-A-1", ed50_tm33));
        extra.inspect(&map_sheet("F29-D-14-A-2", ed50_tm33));
        assert_eq!(extra.inferred_system(), Some((3, 4, 33)));
        let files = extra.into_files();
        assert_eq!(files[0].suffix, ".prj");
        assert_eq!(files[0].bytes, prj(3, 4, 33).unwrap().into_bytes());

        // MPROJ wins when it names a system, whatever the sheets show.
        let mut extra = ExtraFiles::default();
        extra.inspect(&Object::Metadata {
            name: "MPROJ".to_owned(),
            content: MetadataContent::Projection {
                projection: 3,
                datum: 1,
                unknown1: vec![],
                central_meridian: 33,
                unknown2: vec![],
            },
        });
        extra.inspect(&map_sheet("F29-D-14-A-1", ed50_tm33));
        assert_eq!(extra.inferred_system(), None);
        assert_eq!(extra.into_files()[0].bytes, prj(3, 1, 33).unwrap().into_bytes());

        // A sheet that no system places at its bounds tells nothing.
        let mut sheet = map_sheet("F29-D-14-A-1", ed50_tm33);
        if let Object::Geometry {
            geometry: Geometry::MapSheet { header, .. },
        } = &mut sheet
        {
            header.lower_bound.y += 5.0;
            header.upper_bound.y += 5.0;
        }
        let mut extra = ExtraFiles::default();
        extra.inspect(&sheet);
        assert_eq!(extra.inferred_system(), None);
        assert!(extra.into_files().is_empty());
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(decode_base64("TmV0Q0FE").unwrap(), b"NetCAD");
        assert_eq!(decode_base64("TmV0\r\nQ0FEIQ==").unwrap(), b"NetCAD!");
        assert_eq!(decode_base64("TmV0Q0FEIT8=").unwrap(), b"NetCAD!?");
        assert_eq!(decode_base64("not base64!"), None);
        assert_eq!(decode_base64(""), None);
    }

    #[test]
    fn recognises_images() {
        assert_eq!(image_extension(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_extension(b"\xff\xd8\xff\xe0"), Some("jpg"));
        assert_eq!(image_extension(b"{\\rtf1"), None);
    }
}
