// Ported from tmux format.c @ 8f25579c
//! NUL-terminated copies for libc string arguments.

/// The bytes before the first NUL, or the whole slice.
pub(crate) fn cstr(bytes: &[u8]) -> &[u8] {
    bytes
        .iter()
        .position(|&b| b == 0)
        .map_or(bytes, |end| &bytes[..end])
}

/// A copy of [`cstr`] with a trailing NUL.
pub(crate) fn nul_terminated(bytes: &[u8]) -> Vec<u8> {
    let bytes = cstr(bytes);
    let mut copy = Vec::with_capacity(bytes.len() + 1);
    copy.extend_from_slice(bytes);
    copy.push(0);
    copy
}

/// The bytes of a NUL-terminated C string copied out of `ptr`.
///
/// # Safety
/// `ptr` must point to a readable NUL-terminated string.
pub(crate) unsafe fn copy_cstr(ptr: *const libc::c_char) -> Vec<u8> {
    // SAFETY: the caller guarantees a readable NUL-terminated string.
    unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes().to_vec()
}
