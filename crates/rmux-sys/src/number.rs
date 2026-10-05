// Ported from tmux format.c, cmd-run-shell.c @ 8f25579c
//! `strtod` with its end offset and `%.*f` through libc `snprintf`.

use std::io;
use std::ptr;

use crate::cstring::nul_terminated;

/// `strtod(input, &end)`: the value and the number of consumed bytes. Empty
/// input consumes zero bytes and still satisfies the C complete-consumption
/// check (`format.c:5876-5885`). `input` stops at its first NUL.
#[must_use]
pub fn strtod(input: &[u8]) -> (f64, usize) {
    let copy = nul_terminated(input);
    let start: *const libc::c_char = copy.as_ptr().cast();
    let mut end: *mut libc::c_char = ptr::null_mut();
    // SAFETY: `start` is NUL-terminated and outlives the call; `end` is a
    // writable pointer slot.
    let value = unsafe { libc::strtod(start, &raw mut end) };
    // SAFETY: strtod sets `end` to a position inside `copy`, at or after
    // `start`.
    let consumed = unsafe { end.cast_const().offset_from(start) };
    (value, usize::try_from(consumed).unwrap_or(0))
}

/// Append `snprintf("%.*f", precision, value)` to `dst`.
pub fn printf_fixed(dst: &mut Vec<u8>, precision: i32, value: f64) -> io::Result<()> {
    // SAFETY: a null buffer with size 0 only measures the output.
    let needed = unsafe { libc::snprintf(ptr::null_mut(), 0, c"%.*f".as_ptr(), precision, value) };
    let needed = usize::try_from(needed).map_err(|_| io::Error::last_os_error())?;
    let start = dst.len();
    dst.reserve(needed + 1);
    // SAFETY: `reserve` guarantees `needed + 1` writable bytes past `start`,
    // which covers the text and its NUL terminator.
    let written = unsafe {
        libc::snprintf(
            dst.as_mut_ptr().add(start).cast(),
            needed + 1,
            c"%.*f".as_ptr(),
            precision,
            value,
        )
    };
    if usize::try_from(written).ok() != Some(needed) {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: snprintf initialised `needed` bytes starting at `start`.
    unsafe { dst.set_len(start + needed) };
    Ok(())
}

/// Preserve the native C `(double)(long long)value` conversion, including
/// architecture-specific nonfinite and out-of-range results.
pub fn format_integer_operand(value: f64) -> f64 {
    #[cfg(target_arch = "aarch64")]
    {
        let integer: i64;
        // SAFETY: this register-only conversion has no memory or stack effects.
        unsafe {
            std::arch::asm!("fcvtzs {integer}, {value:d}", integer = out(reg) integer, value = in(vreg) value, options(nomem, nostack, pure));
        }
        integer as f64
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is guaranteed by the x86-64 ABI.
        unsafe { std::arch::x86_64::_mm_cvttsd_si64(std::arch::x86_64::_mm_set_sd(value)) as f64 }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        (value as i64) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strtod_end_offsets() {
        assert_eq!(strtod(b""), (0.0, 0));
        assert_eq!(strtod(b"1.5x"), (1.5, 3));
        assert_eq!(strtod(b"  -2e1"), (-20.0, 6));
        assert_eq!(strtod(b"abc"), (0.0, 0));
        assert_eq!(strtod(b"7\0 8"), (7.0, 1));
    }

    #[test]
    fn strtod_nonfinite() {
        let (inf, n) = strtod(b"inf");
        assert!(inf.is_infinite() && inf > 0.0);
        assert_eq!(n, 3);
        let (nan, n) = strtod(b"nan!");
        assert!(nan.is_nan());
        assert_eq!(n, 3);
        let (neg, n) = strtod(b"-Infinity");
        assert!(neg.is_infinite() && neg < 0.0);
        assert_eq!(n, 9);
    }

    fn c_fixed(precision: i32, value: f64) -> Vec<u8> {
        let mut buf = [0u8; 512];
        // SAFETY: `buf` is writable for its full length and the format
        // consumes exactly the two variadic arguments passed.
        let n = unsafe {
            libc::snprintf(
                buf.as_mut_ptr().cast(),
                buf.len(),
                c"%.*f".as_ptr(),
                precision,
                value,
            )
        };
        buf[..usize::try_from(n).unwrap()].to_vec()
    }

    #[test]
    fn printf_fixed_matches_snprintf() {
        for (precision, value) in [
            (2, 1.005),
            (0, 2.5),
            (0, 3.5),
            (6, -0.0),
            (3, 1e20),
            (-1, 1.5),
            (20, 0.1),
            (0, f64::INFINITY),
            (2, f64::NAN),
        ] {
            let mut out = b"x=".to_vec();
            printf_fixed(&mut out, precision, value).unwrap();
            let mut expected = b"x=".to_vec();
            expected.extend(c_fixed(precision, value));
            assert_eq!(out, expected, "%.{precision}f of {value}");
        }
        let mut out = Vec::new();
        printf_fixed(&mut out, 2, 1.005).unwrap();
        assert!(out == b"1.00" || out == b"1.01");
        assert_eq!(out, c_fixed(2, 1.005));
    }

    #[test]
    fn printf_fixed_appends_large_output() {
        let mut out = Vec::new();
        printf_fixed(&mut out, 100, 1e300).unwrap();
        assert_eq!(out, c_fixed(100, 1e300));
        assert_eq!(out.len(), 301 + 1 + 100);
    }
}
