// Ported from tmux format.c, window-clock.c @ 8f25579c
//! `localtime_r`, `ctime_r` and `strftime` bridges that keep the libc locale
//! and timezone.

use std::mem::MaybeUninit;

use crate::cstring::nul_terminated;

/// `ctime_r` needs at least this many bytes.
pub const CTIME_BUFFER: usize = 26;

/// An owned copy of a broken-down `struct tm`.
#[derive(Clone, Copy)]
pub struct LocalTime {
    tm: libc::tm,
}

impl std::fmt::Debug for LocalTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalTime")
            .field("year", &self.tm.tm_year)
            .field("mon", &self.tm.tm_mon)
            .field("mday", &self.tm.tm_mday)
            .field("hour", &self.tm.tm_hour)
            .field("min", &self.tm.tm_min)
            .field("sec", &self.tm.tm_sec)
            .finish_non_exhaustive()
    }
}

impl LocalTime {
    pub const fn sec(&self) -> i32 {
        self.tm.tm_sec
    }
    pub const fn min(&self) -> i32 {
        self.tm.tm_min
    }
    pub const fn hour(&self) -> i32 {
        self.tm.tm_hour
    }
    pub const fn mday(&self) -> i32 {
        self.tm.tm_mday
    }
    pub const fn mon(&self) -> i32 {
        self.tm.tm_mon
    }
    pub const fn year(&self) -> i32 {
        self.tm.tm_year
    }
    pub const fn wday(&self) -> i32 {
        self.tm.tm_wday
    }
    pub const fn yday(&self) -> i32 {
        self.tm.tm_yday
    }
    pub const fn isdst(&self) -> i32 {
        self.tm.tm_isdst
    }
}

/// `localtime_r(&seconds, &tm)`; `None` when libc fails or `seconds` does
/// not fit `time_t`.
#[must_use]
pub fn localtime(seconds: i64) -> Option<LocalTime> {
    let t = libc::time_t::try_from(seconds).ok()?;
    let mut tm = MaybeUninit::<libc::tm>::zeroed();
    // SAFETY: `t` and `tm` are valid for the call; libc fills `tm` or
    // returns null.
    let result = unsafe { libc::localtime_r(&raw const t, tm.as_mut_ptr()) };
    if result.is_null() {
        return None;
    }
    // SAFETY: localtime_r returned non-null, so `tm` is initialised.
    Some(LocalTime {
        tm: unsafe { tm.assume_init() },
    })
}

/// `ctime_r(&seconds, dst)`; `dst` must hold [`CTIME_BUFFER`] bytes. Returns
/// the text length including the trailing newline, for the caller to remove.
#[must_use]
pub fn ctime(seconds: i64, dst: &mut [u8]) -> Option<usize> {
    if dst.len() < CTIME_BUFFER {
        return None;
    }
    let t = libc::time_t::try_from(seconds).ok()?;
    // SAFETY: `dst` holds the 26 bytes ctime_r may write.
    let result = unsafe { libc::ctime_r(&raw const t, dst.as_mut_ptr().cast()) };
    if result.is_null() {
        return None;
    }
    dst.iter().position(|&b| b == 0)
}

/// `strftime(dst, dst.len(), format, tm)`; zero when the result does not fit,
/// exactly like libc. `format` stops at its first NUL.
#[must_use]
pub fn strftime(dst: &mut [u8], format: &[u8], time: &LocalTime) -> usize {
    let format = nul_terminated(format);
    // SAFETY: `dst` is writable for `dst.len()` bytes, `format` is
    // NUL-terminated and `time.tm` is a complete `struct tm`.
    unsafe {
        libc::strftime(
            dst.as_mut_ptr().cast(),
            dst.len(),
            format.as_ptr().cast(),
            &raw const time.tm,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    const CHILD_ENV: &str = "RMUX_SYS_TIME_UTC_CHILD";
    const YEAR: i64 = 86400 * 365;

    unsafe extern "C" {
        fn tzset();
    }

    fn in_utc_child(test: &str) -> bool {
        if std::env::var_os(CHILD_ENV).is_some() {
            // SAFETY: tzset has no preconditions.
            unsafe { tzset() };
            return true;
        }
        let status = Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, "1")
            .env("TZ", "UTC")
            .status()
            .unwrap();
        assert!(status.success(), "UTC child {test} failed");
        false
    }

    #[test]
    fn localtime_and_strftime_utc() {
        if !in_utc_child("time::tests::localtime_and_strftime_utc") {
            return;
        }
        let mut buf = [0u8; 64];
        let epoch = localtime(0).unwrap();
        let n = strftime(&mut buf, b"%Y-%m-%d %H:%M:%S %Z", &epoch);
        assert_eq!(&buf[..n], b"1970-01-01 00:00:00 UTC");
        let year = localtime(YEAR).unwrap();
        let n = strftime(&mut buf, b"%Y-%m-%d %H:%M:%S\0ignored", &year);
        assert_eq!(&buf[..n], b"1971-01-01 00:00:00");
        assert_eq!(strftime(&mut buf, b"", &year), 0);
    }

    #[test]
    fn strftime_overflow_returns_zero() {
        let time = localtime(0).unwrap();
        let mut small = [0xaau8; 3];
        assert_eq!(strftime(&mut small, b"%Y-%m-%d", &time), 0);
        let mut exact = [0u8; 5];
        assert_eq!(strftime(&mut exact, b"%Y", &time), 4);
        let mut empty = [0u8; 0];
        assert_eq!(strftime(&mut empty, b"%Y", &time), 0);
    }

    #[test]
    fn ctime_utc() {
        if !in_utc_child("time::tests::ctime_utc") {
            return;
        }
        let mut buf = [0u8; CTIME_BUFFER];
        let n = ctime(0, &mut buf).unwrap();
        assert_eq!(n, 25);
        assert_eq!(&buf[..n], b"Thu Jan  1 00:00:00 1970\n");
        let mut big = [0u8; 64];
        let n = ctime(YEAR, &mut big).unwrap();
        assert_eq!(&big[..n], b"Fri Jan  1 00:00:00 1971\n");
        assert_eq!(ctime(0, &mut [0u8; 25]), None);
    }

    #[test]
    fn localtime_rejects_out_of_range() {
        assert!(localtime(0).is_some());
        assert!(localtime(i64::MAX).is_none());
    }
}
