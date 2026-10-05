// Ported from tmux compat.h, format.c, options.c, environ.c, cmd-find.c, tty-term.c, window.c @ 8f25579c
//! `fnmatch(3)` bridge.

use std::ops::{BitOr, BitOrAssign};

use crate::cstring::nul_terminated;

/// `fnmatch(3)` flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FnmatchFlags(i32);

impl FnmatchFlags {
    pub const NONE: Self = Self(0);
    // compat.h:217-223 falls back to FNM_IGNORECASE, then 0, only when the
    // libc lacks FNM_CASEFOLD; macOS and glibc both define it.
    pub const CASEFOLD: Self = Self(libc::FNM_CASEFOLD);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for FnmatchFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for FnmatchFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// `fnmatch(pattern, text, flags) == 0`; both inputs stop at their first NUL.
#[must_use]
pub fn fnmatch(pattern: &[u8], text: &[u8], flags: FnmatchFlags) -> bool {
    let pattern = nul_terminated(pattern);
    let text = nul_terminated(text);
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    unsafe { libc::fnmatch(pattern.as_ptr().cast(), text.as_ptr().cast(), flags.0) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casefold() {
        assert!(fnmatch(b"A*", b"abc", FnmatchFlags::CASEFOLD));
        assert!(!fnmatch(b"A*", b"abc", FnmatchFlags::NONE));
        assert!(fnmatch(b"a*", b"abc", FnmatchFlags::NONE));
    }

    #[test]
    fn invalid_pattern_does_not_match() {
        assert!(!fnmatch(b"[", b"[", FnmatchFlags::NONE));
        assert!(!fnmatch(b"[", b"a", FnmatchFlags::CASEFOLD));
    }

    #[test]
    fn nul_truncates() {
        assert!(fnmatch(b"ab\0zz", b"ab\0yy", FnmatchFlags::NONE));
        assert!(!fnmatch(b"ab", b"", FnmatchFlags::NONE));
        assert!(fnmatch(b"", b"", FnmatchFlags::NONE));
    }

    #[test]
    fn flags_combine() {
        let mut f = FnmatchFlags::NONE;
        f |= FnmatchFlags::CASEFOLD;
        assert!(f.contains(FnmatchFlags::CASEFOLD));
        assert!(!FnmatchFlags::NONE.contains(FnmatchFlags::CASEFOLD));
    }
}
