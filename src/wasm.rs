use std::sync::atomic::{AtomicUsize, Ordering};

static RESULT_LENGTH: AtomicUsize = AtomicUsize::new(0);

/// Allocates `length` bytes for the host to copy the NCZ file into.
#[unsafe(no_mangle)]
pub extern "C" fn nczrs_alloc(length: usize) -> *mut u8 {
    let mut bytes = Vec::<u8>::with_capacity(length);
    let pointer = bytes.as_mut_ptr();
    core::mem::forget(bytes);

    pointer
}

/// Parses the NCZ file in `pointer[..length]` and returns a pointer to its objects as JSON.
/// The JSON length is available from `nczrs_result_length`.
///
/// # Safety
/// `pointer` must point to `length` initialized bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nczrs_parse(pointer: *const u8, length: usize) -> *mut u8 {
    let bytes = unsafe { core::slice::from_raw_parts(pointer, length) };

    let objects = crate::parse_objects(bytes);
    let serialized = serde_json::to_vec(&objects).unwrap_or_default();

    let mut serialized = serialized.into_boxed_slice();
    RESULT_LENGTH.store(serialized.len(), Ordering::Relaxed);
    let result = serialized.as_mut_ptr();
    core::mem::forget(serialized);

    result
}

#[unsafe(no_mangle)]
pub extern "C" fn nczrs_result_length() -> usize {
    RESULT_LENGTH.load(Ordering::Relaxed)
}
