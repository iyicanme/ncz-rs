use bopper::{BigEndianReadBuffer, ReadBuffer};
use crate::coordinate::Coordinate;
use crate::geometry::Geometry;
use crate::line_type::LineType;
use crate::string::{get_string, get_string_with_padding};

const GEOMETRY_HEADER_LENGTH: usize = 81;
const RASTER_REFERENCE_LENGTH: usize = 181;

#[derive(Debug, serde::Serialize)]
pub struct Layer {
    /// Index into the AutoCAD color index palette.
    pub color: u8,
    pub unknown1: u8,
    pub unknown2: u8,
    pub unknown3: u8,
    pub flags: u32,
    pub name: String,
}

#[derive(Debug, serde::Serialize)]
pub struct SymbolLibrary {
    pub name: String,
    pub scale: f64,
    pub unknown: Vec<u8>,
}

#[derive(Debug, serde::Serialize)]
pub struct RasterReference {
    /// Path of a raster registration (`.DRE`) file.
    pub path: String,
    pub unknown: Vec<u8>,
}

/// A page of a spatial index over the geometries that follow it.
#[derive(Debug, serde::Serialize)]
pub struct IndexPage {
    pub lower_bound: Coordinate,
    pub upper_bound: Coordinate,
    /// Bit `1 << t` is set when a geometry of type `t` is in the page.
    pub geometry_types: u32,
    /// Indices of the layers that geometries in the page are on, stored as a 256 bit mask.
    pub layers: Vec<u8>,
    /// File offset of the next page of the same kind, -1 for the last page.
    pub next: i32,
    /// File offset of the previous page of the same kind, -1 for the first page.
    pub previous: i32,
    /// Number of geometries in the page.
    pub count: u32,
}

#[derive(Debug, serde::Serialize)]
pub enum MetadataContent {
    /// XML, JSON or plain text.
    Text(String),
    /// Content of the `SYMBOLS` entry.
    SymbolLibraries { version: u32, unknown: u32, libraries: Vec<SymbolLibrary> },
    Binary(Vec<u8>),
}

#[derive(Debug, serde::Serialize)]
pub enum Object {
    FileHeader { bytes: Vec<u8> },
    UserInformation { name: String },
    /// Map scale divided by 1000, e.g. 0.5 for 1:500.
    Scale { scale: f64 },
    Extents { lower_bound: Coordinate, upper_bound: Coordinate },
    /// Path of a linked file, e.g. a `.NCO` file. Empty in most files.
    LinkedFile { path: String },
    Layers { unknown1: Vec<u8>, layers: Vec<Layer>, unknown2: Vec<u8> },
    LineTypes { types: Vec<LineType> },
    Fonts { names: Vec<String> },
    /// Symbol and block library files.
    Libraries { names: Vec<String> },
    BlockSectionBegin,
    /// A BMP image, including its file header.
    Thumbnail { bytes: Vec<u8> },
    BlockBegin { name: String },
    BlockEnd,
    Geometry { geometry: Geometry },
    BlockSectionEnd,
    Version { name: String },
    RasterReferences { references: Vec<RasterReference> },
    /// Groups the [`Object::IndexPage`]s that follow it.
    IndexGroup { page: IndexPage },
    IndexPage { page: IndexPage },
    Metadata { name: String, content: MetadataContent },
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
            3 => {
                let scale = buffer.get_f64()?;

                Some(Self::Scale { scale })
            }
            4 => {
                let lower_bound = Coordinate::from_buffer_2d(&mut buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(&mut buffer)?;

                Some(Self::Extents {
                    lower_bound,
                    upper_bound,
                })
            }
            5 => {
                let path = get_string_with_padding(&mut buffer, 79)?;

                Some(Self::LinkedFile { path })
            }
            6 => {
                let unknown1 = buffer.get_slice(9)?;

                let mut layers = vec![];
                while buffer.remaining() >= 29 {
                    let color = buffer.get_u8()?;
                    let unknown1 = buffer.get_u8()?;
                    let unknown2 = buffer.get_u8()?;
                    let unknown3 = buffer.get_u8()?;
                    let flags = buffer.get_u32()?;
                    let name = get_string_with_padding(&mut buffer, 20)?;

                    layers.push(Layer {
                        color,
                        unknown1,
                        unknown2,
                        unknown3,
                        flags,
                        name,
                    });
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
            26 => {
                let mut references = vec![];
                while buffer.remaining() >= RASTER_REFERENCE_LENGTH {
                    let path = get_string_with_padding(&mut buffer, 80)?;
                    let unknown = buffer.get_slice(RASTER_REFERENCE_LENGTH - 81)?;

                    references.push(RasterReference { path, unknown });
                }

                Some(Self::RasterReferences { references })
            }
            27 => read_index_page(&mut buffer).map(|page| Self::IndexPage { page }),
            29 => read_index_page(&mut buffer).map(|page| Self::IndexGroup { page }),
            28 => {
                let name = get_string_with_padding(&mut buffer, 10)?;

                let content_bytes = buffer.to_vec();
                let content = match name.as_str() {
                    "SYMBOLS" => read_symbol_libraries(&mut BigEndianReadBuffer::new(&content_bytes)),
                    _ => None,
                };
                let content = content.unwrap_or_else(|| match decode_metadata(&content_bytes) {
                    Some(content) => MetadataContent::Text(content.trim().to_owned()),
                    None => MetadataContent::Binary(content_bytes),
                });

                Some(Self::Metadata { name, content })
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

fn read_index_page(buffer: &mut impl ReadBuffer) -> Option<IndexPage> {
    let lower_bound = Coordinate::from_buffer_2d(buffer)?;
    let upper_bound = Coordinate::from_buffer_2d(buffer)?;
    let geometry_types = buffer.get_u32()?;
    let layer_mask = buffer.get_slice(32)?;
    let layers = (0..=u8::MAX)
        .filter(|&layer| layer_mask[usize::from(layer / 8)] & (1 << (layer % 8)) != 0)
        .collect();
    let next = buffer.get_u32()?.cast_signed();
    let previous = buffer.get_u32()?.cast_signed();
    let count = buffer.get_u32()?;

    Some(IndexPage {
        lower_bound,
        upper_bound,
        geometry_types,
        layers,
        next,
        previous,
        count,
    })
}

fn read_file_names(buffer: &mut impl ReadBuffer) -> Option<Vec<String>> {
    let count = buffer.get_u32()?;

    (0..count)
        .map(|_| get_string_with_padding(buffer, 79))
        .collect()
}

fn read_symbol_libraries(buffer: &mut impl ReadBuffer) -> Option<MetadataContent> {
    let version = buffer.get_u32()?;
    let count = buffer.get_u32()?;
    let unknown = buffer.get_u32()?;

    let libraries = (0..count)
        .map(|_| {
            let name = get_string_with_padding(buffer, 80)?;
            let scale = buffer.get_f64()?;
            let unknown = buffer.get_slice(26)?;

            Some(SymbolLibrary {
                name,
                scale,
                unknown,
            })
        })
        .collect::<Option<Vec<_>>>()?;

    Some(MetadataContent::SymbolLibraries {
        version,
        unknown,
        libraries,
    })
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
