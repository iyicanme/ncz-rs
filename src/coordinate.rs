use bopper::ReadBuffer;

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Coordinate {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Coordinate {
    pub fn from_buffer_2d(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let x = buffer.get_f64()?;
        let y = buffer.get_f64()?;

        Some(Self { x, y, z: 0.0 })
    }

    pub fn from_buffer_3d(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let x = buffer.get_f64()?;
        let y = buffer.get_f64()?;
        let z = buffer.get_f64()?;

        Some(Self { x, y, z })
    }

    /// Reads a 2D coordinate followed by an `f32` elevation and 4 bytes of padding.
    pub fn from_buffer_with_elevation(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let x = buffer.get_f64()?;
        let y = buffer.get_f64()?;
        let z = buffer.get_f32()?.into();
        buffer.get_slice(4)?;

        Some(Self { x, y, z })
    }
}
