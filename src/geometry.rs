use crate::coordinate::Coordinate;
use crate::string::{get_string, get_string_with_padding, get_varint_string};
use bopper::ReadBuffer;

pub const POLYLINE_CLOSED: u32 = 0x01;

/// Fields shared by every geometry record, read before the type specific part.
#[derive(Debug, serde::Serialize)]
pub struct Header {
    /// Zero in version 5 files, a copy of the geometry type in version 8 files.
    pub unknown1: u8,
    /// Index into the layer table, see [`crate::object::Object::Layers`].
    pub layer_id: u8,
    /// Index into the line type table, see [`crate::object::Object::LineTypes`].
    pub line_type: u8,
    /// Usually equal to `line_type`, sometimes with bit 0x80 set.
    pub line_type2: u8,
    /// Colour index of the geometry, zero when it takes the color of its layer.
    pub color: u8,
    pub attributes: Vec<u8>,
    pub lower_bound: Coordinate,
    pub upper_bound: Coordinate,
    pub unknown2: u32,
    /// Feature code, only present on geometries stored as object 22.
    pub code: Option<String>,
    /// Only present on geometries stored as object 22. Usually a database record reference such as `@TAR453489`
    /// (also `@PRG`, `@SEM`, `@TAB`), sometimes a free label such as a street name.
    pub link: Option<String>,
}

impl Header {
    /// Returns the geometry type, the anchor coordinate and the header.
    fn read(buffer: &mut impl ReadBuffer, has_code: bool) -> Option<(u8, Coordinate, Self)> {
        let unknown1 = buffer.get_u8()?;
        let geometry_type = buffer.get_u8()?;
        let layer_id = buffer.get_u8()?;
        let anchor = Coordinate::from_buffer_with_elevation(buffer)?;
        let line_type = buffer.get_u8()?;
        let line_type2 = buffer.get_u8()?;
        let attributes = buffer.get_slice(16)?;
        let color = attributes[3];
        let lower_bound = Coordinate::from_buffer_2d(buffer)?;
        let upper_bound = Coordinate::from_buffer_2d(buffer)?;
        let unknown2 = buffer.get_u32()?;
        let (code, link) = if has_code {
            (
                Some(get_string_with_padding(buffer, 10)?),
                Some(get_string_with_padding(buffer, 16)?),
            )
        } else {
            (None, None)
        };

        let header = Self {
            unknown1,
            layer_id,
            line_type,
            line_type2,
            color,
            attributes,
            lower_bound,
            upper_bound,
            unknown2,
            code,
            link,
        };

        Some((geometry_type, anchor, header))
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ImageProperty {
    pub key: String,
    pub kind: u8,
    pub value: String,
    pub caption: String,
    pub unknown: Vec<u8>,
}

#[derive(Debug, serde::Serialize)]
pub enum Geometry {
    Point {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        name: String,
        unknown3: Vec<u8>,
    },
    Line {
        #[serde(flatten)]
        header: Header,
        begin: Coordinate,
        end: Coordinate,
    },
    Circle {
        #[serde(flatten)]
        header: Header,
        center: Coordinate,
        radius: f64,
    },
    Arc {
        #[serde(flatten)]
        header: Header,
        center: Coordinate,
        radius: f64,
        unknown3: Vec<u8>,
        start_angle: f64,
        end_angle: f64,
    },
    Text {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        spacing: f32,
        size: f32,
        tilt: f32,
        /// Alignment as an ASCII letter, e.g. 'M', 'C', 'U', 'L'.
        snap: char,
        italic: bool,
        underline: bool,
        highlight: bool,
        /// Index into the font table, see [`crate::object::Object::Fonts`].
        font: u8,
        string: String,
    },
    /// Bit 0x01 of `flags` marks a closed polyline, see [`POLYLINE_CLOSED`].
    PolyLine {
        #[serde(flatten)]
        header: Header,
        name: String,
        id: u32,
        flags: u32,
        /// Area enclosed by the points. Some polylines store zero, and a few store a stale value.
        area: f64,
        points: Vec<Coordinate>,
    },
    /// Probably a reference to a symbol in one of the libraries listed in [`crate::object::Object::Libraries`].
    Symbol {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        scale: f32,
        /// In radians.
        rotation: f32,
        symbol: u16,
        unknown4: u8,
    },
    Contour {
        #[serde(flatten)]
        header: Header,
        origin: Coordinate,
        unknown3: Vec<u8>,
        points: Vec<Coordinate>,
        point_flags: Vec<u8>,
    },
    BlockInsert {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        name: String,
        id: u32,
        unknown3: u32,
        rotation: f64,
        scale_x: f64,
        scale_y: f64,
    },
    /// An object described by properties, such as an embedded image (`imageContent`) or rich text (`rtfData`).
    Image {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        scale: f32,
        unknown3: Vec<u8>,
        /// Number of bytes after this field, plus 8.
        data_length: u32,
        /// 2 for images and rich text, 1 for other objects.
        kind: u8,
        /// Identifies the kind of object, images share one and rich text shares another.
        class_id: String,
        unknown4: u32,
        properties: Vec<ImageProperty>,
    },
    /// A named rectangle given by two opposite corners, e.g. a plot window (`CIZPEN-n`) or a map sheet.
    Window {
        #[serde(flatten)]
        header: Header,
        corner1: Coordinate,
        corner2: Coordinate,
        name: String,
        /// Probably in radians, it is zero for most windows.
        rotation: f32,
    },
    /// A map sheet (pafta) frame, e.g. `G29-A-07-B-3-A`. Its extent is the header bounds.
    MapSheet {
        #[serde(flatten)]
        header: Header,
        name: String,
    },
    /// A face of a triangulated terrain model.
    Triangle {
        #[serde(flatten)]
        header: Header,
        vertices: [Coordinate; 3],
        min_z: f32,
        max_z: f32,
    },
    /// Has no body and inverted, empty bounds.
    Empty {
        #[serde(flatten)]
        header: Header,
    },
    Other {
        #[serde(flatten)]
        header: Header,
        id: u8,
        bytes: Vec<u8>,
    },
}

impl Geometry {
    pub fn from_buffer(buffer: &mut impl ReadBuffer, has_code: bool) -> Option<Self> {
        let (geometry_type, anchor, header) = Header::read(buffer, has_code)?;

        match geometry_type {
            1 => {
                let name = get_string_with_padding(buffer, 10)?;
                let unknown3 = buffer.get_slice(12)?;

                Some(Self::Point {
                    header,
                    position: anchor,
                    name,
                    unknown3,
                })
            }
            2 => {
                let mut end = Coordinate::from_buffer_2d(buffer)?;
                end.z = buffer.get_f32()?.into();

                Some(Self::Line {
                    header,
                    begin: anchor,
                    end,
                })
            }
            3 => {
                let radius = buffer.get_f64()?;

                Some(Self::Circle {
                    header,
                    center: anchor,
                    radius,
                })
            }
            4 => {
                let radius = buffer.get_f64()?;
                // Single precision copies of the start and end angles.
                buffer.get_slice(8)?;
                let unknown3 = buffer.get_slice(2)?;
                let start_angle = buffer.get_f64()?;
                let end_angle = buffer.get_f64()?;

                Some(Self::Arc {
                    header,
                    center: anchor,
                    radius,
                    unknown3,
                    start_angle,
                    end_angle,
                })
            }
            5 => {
                let spacing = f32::from_le_bytes(header.attributes[12..16].try_into().ok()?);
                let size = buffer.get_f32()?;
                let tilt = buffer.get_f32()?;
                let snap = buffer.get_u8()?.into();
                let flags = buffer.get_u8()?;
                let italic = (flags & 0x01) != 0;
                let underline = (flags & 0x02) != 0;
                let highlight = (flags & 0x04) != 0;

                let font = buffer.get_u8()?;
                let string = get_string(buffer)?;

                Some(Self::Text {
                    header,
                    position: anchor,
                    spacing,
                    size,
                    tilt,
                    snap,
                    italic,
                    underline,
                    highlight,
                    font,
                    string,
                })
            }
            6 => {
                let scale = buffer.get_f32()?;
                let rotation = buffer.get_f32()?;
                let symbol = buffer.get_u16()?;
                let unknown4 = buffer.get_u8()?;

                Some(Self::Symbol {
                    header,
                    position: anchor,
                    scale,
                    rotation,
                    symbol,
                    unknown4,
                })
            }
            7 => {
                let name = get_string_with_padding(buffer, 10)?;
                let id = buffer.get_u32()?;
                let flags = buffer.get_u32()?;
                let area = buffer.get_f64()?;

                let mut points = vec![];
                while let Some(c) = Coordinate::from_buffer_3d(buffer) {
                    points.push(c);
                }

                Some(Self::PolyLine {
                    header,
                    name,
                    id,
                    flags,
                    area,
                    points,
                })
            }
            9 => {
                let unknown3 = buffer.get_slice(36)?;

                // Points are stored as f32 offsets from the origin.
                let mut points = vec![];
                let mut point_flags = vec![];
                while let (Some(dx), Some(dy), Some(flag)) =
                    (buffer.get_f32(), buffer.get_f32(), buffer.get_u8())
                {
                    points.push(Coordinate {
                        x: anchor.x + f64::from(dx),
                        y: anchor.y + f64::from(dy),
                        z: anchor.z,
                    });
                    point_flags.push(flag);
                }

                Some(Self::Contour {
                    header,
                    origin: anchor,
                    unknown3,
                    points,
                    point_flags,
                })
            }
            10 => {
                let name = get_string_with_padding(buffer, 17)?;
                let corner2 = Coordinate::from_buffer_2d(buffer)?;
                let rotation = buffer.get_f32()?;

                Some(Self::Window {
                    header,
                    corner1: anchor,
                    corner2,
                    name,
                    rotation,
                })
            }
            11 => {
                let name = get_string_with_padding(buffer, 17)?;

                Some(Self::MapSheet { header, name })
            }
            12 => {
                let mut second = Coordinate::from_buffer_2d(buffer)?;
                second.z = buffer.get_f32()?.into();
                let mut third = Coordinate::from_buffer_2d(buffer)?;
                third.z = buffer.get_f32()?.into();
                let min_z = buffer.get_f32()?;
                let max_z = buffer.get_f32()?;

                Some(Self::Triangle {
                    header,
                    vertices: [anchor, second, third],
                    min_z,
                    max_z,
                })
            }
            13 => {
                let name = get_string_with_padding(buffer, 34)?;
                let id = buffer.get_u32()?;
                let unknown3 = buffer.get_u32()?;
                let rotation = buffer.get_f64()?;
                let scale_x = buffer.get_f64()?;
                let scale_y = buffer.get_f64()?;

                Some(Self::BlockInsert {
                    header,
                    position: anchor,
                    name,
                    id,
                    unknown3,
                    rotation,
                    scale_x,
                    scale_y,
                })
            }
            15 => {
                let scale = buffer.get_f32()?;
                let unknown3 = buffer.get_slice(24)?;
                let data_length = buffer.get_u32()?;
                let kind = buffer.get_u8()?;
                let class_id = buffer
                    .get_slice(16)?
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let unknown4 = buffer.get_u32()?;
                let property_count = buffer.get_u32()?;

                let mut properties = vec![];
                for _ in 0..property_count {
                    let key = get_string(buffer)?;
                    let kind = buffer.get_u8()?;
                    let value = get_varint_string(buffer)?;
                    let caption = get_varint_string(buffer)?;
                    let unknown = buffer.get_slice(7)?;

                    properties.push(ImageProperty {
                        key,
                        kind,
                        value,
                        caption,
                        unknown,
                    });
                }

                Some(Self::Image {
                    header,
                    position: anchor,
                    scale,
                    unknown3,
                    data_length,
                    kind,
                    class_id,
                    unknown4,
                    properties,
                })
            }
            14 => Some(Self::Empty { header }),
            id => Some(Self::Other {
                header,
                id,
                bytes: buffer.to_vec(),
            }),
        }
    }

    /// A human readable name of the kind of geometry.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Point { .. } => "Point",
            Self::Line { .. } => "Line",
            Self::Circle { .. } => "Circle",
            Self::Arc { .. } => "Arc",
            Self::Text { .. } => "Text",
            Self::PolyLine { .. } => "Polyline",
            Self::Symbol { .. } => "Symbol",
            Self::Contour { .. } => "Contour",
            Self::BlockInsert { .. } => "Block insert",
            Self::Image { .. } => "Image",
            Self::Window { .. } => "Window",
            Self::MapSheet { .. } => "Map sheet",
            Self::Triangle { .. } => "Triangle",
            Self::Empty { .. } => "Empty",
            Self::Other { .. } => "Other",
        }
    }

    pub fn header(&self) -> &Header {
        match self {
            Self::Point { header, .. }
            | Self::Line { header, .. }
            | Self::Circle { header, .. }
            | Self::Arc { header, .. }
            | Self::Text { header, .. }
            | Self::PolyLine { header, .. }
            | Self::Symbol { header, .. }
            | Self::Contour { header, .. }
            | Self::BlockInsert { header, .. }
            | Self::Image { header, .. }
            | Self::Window { header, .. }
            | Self::MapSheet { header, .. }
            | Self::Triangle { header, .. }
            | Self::Empty { header, .. }
            | Self::Other { header, .. } => header,
        }
    }
}
