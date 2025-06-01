pub mod coordinate;
pub mod geometry;
pub mod line_type;

use crate::geometry::Geometry;
use crate::line_type::LineType;
use bopper::ReadBuffer;

#[derive(Debug, serde::Serialize)]
pub enum Object {
    FileHeader { bytes: Vec<u8> },
    UserInformation { name: String },
    LineTypes { types: Vec<LineType> },
    Geometry { geometry: Geometry },
    Version { name: String },
    Metadata { name: String, content: String },
    LayerInformation { color: u32, name: String },
    Unknown { id: u8, bytes: Vec<u8> },
}

impl Object {
    pub fn from_buffer(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let object_type_id = buffer.get_u8()?;
        let object_length = buffer.get_u32()?.try_into().ok()?;

        let mut buffer = buffer.get_buffer(object_length)?;

        match object_type_id {
            1 => Some(Self::FileHeader {
                bytes: buffer.to_vec(),
            }),
            2 => {
                let name = get_string(&mut buffer)?;

                Some(Self::UserInformation { name })
            }
            8 => {
                let mut types = vec![];
                loop {
                    let Some(line_type) = LineType::read(&mut buffer) else {
                        continue;
                    };

                    types.push(line_type);

                    if buffer.is_empty() {
                        break;
                    }
                }

                Some(Self::LineTypes { types })
            }
            21 => Geometry::from_buffer(&mut buffer).map(|g| Self::Geometry { geometry: g }),
            25 => {
                let name = get_string(&mut buffer)?;

                Some(Self::Version { name })
            }
            28 => {
                let name = get_string_with_padding(&mut buffer, 10)?;

                let content_bytes = buffer.to_vec();

                let content = String::from_utf8(content_bytes).ok().unwrap_or_default();
                let content = content.trim().to_owned();

                Some(Self::Metadata { name, content })
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

pub fn get_string(buffer: &mut impl ReadBuffer) -> Option<String> {
    let string_length = buffer.get_u8()?.into();
    let string_bytes = buffer.get_slice(string_length)?;

    String::from_utf8(string_bytes).ok()
}

pub fn get_string_with_padding(
    buffer: &mut impl ReadBuffer,
    total_length: usize,
) -> Option<String> {
    let string_length = buffer.get_u8()?.into();
    let string_bytes = buffer.get_buffer(total_length)?.to_vec();
    let actual_bytes = &string_bytes[..string_length];

    let string = String::from_utf8_lossy(actual_bytes).to_string();
    Some(string)
}
