use crate::coordinate::Coordinate;
use bopper::ReadBuffer;
use crate::string::get_string;

#[derive(Debug, serde::Serialize)]
pub enum Geometry {
    Point {
        layer_id: u8,
        position: Coordinate,
    },
    Line {
        layer_id: u8,
        unknown1: u8,
        begin: Coordinate,
        line_type: u8,
        line_type2: u8,
        unknown2: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown3: Vec<u8>,
        end: Coordinate,
        unknown4: Vec<u8>,
    },
    Circle {
        layer_id: u8,
        unknown1: u8,
        center: Coordinate,
        unknown2: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown3: u32,
        radius: f64,
    },
    Arc {
        layer_id: u8,
        unknown1: u8,
        center: Coordinate,
        unknown2: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown3: u32,
        radius: f64,
        unknown4: Vec<u8>,
        start_angle: f64,
        end_angle: f64,
    },
    Text {
        layer_id: u8,
        unknown1: u8,
        position: Coordinate,
        unknown2: Vec<u8>,
        spacing: f32,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown3: Vec<u8>,
        size: f32,
        tilt: f32,
        snap: u8,
        italic: bool,
        underline: bool,
        highlight: bool,
        unknown4: u8,
        string: String,
    },
    Curve {
        layer_id: u8,
        unknown1: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown2: Vec<u8>,
        unknown3: Vec<u8>,
        points: Vec<Coordinate>,
    },
    PolyLine {
        layer_id: u8,
        unknown1: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown2: Vec<u8>,
        unknown3: Vec<u8>,
        points: Vec<Coordinate>,
    },
    SmoothCurve {
        layer_id: u8,
        unknown1: Vec<u8>,
        lower_bound: Coordinate,
        upper_bound: Coordinate,
        unknown2: Vec<u8>,
        unknown3: Vec<u8>,
        points: Vec<Coordinate>,
    },
    Other {
        layer_id: u8,
        id: u8,
        bytes: Vec<u8>,
    },
}

impl Geometry {
    pub fn from_buffer(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let layer_id = buffer.get_u8()?;
        let geometry_type = buffer.get_u8()?;

        match geometry_type {
            1 => {
                let coordinate = Coordinate::from_buffer_3d(buffer)?;

                Some(Self::Point {
                    layer_id,
                    position: coordinate,
                })
            }
            2 => {
                let unknown1 = buffer.get_u8()?;
                let begin = Coordinate::from_buffer_3d(buffer)?;
                let line_type = buffer.get_u8()?;
                let line_type2 = buffer.get_u8()?;
                let unknown2 = buffer.get_slice(16)?;
                let lower_bound = Coordinate::from_buffer_2d(buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(buffer)?;
                let unknown3 = buffer.get_slice(4)?;
                let end = Coordinate::from_buffer_2d(buffer)?;
                let unknown4 = buffer.get_slice(4)?;

                Some(Self::Line {
                    layer_id,
                    unknown1,
                    begin,
                    line_type,
                    line_type2,
                    unknown2,
                    lower_bound,
                    upper_bound,
                    unknown3,
                    end,
                    unknown4,
                })
            }
            3 => {
                let unknown1 = buffer.get_u8()?;
                let center = Coordinate::from_buffer_3d(buffer)?;
                let unknown2 = buffer.get_slice(18)?;
                let lower_bound = Coordinate::from_buffer_2d(buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(buffer)?;
                let unknown3 = buffer.get_u32()?;
                let radius = buffer.get_f64()?;

                Some(Self::Circle {
                    layer_id,
                    unknown1,
                    center,
                    unknown2,
                    lower_bound,
                    upper_bound,
                    unknown3,
                    radius,
                })
            }
            4 => {
                let unknown1 = buffer.get_u8()?;
                let center = Coordinate::from_buffer_3d(buffer)?;
                let unknown2 = buffer.get_slice(18)?;
                let lower_bound = Coordinate::from_buffer_2d(buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(buffer)?;
                let unknown3 = buffer.get_u32()?;
                let radius = buffer.get_f64()?;
                let unknown4 = buffer.get_slice(10)?;
                let start_angle = buffer.get_f64()?;
                let end_angle = buffer.get_f64()?;

                Some(Self::Arc {
                    layer_id,
                    unknown1,
                    center,
                    unknown2,
                    lower_bound,
                    upper_bound,
                    unknown3,
                    radius,
                    unknown4,
                    start_angle,
                    end_angle,
                })
            }
            5 => {
                let unknown1 = buffer.get_u8()?;
                let position = Coordinate::from_buffer_3d(buffer)?;
                let unknown2 = buffer.get_slice(14)?;
                let spacing = buffer.get_f32()?;
                let lower_bound = Coordinate::from_buffer_2d(buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(buffer)?;
                let unknown3 = buffer.get_slice(4)?;
                let size = buffer.get_f32()?;
                let tilt = buffer.get_f32()?;
                let snap = buffer.get_u8()?;
                let flags = buffer.get_u8()?;
                let italic = (flags & 0x01) != 0;
                let underline = (flags & 0x02) != 0;
                let highlight = (flags & 0x04) != 0;

                let unknown4 = buffer.get_u8()?;
                let string = get_string(buffer)?;

                Some(Geometry::Text {
                    layer_id,
                    unknown1,
                    position,
                    unknown2,
                    spacing,
                    lower_bound,
                    upper_bound,
                    unknown3,
                    size,
                    tilt,
                    snap,
                    italic,
                    underline,
                    highlight,
                    unknown4,
                    string,
                })
            }
            7 => {
                let unknown1 = buffer.get_slice(43)?;
                let lower_bound = Coordinate::from_buffer_2d(buffer)?;
                let upper_bound = Coordinate::from_buffer_2d(buffer)?;
                let unknown2 = buffer.get_slice(19)?;
                let polyline_type = buffer.get_u8()?;
                let unknown3 = buffer.get_slice(11)?;

                let mut points = vec![];
                while let Some(c) = Coordinate::from_buffer_3d(buffer) {
                    points.push(c);
                }

                match polyline_type {
                    4 => Some(Self::Curve {
                        layer_id,
                        unknown1,
                        lower_bound,
                        upper_bound,
                        unknown2,
                        unknown3,
                        points,
                    }),
                    16 => Some(Self::PolyLine {
                        layer_id,
                        unknown1,
                        lower_bound,
                        upper_bound,
                        unknown2,
                        unknown3,
                        points,
                    }),
                    64 => Some(Self::SmoothCurve {
                        layer_id,
                        unknown1,
                        lower_bound,
                        upper_bound,
                        unknown2,
                        unknown3,
                        points,
                    }),
                    id => Some(Self::Other {
                        layer_id,
                        id,
                        bytes: vec![],
                    }),
                }
            }
            id => Some(Self::Other {
                layer_id,
                id,
                bytes: buffer.to_vec(),
            }),
        }
    }
}
