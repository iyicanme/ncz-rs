use crate::coordinate::Coordinate;
use bopper::ReadBuffer;
use crate::string::{get_string, get_string_with_padding, get_varint_string};

pub const POLYLINE_CLOSED: u32 = 0x01;

/// Fields shared by every geometry record, read before the type specific part.
#[derive(Debug, serde::Serialize)]
pub struct Header {
    pub layer_id: u8,
    pub unknown1: u8,
    pub attributes: Vec<u8>,
    pub lower_bound: Coordinate,
    pub upper_bound: Coordinate,
    pub unknown2: u32,
    /// Feature code, only present on geometries stored as object 22.
    pub code: Option<String>,
}

impl Header {
    /// Returns the geometry type, the anchor coordinate and the header.
    fn read(buffer: &mut impl ReadBuffer, has_code: bool) -> Option<(u8, Coordinate, Self)> {
        let layer_id = buffer.get_u8()?;
        let geometry_type = buffer.get_u8()?;
        let unknown1 = buffer.get_u8()?;
        let anchor = Coordinate::from_buffer_with_elevation(buffer)?;
        let attributes = buffer.get_slice(18)?;
        let lower_bound = Coordinate::from_buffer_2d(buffer)?;
        let upper_bound = Coordinate::from_buffer_2d(buffer)?;
        let unknown2 = buffer.get_u32()?;
        let code = if has_code {
            Some(get_string_with_padding(buffer, 27)?)
        } else {
            None
        };

        let header = Self {
            layer_id,
            unknown1,
            attributes,
            lower_bound,
            upper_bound,
            unknown2,
            code,
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
        line_type: u8,
        line_type2: u8,
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
        unknown3: u8,
        string: String,
    },
    /// Bit 0x01 of `flags` marks a closed polyline, see [`POLYLINE_CLOSED`].
    PolyLine {
        #[serde(flatten)]
        header: Header,
        name: String,
        id: u32,
        flags: u32,
        unknown3: Vec<u8>,
        points: Vec<Coordinate>,
    },
    /// Probably a reference to a symbol in one of the libraries listed in [`crate::object::Object::Libraries`].
    Symbol {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        scale: f32,
        unknown3: u32,
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
    Image {
        #[serde(flatten)]
        header: Header,
        position: Coordinate,
        unknown3: Vec<u8>,
        properties: Vec<ImageProperty>,
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
                let line_type = header.attributes[0];
                let line_type2 = header.attributes[1];
                let mut end = Coordinate::from_buffer_2d(buffer)?;
                end.z = buffer.get_f32()?.into();

                Some(Self::Line {
                    header,
                    begin: anchor,
                    line_type,
                    line_type2,
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
                let unknown3 = buffer.get_slice(10)?;
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
                let spacing = f32::from_le_bytes(header.attributes[14..18].try_into().ok()?);
                let size = buffer.get_f32()?;
                let tilt = buffer.get_f32()?;
                let snap = buffer.get_u8()?.into();
                let flags = buffer.get_u8()?;
                let italic = (flags & 0x01) != 0;
                let underline = (flags & 0x02) != 0;
                let highlight = (flags & 0x04) != 0;

                let unknown3 = buffer.get_u8()?;
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
                    unknown3,
                    string,
                })
            }
            6 => {
                let scale = buffer.get_f32()?;
                let unknown3 = buffer.get_u32()?;
                let symbol = buffer.get_u16()?;
                let unknown4 = buffer.get_u8()?;

                Some(Self::Symbol {
                    header,
                    position: anchor,
                    scale,
                    unknown3,
                    symbol,
                    unknown4,
                })
            }
            7 => {
                let name = get_string_with_padding(buffer, 10)?;
                let id = buffer.get_u32()?;
                let flags = buffer.get_u32()?;
                let unknown3 = buffer.get_slice(8)?;

                let mut points = vec![];
                while let Some(c) = Coordinate::from_buffer_3d(buffer) {
                    points.push(c);
                }

                Some(Self::PolyLine {
                    header,
                    name,
                    id,
                    flags,
                    unknown3,
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
                let unknown3 = buffer.get_slice(53)?;
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
                    unknown3,
                    properties,
                })
            }
            id => Some(Self::Other {
                header,
                id,
                bytes: buffer.to_vec(),
            }),
        }
    }
}
