//! Files that go next to the DXF: the coordinate system as a `.prj` file, and the pictures and rich texts that
//! NCZ files embed (type-15 objects), whose content a DXF frame cannot hold.

use crate::geometry::Geometry;
use crate::object::{MetadataContent, Object};

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

    pub fn into_files(self) -> Vec<ExtraFile> {
        let mut files = vec![];
        let mut add = |suffix: String, bytes: Vec<u8>| {
            files.push(ExtraFile {
                suffix,
                size: bytes.len(),
                bytes,
            });
        };
        if let Some(prj) = self.prj {
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
