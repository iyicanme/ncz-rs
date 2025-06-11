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
    let string_length = buffer.get_u8()?.into();
    let string_bytes = buffer.get_buffer(total_length)?.to_vec();
    let actual_bytes = &string_bytes[..string_length];

    let string = WINDOWS_1254.decode(&actual_bytes).0.to_string();
    Some(string)
}
