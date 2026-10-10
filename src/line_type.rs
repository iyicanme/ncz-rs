use crate::string::get_string_with_padding;
use bopper::ReadBuffer;

const MAX_SEGMENTS: usize = 28;

#[derive(Debug, serde::Serialize)]
pub struct LineType {
    pub description: String,
    pub name: String,
    /// Negative counts are used by line types that also set `offset` and `unknown1`.
    pub segment_count: i8,
    /// Equals the sum of the dash and gap lengths for most line types.
    pub pattern_length: f32,
    pub offset: f32,
    pub unknown1: u16,
    /// Positive values are dashes and negative values are gaps. Values above 1000 seem to refer to symbols.
    pub segments: Vec<f32>,
    pub unknown2: Vec<u8>,
}

impl LineType {
    pub fn read(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let description = get_string_with_padding(buffer, 81)?;
        let name = get_string_with_padding(buffer, 10)?;
        let segment_count = buffer.get_u8()? as i8;
        let pattern_length = buffer.get_f32()?;
        let offset = buffer.get_f32()?;
        let unknown1 = buffer.get_u16()?;

        let mut segments = (0..MAX_SEGMENTS)
            .map(|_| buffer.get_f32())
            .collect::<Option<Vec<_>>>()?;
        // Unused slots can hold stale data, and a few slots hold garbage with an impossible count.
        segments.truncate(usize::from(segment_count.unsigned_abs()));

        let unknown2 = buffer.get_slice(3)?;

        let line_type = LineType {
            description,
            name,
            segment_count,
            pattern_length,
            offset,
            unknown1,
            segments,
            unknown2,
        };

        Some(line_type)
    }
}
