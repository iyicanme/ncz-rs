use crate::object::Object;
use bopper::{BigEndianReadBuffer, ReadBuffer};

pub mod coordinate;
#[cfg(feature = "dxf")]
pub mod dxf_export;
pub mod geometry;
pub mod line_type;
pub mod object;
pub mod string;
#[cfg(feature = "dxf")]
pub mod summary;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(feature = "dxf")]
pub use dxf_export::objects_to_dxf;

/// An object whose decoder failed. Its bytes are skipped.
#[derive(Debug, serde::Serialize)]
pub struct ParseFailure {
    /// Offset of the object header in the file.
    pub offset: usize,
    pub object_type_id: u8,
    /// The declared body length, or `None` when the header itself is cut off.
    pub length: Option<u32>,
}

#[derive(Debug)]
pub enum ParseError {
    /// The file does not start with a file header, as every NCZ file in the corpus does.
    NotNcz,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotNcz => f.write_str("not an NCZ file"),
        }
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, serde::Serialize)]
pub struct ParsedFile {
    pub objects: Vec<Object>,
    pub failures: Vec<ParseFailure>,
}

pub fn parse_objects(bytes: &[u8]) -> Result<ParsedFile, ParseError> {
    let mut buffer = BigEndianReadBuffer::new(bytes);

    // Rejects files of zero bytes and other files that are not NCZ before they turn into a stream of garbage objects.
    let Some(header @ Object::FileHeader { .. }) = Object::from_buffer(&mut buffer) else {
        return Err(ParseError::NotNcz);
    };

    let mut objects = vec![header];
    let mut failures = vec![];
    while !buffer.is_empty() {
        let offset = buffer.offset();
        let Some(object) = Object::from_buffer(&mut buffer) else {
            let mut header = BigEndianReadBuffer::new(&bytes[offset..]);
            failures.push(ParseFailure {
                offset,
                object_type_id: header.get_u8().unwrap_or_default(),
                length: header.get_u32(),
            });
            continue;
        };

        objects.push(object);
    }

    Ok(ParsedFile { objects, failures })
}
