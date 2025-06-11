use crate::string::get_string_with_padding;
use bopper::ReadBuffer;

#[derive(Debug, serde::Serialize)]
pub struct LineType {
    pub unknown1: Vec<u8>,
    pub name: String,
    pub unknown2: Vec<u8>,
}

impl LineType {
    pub fn read(buffer: &mut impl ReadBuffer) -> Option<Self> {
        let unknown1 = buffer.get_slice(82)?;
        let name = get_string_with_padding(buffer, 10)?;
        let unknown2 = buffer.get_slice(126)?;

        let line_type = LineType {
            unknown1,
            name,
            unknown2,
        };

        Some(line_type)
    }
}
