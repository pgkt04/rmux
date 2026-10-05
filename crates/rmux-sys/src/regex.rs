// Ported from tmux format.c, regsub.c, window-copy.c, window.c, compat.h @ 8f25579c
//! Safe owner of a POSIX `regex_t` for the format, regsub, search and
//! find-window callers.

use std::mem::MaybeUninit;
use std::ops::{BitOr, BitOrAssign, Range};
use std::ptr;

use crate::cstring::{cstr, nul_terminated};

/// `regcomp(3)` flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegexFlags(i32);

impl RegexFlags {
    pub const NONE: Self = Self(0);
    pub const EXTENDED: Self = Self(libc::REG_EXTENDED);
    pub const ICASE: Self = Self(libc::REG_ICASE);
    pub const NOSUB: Self = Self(libc::REG_NOSUB);
    pub const NEWLINE: Self = Self(libc::REG_NEWLINE);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for RegexFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for RegexFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// `regexec(3)` flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExecFlags(i32);

impl ExecFlags {
    pub const NONE: Self = Self(0);
    pub const NOTBOL: Self = Self(libc::REG_NOTBOL);
    pub const NOTEOL: Self = Self(libc::REG_NOTEOL);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for ExecFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for ExecFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A `regcomp`/`regexec` failure: the libc code and the copied `regerror` text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexError {
    pub code: i32,
    pub message: Vec<u8>,
}

/// Caller-owned, reusable capture storage. `ranges.len()` is the `nmatch`
/// passed to `regexec`; slot zero is the whole match and unmatched slots are
/// `None`. tmux uses ten slots (`regsub.c:65`), one (`window-copy.c:4136`) or
/// none (`format.c:5113`).
#[derive(Clone, Default)]
pub struct RegexMatch {
    pub ranges: Vec<Option<Range<usize>>>,
    subject: Vec<u8>,
    pmatch: Vec<libc::regmatch_t>,
}

impl std::fmt::Debug for RegexMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegexMatch")
            .field("ranges", &self.ranges)
            .finish_non_exhaustive()
    }
}

impl RegexMatch {
    /// The ten slots of `regmatch_t m[10]`.
    pub const SLOTS: usize = 10;

    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self {
            ranges: vec![None; slots],
            subject: Vec::new(),
            pmatch: Vec::new(),
        }
    }

    /// Storage with [`Self::SLOTS`] capture slots.
    #[must_use]
    pub fn with_ten_slots() -> Self {
        Self::new(Self::SLOTS)
    }

    /// Full-match range, if slot zero exists and matched.
    #[must_use]
    pub fn whole(&self) -> Option<Range<usize>> {
        self.ranges.first().cloned().flatten()
    }
}

/// A compiled POSIX regular expression. Only successful compilations are
/// constructed, so `Drop` always releases a valid `regex_t`.
pub struct PosixRegex {
    regex: libc::regex_t,
}

impl std::fmt::Debug for PosixRegex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PosixRegex").finish_non_exhaustive()
    }
}

impl PosixRegex {
    /// Compile `pattern` (up to its first NUL) with `flags`.
    pub fn new(pattern: &[u8], flags: RegexFlags) -> Result<Self, RegexError> {
        let pattern = nul_terminated(pattern);
        let mut regex = MaybeUninit::<libc::regex_t>::zeroed();
        // SAFETY: `pattern` is NUL-terminated and outlives the call; `regex`
        // is writable storage for one `regex_t`.
        let code = unsafe { libc::regcomp(regex.as_mut_ptr(), pattern.as_ptr().cast(), flags.0) };
        if code != 0 {
            // A failed regcomp leaves `regex` unspecified; it is never regfreed.
            return Err(error(code, regex.as_ptr()));
        }
        // SAFETY: regcomp returned 0, so `regex` is fully initialised.
        let regex = unsafe { regex.assume_init() };
        Ok(Self { regex })
    }

    /// Execute against `s` (up to its first NUL), filling `matches.ranges`.
    /// Returns `Ok(false)` on `REG_NOMATCH`, `Err` on any other failure.
    pub fn exec(
        &self,
        s: &[u8],
        matches: &mut RegexMatch,
        flags: ExecFlags,
    ) -> Result<bool, RegexError> {
        matches.subject.clear();
        matches.subject.extend_from_slice(cstr(s));
        matches.subject.push(0);

        let nmatch = matches.ranges.len();
        matches.pmatch.resize(
            nmatch,
            libc::regmatch_t {
                rm_so: -1,
                rm_eo: -1,
            },
        );
        let pmatch = if nmatch == 0 {
            ptr::null_mut()
        } else {
            matches.pmatch.as_mut_ptr()
        };
        // SAFETY: `self.regex` is a compiled regex; the subject is
        // NUL-terminated; `pmatch` is null or points to `nmatch` writable
        // `regmatch_t` elements that live for the call.
        let code = unsafe {
            libc::regexec(
                &raw const self.regex,
                matches.subject.as_ptr().cast(),
                nmatch,
                pmatch,
                flags.0,
            )
        };
        if code == libc::REG_NOMATCH {
            return Ok(false);
        }
        if code != 0 {
            return Err(error(code, &raw const self.regex));
        }
        for (range, m) in matches.ranges.iter_mut().zip(&matches.pmatch) {
            *range = match (usize::try_from(m.rm_so), usize::try_from(m.rm_eo)) {
                (Ok(start), Ok(end)) => Some(start..end),
                _ => None,
            };
        }
        Ok(true)
    }
}

impl Drop for PosixRegex {
    fn drop(&mut self) {
        // SAFETY: `self.regex` came from a successful regcomp and is freed once.
        unsafe { libc::regfree(&raw mut self.regex) };
    }
}

fn error(code: i32, regex: *const libc::regex_t) -> RegexError {
    // SAFETY: a null buffer with size 0 only queries the required length.
    let needed = unsafe { libc::regerror(code, regex, ptr::null_mut(), 0) };
    let mut message = vec![0u8; needed];
    if needed > 0 {
        // SAFETY: `message` holds `needed` writable bytes, the size regerror
        // itself reported for this code.
        let written = unsafe { libc::regerror(code, regex, message.as_mut_ptr().cast(), needed) };
        message.truncate(written.min(needed).saturating_sub(1));
    }
    RegexError { code, message }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_captures_preserve_unmatched_slots() {
        let re = PosixRegex::new(b"(a)(b)?(c)", RegexFlags::EXTENDED).unwrap();
        let mut m = RegexMatch::with_ten_slots();
        assert!(re.exec(b"xac", &mut m, ExecFlags::NONE).unwrap());
        assert_eq!(m.ranges.len(), 10);
        assert_eq!(m.ranges[0], Some(1..3));
        assert_eq!(m.ranges[1], Some(1..2));
        assert_eq!(m.ranges[2], None);
        assert_eq!(m.ranges[3], Some(2..3));
        assert!(m.ranges[4..].iter().all(Option::is_none));
        assert_eq!(m.whole(), Some(1..3));
    }

    #[test]
    fn failed_compilation_returns_error_without_crash() {
        let err = PosixRegex::new(b"(", RegexFlags::EXTENDED).unwrap_err();
        assert_ne!(err.code, 0);
        assert!(!err.message.is_empty());
        // A second failure and a success afterwards must not disturb anything.
        assert!(PosixRegex::new(b"[", RegexFlags::NONE).is_err());
        drop(PosixRegex::new(b"a", RegexFlags::NONE).unwrap());
    }

    #[test]
    fn exec_reuses_match_storage() {
        let re = PosixRegex::new(b"(b+)", RegexFlags::EXTENDED).unwrap();
        let mut m = RegexMatch::new(2);
        assert!(re.exec(b"abbc", &mut m, ExecFlags::NONE).unwrap());
        assert_eq!(m.ranges, vec![Some(1..3), Some(1..3)]);
        let cap = m.ranges.capacity();
        assert!(!re.exec(b"xyz", &mut m, ExecFlags::NONE).unwrap());
        assert_eq!(m.ranges.len(), 2);
        assert!(re.exec(b"b", &mut m, ExecFlags::NONE).unwrap());
        assert_eq!(m.ranges, vec![Some(0..1), Some(0..1)]);
        assert_eq!(m.ranges.capacity(), cap);
    }

    #[test]
    fn zero_slots_match_without_captures() {
        let re = PosixRegex::new(b"a.c", RegexFlags::EXTENDED | RegexFlags::NOSUB).unwrap();
        let mut m = RegexMatch::new(0);
        assert!(re.exec(b"abc", &mut m, ExecFlags::NONE).unwrap());
        assert!(!re.exec(b"ab", &mut m, ExecFlags::NONE).unwrap());
        assert!(m.ranges.is_empty());
    }

    #[test]
    fn icase_and_notbol() {
        let re = PosixRegex::new(b"^ABC", RegexFlags::EXTENDED | RegexFlags::ICASE).unwrap();
        let mut m = RegexMatch::new(1);
        assert!(re.exec(b"abcd", &mut m, ExecFlags::NONE).unwrap());
        assert!(!re.exec(b"abcd", &mut m, ExecFlags::NOTBOL).unwrap());
        let plain = PosixRegex::new(b"^ABC", RegexFlags::EXTENDED).unwrap();
        assert!(!plain.exec(b"abcd", &mut m, ExecFlags::NONE).unwrap());
    }

    #[test]
    fn nul_in_subject_and_pattern_stops_matching() {
        let re = PosixRegex::new(b"abc", RegexFlags::EXTENDED).unwrap();
        let mut m = RegexMatch::new(1);
        assert!(!re.exec(b"ab\0c", &mut m, ExecFlags::NONE).unwrap());
        assert!(re.exec(b"abc\0zzz", &mut m, ExecFlags::NONE).unwrap());
        assert_eq!(m.ranges[0], Some(0..3));
        let truncated = PosixRegex::new(b"ab\0c", RegexFlags::EXTENDED).unwrap();
        assert!(truncated.exec(b"ab", &mut m, ExecFlags::NONE).unwrap());
    }

    #[test]
    fn flags_combine() {
        let f = RegexFlags::EXTENDED | RegexFlags::ICASE;
        assert!(f.contains(RegexFlags::ICASE));
        assert!(!f.contains(RegexFlags::NOSUB));
        let mut e = ExecFlags::NONE;
        e |= ExecFlags::NOTEOL;
        assert!(e.contains(ExecFlags::NOTEOL));
        assert!(!e.contains(ExecFlags::NOTBOL));
    }
}
