// Ported from tmux log.c, tmux.c @ 8f25579c

use std::{ffi::CString, io};

/// Current thread `errno`.
pub fn errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// `strerror(error)` as a copied byte string (`log.c:145`).
pub fn strerror(error: i32) -> Vec<u8> {
    let mut buf = [0u8; 1024];
    // SAFETY: strerror_r writes a NUL-terminated string within buf.len() bytes
    // and returns a status code; the buffer is a valid, exclusively owned
    // writable region of that size.
    unsafe {
        libc::strerror_r(error, buf.as_mut_ptr().cast(), buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    buf[..end].to_vec()
}

/// `access(path, X_OK) == 0` (`tmux.c:104`); the path is read up to its first NUL.
pub fn access_executable(path: &[u8]) -> bool {
    let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
    let Ok(path) = CString::new(&path[..end]) else {
        return false;
    };
    // SAFETY: path is a valid NUL-terminated C string for the duration of the call.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}
