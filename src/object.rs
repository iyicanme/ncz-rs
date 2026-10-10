use bopper::{BigEndianReadBuffer, ReadBuffer};
use crate::coordinate::Coordinate;
use crate::geometry::Geometry;
use crate::line_type::LineType;
use crate::string::{get_string, get_string_with_padding};

const GEOMETRY_HEADER_LENGTH: usize = 81;

#[derive(Debug, serde::Serialize)]
pub struct Layer {
    pub unknown1: Vec<u8>,
    pub name: String,
}

#[derive(Debug, serde::Serialize)]
pub enum Object {
    FileHeader { bytes: Vec<u8> },
    UserInformation { name: String },
    Extents { lower_bound: Coordinate, upper_bound: Coordinate },
    Layers { unknown1: Vec<u8>, layers: Vec<Layer>, unknown2: Vec<u8> },
    LineTypes { types: Vec<LineType> },
    Fonts { names: Vec<String> },
    /// Symbol and block library files.
    Libraries { names: Vec<String> },
    BlockSectionBegin,
    Thumbnail { bytes: Vec<u8> },
    BlockBegin { name: String },
    BlockEnd,
    Geometry { geometry: Geometry },
    BlockSectionEnd,
    Version { name: String },
    Metadata { name: String, content: String, bytes: Option<Vec<u8>> },
    /// Probably a hatch or fill; it wraps the next polyline until [`Object::HatchEnd`].
    HatchBegin { bytes: Vec<u8> },
    HatchEnd { begin_length: u32 },
    /// Extra style for the next geometry, which has type `geometry_type`.
    GeometryStyle { geometry_type: u8, bytes: Vec<u8> },
    LayerInformation { color: u32, name: String },
    Unknown { id: u8, bytes: Vec<u8> },
}

impl Object {
    pub fn from_buffer(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let object_type_id = buffer.get_u8()?;
        let object_length = buffer.get_u32()?.try_into().ok()?;

        let mut bytes = buffer.get_slice(object_length)?;
        // Image geometries declare a length that excludes the geometry header.
        if object_type_id == 21 && bytes.get(1) == Some(&15) {
            bytes.extend(buffer.get_slice(GEOMETRY_HEADER_LENGTH)?);
        }

        let mut buffer = BigEndianReadBuffer::new(&bytes);

        match object_type_id {
            1 => Some(Self::FileHeader {
                bytes: buffer.to_vec(),
            }),
            2 => {
                let name = get_string(&mut buffer)?;

                Some(Self::UserInformation { name })
            }
            4 => {
                let lower_bound = Coordinate::from_buffer_2d(&mut buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(&mut buffer)?;

                Some(Self::Extents {
                    lower_bound,
                    upper_bound,
                })
            }
            6 => {
                let unknown1 = buffer.get_slice(9)?;

                let mut layers = vec![];
                while buffer.remaining() >= 29 {
                    let unknown1 = buffer.get_slice(8)?;
                    let name = get_string_with_padding(&mut buffer, 20)?;

                    layers.push(Layer { unknown1, name });
                }

                let unknown2 = buffer.to_vec();

                Some(Self::Layers {
                    unknown1,
                    layers,
                    unknown2,
                })
            }
            8 => {
                let mut types = vec![];
                while let Some(line_type) = LineType::read(&mut buffer) {
                    types.push(line_type);
                }

                Some(Self::LineTypes { types })
            }
            11 => read_file_names(&mut buffer).map(|names| Self::Fonts { names }),
            12 => read_file_names(&mut buffer).map(|names| Self::Libraries { names }),
            13 => Some(Self::BlockSectionBegin),
            14 => Some(Self::Thumbnail {
                bytes: buffer.to_vec(),
            }),
            15 => {
                let name = get_string_with_padding(&mut buffer, 30)?;

                Some(Self::BlockBegin { name })
            }
            16 => Some(Self::BlockEnd),
            21 => Geometry::from_buffer(&mut buffer, false).map(|g| Self::Geometry { geometry: g }),
            22 => Geometry::from_buffer(&mut buffer, true).map(|g| Self::Geometry { geometry: g }),
            23 => Some(Self::BlockSectionEnd),
            25 => {
                let name = get_string(&mut buffer)?;

                Some(Self::Version { name })
            }
            28 => {
                let name = get_string_with_padding(&mut buffer, 10)?;

                let content_bytes = buffer.to_vec();
                let (content, bytes) = match decode_metadata(&content_bytes) {
                    Some(content) => (content.trim().to_owned(), None),
                    None => (String::new(), Some(content_bytes)),
                };

                Some(Self::Metadata {
                    name,
                    content,
                    bytes,
                })
            }
            30 => Some(Self::HatchBegin {
                bytes: buffer.to_vec(),
            }),
            31 => {
                let begin_length = buffer.get_u32()?;

                Some(Self::HatchEnd { begin_length })
            }
            33 => {
                let _length = buffer.get_u32()?;
                let geometry_type = buffer.get_u8()?;

                Some(Self::GeometryStyle {
                    geometry_type,
                    bytes: buffer.to_vec(),
                })
            }
            226 => {
                let color = buffer.get_u32()?;
                let name = get_string(&mut buffer)?;

                Some(Self::LayerInformation { color, name })
            }
            id => Some(Self::Unknown {
                id,
                bytes: buffer.to_vec(),
            }),
        }
    }
}

fn read_file_names(buffer: &mut impl ReadBuffer) -> Option<Vec<String>> {
    let count = buffer.get_u32()?;

    (0..count)
        .map(|_| get_string_with_padding(buffer, 79))
        .collect()
}

/// Decodes text metadata, which is either UTF-8 or UTF-16LE. Returns `None` for binary content.
fn decode_metadata(bytes: &[u8]) -> Option<String> {
    let as_text = |content: String| {
        let content = content.trim_end_matches('\0');
        let is_text = content
            .chars()
            .all(|c| !c.is_control() || c.is_whitespace());

        is_text.then(|| content.to_owned())
    };

    let looks_like_utf16 = bytes.len() >= 2 && bytes.len().is_multiple_of(2) && bytes[0] != 0 && bytes[1] == 0;
    if looks_like_utf16 {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();

        if let Some(content) = String::from_utf16(&units).ok().and_then(as_text) {
            return Some(content);
        }
    }

    String::from_utf8(bytes.to_vec()).ok().and_then(as_text)
}
