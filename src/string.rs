use bopper::ReadBuffer;
use encoding_rs::WINDOWS_1254;

pub fn get_string(buffer: &mut impl ReadBuffer) -> Option<String> {
    let string_length = buffer.get_u8()?.into();
    let string_bytes = buffer.get_slice(string_length)?;

    let string = WINDOWS_1254.decode(&string_bytes).0.to_string();
    Some(string)
}

pub fn get_string_with_padding(
    buffer: &mut impl ReadBuffer,
    total_length: usize,
) -> Option<String> {
    let string_length: usize = buffer.get_u8()?.into();
    let string_bytes = buffer.get_buffer(total_length)?.to_vec();
    // Some files hold stale data in padded fields, so a length past the field is clamped.
    let actual_bytes = &string_bytes[..string_length.min(total_length)];

    let string = WINDOWS_1254.decode(actual_bytes).0.to_string();
    Some(string)
}

pub fn get_varint_string(buffer: &mut impl ReadBuffer) -> Option<String> {
    let mut string_length = 0usize;
    let mut shift = 0;
    loop {
        let byte = buffer.get_u8()?;
        string_length |= usize::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            break;
        }
    }
    let string_bytes = buffer.get_slice(string_length)?;

    Some(String::from_utf8_lossy(&string_bytes).into_owned())
}
