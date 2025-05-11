#[derive(Debug, serde::Serialize)]
pub struct LineType {
    pub unknown1: Vec<u8>,
    pub name: String,
    pub unknown2: Vec<u8>,
}
