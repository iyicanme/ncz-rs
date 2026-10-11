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

/// Number of objects in a row that must decode before parsing resumes after a failure. Random bytes almost never
/// pass, since each object needs a known type and a length that fits the file.
const RESYNC_OBJECTS: usize = 8;

/// An object that could not be decoded, or that has a type no NCZ file uses, which means the file is corrupt from
/// there on. Parsing resumes at the next readable object.
#[derive(Debug, serde::Serialize)]
pub struct ParseFailure {
    /// Offset of the object header in the file.
    pub offset: usize,
    pub object_type_id: u8,
    /// The declared body length, or `None` when the header itself is cut off.
    pub length: Option<u32>,
    /// Number of bytes skipped from `offset` to the next readable object, or to the end of the file.
    pub skipped: usize,
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

/// Decodes the object at the start of the buffer, failing on types that no NCZ file uses.
fn read_object(buffer: &mut impl ReadBuffer) -> Option<Object> {
    match Object::from_buffer(buffer)? {
        Object::Unknown { id, .. } if !Object::is_known_type(id) => None,
        object => Some(object),
    }
}

/// Whether parsing can resume at `offset`: the next [`RESYNC_OBJECTS`] objects decode. Fewer objects up to the end of
/// the file are not enough, since a few bytes can pass for an empty object.
fn is_readable_at(bytes: &[u8], offset: usize) -> bool {
    let Some(rest) = bytes.get(offset..).filter(|rest| !rest.is_empty()) else {
        return false;
    };
    // Cheap checks first, since most offsets fail them.
    if !Object::is_known_type(rest[0]) {
        return false;
    }

    let mut buffer = BigEndianReadBuffer::new(rest);
    (0..RESYNC_OBJECTS).all(|_| read_object(&mut buffer).is_some())
}

/// The offset of the next readable object after a failure at `offset`. The end of the object, as its header
/// declares it, is tried first, then every following byte.
fn resync(bytes: &[u8], offset: usize, length: Option<u32>) -> Option<usize> {
    let declared_end =
        length.and_then(|length| offset.checked_add(5 + usize::try_from(length).ok()?));
    if let Some(end) = declared_end.filter(|&end| is_readable_at(bytes, end)) {
        return Some(end);
    }

    (offset + 1..bytes.len()).find(|&candidate| is_readable_at(bytes, candidate))
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
        let Some(object) = read_object(&mut buffer) else {
            let mut header = BigEndianReadBuffer::new(&bytes[offset..]);
            let object_type_id = header.get_u8().unwrap_or_default();
            let length = header.get_u32();
            let next = resync(bytes, offset, length).unwrap_or(bytes.len());
            failures.push(ParseFailure {
                offset,
                object_type_id,
                length,
                skipped: next - offset,
            });

            buffer = BigEndianReadBuffer::new(bytes);
            buffer.skip(next).expect("parsing resumes inside the file");
            continue;
        };

        objects.push(object);
    }

    Ok(ParsedFile { objects, failures })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(id: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![id];
        bytes.extend((body.len() as u32).to_le_bytes());
        bytes.extend(body);
        bytes
    }

    fn scale(value: f64) -> Vec<u8> {
        object(3, &value.to_le_bytes())
    }

    #[test]
    fn resumes_after_corrupt_bytes() {
        let mut bytes = object(1, &[0; 120]);
        bytes.extend(scale(1.0));
        // A type no NCZ file uses, with a length that runs past the end of the file.
        let garbage = [0, 0, 0, 0, 3, b'X', b'T', b'B', 0, 1, 0, 0];
        bytes.extend(garbage);
        for _ in 0..RESYNC_OBJECTS + 1 {
            bytes.extend(scale(2.0));
        }

        let parsed = parse_objects(&bytes).unwrap();

        assert_eq!(parsed.objects.len(), 2 + RESYNC_OBJECTS + 1);
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(parsed.failures[0].skipped, garbage.len());
    }

    #[test]
    fn skips_a_cut_off_object_at_the_end() {
        let mut bytes = object(1, &[0; 120]);
        bytes.extend(scale(1.0));
        bytes.extend(&scale(2.0)[..6]);

        let parsed = parse_objects(&bytes).unwrap();

        assert_eq!(parsed.objects.len(), 2);
        assert_eq!(parsed.failures[0].skipped, 6);
    }
}
