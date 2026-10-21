use crate::coordinate::Coordinate;
use crate::geometry::Geometry;
use crate::line_type::LineType;
use crate::string::{get_string, get_string_with_padding};
use bopper::{BigEndianReadBuffer, ReadBuffer};

pub(crate) const GEOMETRY_HEADER_LENGTH: usize = 81;
const LAYERS_HEADER_LENGTH: usize = 9;

/// Bit of [`Layer::flags`] that is set when the layer is shown. NetCAD exports layers without it as off.
pub const LAYER_VISIBLE: u32 = 0x01;
const RASTER_REFERENCE_LENGTH: usize = 181;
const LAYER_STYLES_HEADER_LENGTH: usize = 63;
const LAYER_STYLE_LENGTH: usize = 256;

#[derive(Debug, serde::Serialize)]
pub struct Layer {
    /// A bitmask, see [`LAYER_VISIBLE`].
    pub flags: u32,
    pub name: String,
    /// Index into the AutoCAD color index palette.
    pub color: u8,
    pub unknown1: u8,
    pub unknown2: u8,
    pub unknown3: u8,
}

#[derive(Debug, serde::Serialize)]
pub struct SymbolLibrary {
    pub name: String,
    pub scale: f64,
    pub unknown: Vec<u8>,
}

/// An entry of the `LEX.ST2` metadata, in the order of [`Object::Layers`].
#[derive(Debug, serde::Serialize)]
pub struct LayerStyle {
    /// True color of the layer, probably in R, G, B order. It differs from the ACI color of the layer index when
    /// the user changed the palette, e.g. index 102 is `9a cf 19` in one file and `80 ff 00` in another.
    pub rgb: [u8; 3],
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
    SymbolLibraries {
        version: u32,
        unknown: u32,
        libraries: Vec<SymbolLibrary>,
    },
    /// Content of the `MPROJ` entry, the coordinate system.
    Projection {
        /// 1 for geographic coordinates, 2 for 6° zones, 3 for 3° zones, 0 when undefined.
        projection: u8,
        /// 0 WGS-84, 1 ITRF, 4 ED50, 254 ED50 (HGK).
        datum: u8,
        unknown1: Vec<u8>,
        /// Central meridian of the zone in degrees, e.g. 33.
        central_meridian: u8,
        unknown2: Vec<u8>,
    },
    /// Content of the `LEX.ST2` entry, extra settings for each layer.
    LayerStyles {
        unknown1: Vec<u8>,
        layers: Vec<LayerStyle>,
        unknown2: Vec<u8>,
    },
    Binary(Vec<u8>),
}

#[derive(Debug, serde::Serialize)]
pub enum Object {
    FileHeader {
        bytes: Vec<u8>,
    },
    UserInformation {
        name: String,
    },
    /// Map scale divided by 1000, e.g. 0.5 for 1:500.
    Scale {
        scale: f64,
    },
    Extents {
        lower_bound: Coordinate,
        upper_bound: Coordinate,
    },
    /// Path of a linked file, e.g. a `.NCO` file. Empty in most files.
    LinkedFile {
        path: String,
    },
    Layers {
        unknown1: Vec<u8>,
        unknown2: u8,
        unknown3: u8,
        layers: Vec<Layer>,
    },
    LineTypes {
        types: Vec<LineType>,
    },
    Fonts {
        names: Vec<String>,
    },
    /// Symbol and block library files.
    Libraries {
        names: Vec<String>,
    },
    BlockSectionBegin,
    /// A BMP image, including its file header.
    Thumbnail {
        bytes: Vec<u8>,
    },
    BlockBegin {
        name: String,
    },
    BlockEnd,
    Geometry {
        geometry: Geometry,
    },
    BlockSectionEnd,
    Version {
        name: String,
    },
    RasterReferences {
        references: Vec<RasterReference>,
    },
    /// Groups the [`Object::IndexPage`]s that follow it.
    IndexGroup {
        page: IndexPage,
    },
    IndexPage {
        page: IndexPage,
    },
    Metadata {
        name: String,
        content: MetadataContent,
    },
    /// An area record around the polylines up to [`Object::AreaEnd`]. Some files wrap every polyline in one, and
    /// NetCAD draws them as plain outlines, not filled. The body holds the name, bounds and centroid. Formerly read as
    /// a hatch.
    AreaBegin {
        bytes: Vec<u8>,
    },
    AreaEnd {
        begin_length: u32,
    },
    /// Extra style for the next geometry, which has type `geometry_type`.
    GeometryStyle {
        geometry_type: u8,
        bytes: Vec<u8>,
    },
    LayerInformation {
        color: u32,
        name: String,
    },
    Unknown {
        id: u8,
        bytes: Vec<u8>,
    },
}

impl Object {
    /// Whether NCZ files use the object type. Type 10 is a marker without a body that has no decoder.
    pub fn is_known_type(id: u8) -> bool {
        matches!(id, 1..=6 | 8 | 10..=16 | 21..=23 | 25..=31 | 33 | 226)
    }

    /// Image geometries declare a length that excludes the geometry header, which follows the declared body. Takes
    /// the object type and the second byte of the body, the geometry type.
    pub(crate) fn has_extra_geometry_header(object_type_id: u8, geometry_type: Option<u8>) -> bool {
        object_type_id == 21 && geometry_type == Some(15)
    }

    pub fn from_buffer(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let object_type_id = buffer.get_u8()?;
        let object_length = buffer.get_u32()?.try_into().ok()?;

        let mut bytes = buffer.get_slice(object_length)?;
        if Self::has_extra_geometry_header(object_type_id, bytes.get(1).copied()) {
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
                let unknown1 = buffer.get_slice(LAYERS_HEADER_LENGTH)?;
                let unknown2 = buffer.get_u8()?;
                let unknown3 = buffer.get_u8()?;
                let count = buffer.get_u16()?;

                let layers = (0..count)
                    .map(|_| {
                        let flags = buffer.get_u32()?;
                        let name = get_string_with_padding(&mut buffer, 20)?;
                        let color = buffer.get_u8()?;
                        let unknown1 = buffer.get_u8()?;
                        let unknown2 = buffer.get_u8()?;
                        let unknown3 = buffer.get_u8()?;

                        Some(Layer {
                            flags,
                            name,
                            color,
                            unknown1,
                            unknown2,
                            unknown3,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;

                Some(Self::Layers {
                    unknown1,
                    unknown2,
                    unknown3,
                    layers,
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
                    "SYMBOLS" => {
                        read_symbol_libraries(&mut BigEndianReadBuffer::new(&content_bytes))
                    }
                    "MPROJ" => read_projection(&mut BigEndianReadBuffer::new(&content_bytes)),
                    "LEX.ST2" => read_layer_styles(&mut BigEndianReadBuffer::new(&content_bytes)),
                    _ => None,
                };
                let content = content.unwrap_or_else(|| match decode_metadata(&content_bytes) {
                    Some(content) => MetadataContent::Text(content.trim().to_owned()),
                    None => MetadataContent::Binary(content_bytes),
                });

                Some(Self::Metadata { name, content })
            }
            30 => Some(Self::AreaBegin {
                bytes: buffer.to_vec(),
            }),
            31 => {
                let begin_length = buffer.get_u32()?;

                Some(Self::AreaEnd { begin_length })
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

fn read_projection(buffer: &mut impl ReadBuffer) -> Option<MetadataContent> {
    let projection = buffer.get_u8()?;
    let datum = buffer.get_u8()?;
    let unknown1 = buffer.get_slice(3)?;
    let central_meridian = buffer.get_u8()?;

    Some(MetadataContent::Projection {
        projection,
        datum,
        unknown1,
        central_meridian,
        unknown2: buffer.to_vec(),
    })
}

fn read_layer_styles(buffer: &mut impl ReadBuffer) -> Option<MetadataContent> {
    let unknown1 = buffer.get_slice(LAYER_STYLES_HEADER_LENGTH)?;
    // Only the low byte of the layer count is known to be the count.
    let count = unknown1[4];

    let layers = (0..count)
        .map(|_| {
            let rgb = buffer.get_slice(3)?.try_into().ok()?;
            let unknown = buffer.get_slice(LAYER_STYLE_LENGTH - 3)?;

            Some(LayerStyle { rgb, unknown })
        })
        .collect::<Option<Vec<_>>>()?;

    Some(MetadataContent::LayerStyles {
        unknown1,
        layers,
        unknown2: buffer.to_vec(),
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

    let looks_like_utf16 =
        bytes.len() >= 2 && bytes.len().is_multiple_of(2) && bytes[0] != 0 && bytes[1] == 0;
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
