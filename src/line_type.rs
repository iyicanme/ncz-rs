use crate::string::get_string_with_padding;
use bopper::ReadBuffer;

const MAX_SEGMENTS: usize = 28;

#[derive(Debug, serde::Serialize)]
pub struct LineType {
    pub description: String,
    pub name: String,
    /// A negative count makes a multi-line, which also draws line type `unknown1` at `offset` to its right.
    pub segment_count: i8,
    /// Equals the sum of the dash and gap lengths for most line types, but not for all.
    pub pattern_length: f32,
    /// For a negative `segment_count`, the distance of the parallel line type to the right.
    pub offset: f32,
    /// For a negative `segment_count`, the index of the line type drawn parallel to this one.
    pub unknown1: u16,
    /// Positive values are dashes and negative values are gaps. `1000 + n` puts symbol `n` of the first library along
    /// the line, `-(1000 + n)` puts it turned by a half turn. Other thousands digits are of unknown meaning.
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
