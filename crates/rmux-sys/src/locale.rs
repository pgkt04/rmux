// Ported from tmux tmux.c, utf8.c, compat/utf8proc.c @ 8f25579c

use std::ffi::CStr;
use std::fmt;

/// Startup locale failures (`tmux.c:445-452`), with the exact `errx` texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleError {
    InvalidLocale,
    NotUtf8(Vec<u8>),
}

impl LocaleError {
    /// The startup message as raw bytes (the codeset comes from libc).
    pub fn message(&self) -> Vec<u8> {
        match self {
            Self::InvalidLocale => b"invalid LC_ALL, LC_CTYPE or LANG".to_vec(),
            Self::NotUtf8(codeset) => {
                let mut msg = b"need UTF-8 locale (LC_CTYPE) but have ".to_vec();
                msg.extend_from_slice(codeset);
                msg
            }
        }
    }
}

impl fmt::Display for LocaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&String::from_utf8_lossy(&self.message()))
    }
}

impl std::error::Error for LocaleError {}

fn setlocale_ctype(name: &CStr) -> bool {
    // SAFETY: name is a valid NUL-terminated string; the returned pointer is
    // only tested against NULL and never dereferenced.
    !unsafe { libc::setlocale(libc::LC_CTYPE, name.as_ptr()) }.is_null()
}

fn setup_ctype_named(named: &[&CStr]) -> Result<(), LocaleError> {
    if named.iter().any(|name| setlocale_ctype(name)) {
        return Ok(());
    }
    if !setlocale_ctype(c"") {
        return Err(LocaleError::InvalidLocale);
    }
    // SAFETY: nl_langinfo returns a pointer to a NUL-terminated string in libc
    // static storage; it is copied before any other locale call.
    let codeset = unsafe { CStr::from_ptr(libc::nl_langinfo(libc::CODESET)) }
        .to_bytes()
        .to_vec();
    if codeset.eq_ignore_ascii_case(b"UTF-8") || codeset.eq_ignore_ascii_case(b"UTF8") {
        Ok(())
    } else {
        Err(LocaleError::NotUtf8(codeset))
    }
}

/// `tmux.c:445-452`: named UTF-8 locales first; environment and codeset check only after both fail.
pub fn setup_ctype() -> Result<(), LocaleError> {
    setup_ctype_named(&[c"en_US.UTF-8", c"C.UTF-8"])
}

pub fn is_alnum(byte: u8) -> bool {
    // SAFETY: an unsigned byte is in the ctype argument domain.
    unsafe { libc::isalnum(i32::from(byte)) != 0 }
}

pub fn is_digit(byte: u8) -> bool {
    // SAFETY: an unsigned byte is in the ctype argument domain.
    unsafe { libc::isdigit(i32::from(byte)) != 0 }
}

pub fn is_alpha(byte: u8) -> bool {
    // SAFETY: an unsigned byte is in the ctype argument domain.
    unsafe { libc::isalpha(i32::from(byte)) != 0 }
}

mod ffi {
    use std::ffi::c_int;

    unsafe extern "C" {
        pub fn tzset();
        pub fn mbtowc(pwc: *mut libc::wchar_t, s: *const libc::c_char, n: libc::size_t) -> c_int;
        pub fn wctomb(s: *mut libc::c_char, wc: libc::wchar_t) -> c_int;
        #[cfg(not(target_os = "macos"))]
        pub fn wcwidth(wc: libc::wchar_t) -> c_int;
    }
}

/// `tmux.c:454-455`.
pub fn setup_time() {
    // SAFETY: both calls only read the environment and libc locale tables.
    unsafe {
        libc::setlocale(libc::LC_TIME, c"".as_ptr());
        ffi::tzset();
    }
}

/// Reset the libc shift state like `mbtowc(NULL, NULL, MB_CUR_MAX)` (`utf8.c:602`).
fn reset_mbtowc() {
    // SAFETY: NULL arguments only reset the internal conversion state; n is ignored.
    unsafe {
        ffi::mbtowc(std::ptr::null_mut(), std::ptr::null(), 1);
    }
}

/// Reset the libc shift state like `wctomb(NULL, 0)` (`utf8.c:690`).
fn reset_wctomb() {
    // SAFETY: a NULL buffer only resets the internal conversion state.
    unsafe {
        ffi::wctomb(std::ptr::null_mut(), 0);
    }
}

#[cfg(target_os = "macos")]
mod utf8proc {
    use std::ffi::c_int;

    // `utf8proc.h` 2.11: UTF8PROC_CATEGORY_CO = 29 (Other, private use).
    pub const CATEGORY_CO: c_int = 29;

    unsafe extern "C" {
        pub fn utf8proc_iterate(str: *const u8, strlen: isize, codepoint_ref: *mut i32) -> isize;
        pub fn utf8proc_codepoint_valid(codepoint: i32) -> bool;
        pub fn utf8proc_encode_char(codepoint: i32, dst: *mut u8) -> isize;
        pub fn utf8proc_category(codepoint: i32) -> c_int;
        pub fn utf8proc_charwidth(codepoint: i32) -> c_int;
    }
}

/// `utf8proc_mbtowc` (`compat/utf8proc.c:39-55`) or libc `mbtowc`, with the
/// `utf8_towc` result rules (`utf8.c:594-606`): the first code point, `None`
/// on failure (state reset) or on a zero return value. The utf8proc path
/// returns length 1 for a NUL byte, so U+0000 decodes there but not with libc.
pub fn mbtowc(bytes: &[u8]) -> Option<u32> {
    let Ok(len) = isize::try_from(bytes.len()) else {
        return None;
    };
    #[cfg(target_os = "macos")]
    let (n, wc) = {
        let mut wc: i32 = 0;
        // SAFETY: bytes is valid for len bytes; wc is a valid out pointer.
        let slen = unsafe { utf8proc::utf8proc_iterate(bytes.as_ptr(), len, &mut wc) };
        if wc == -1 || slen < 0 {
            (-1, 0)
        } else {
            // slen is at most the input length, which fits an int.
            (slen as i32, wc)
        }
    };
    #[cfg(not(target_os = "macos"))]
    let (n, wc) = {
        let mut wc: libc::wchar_t = 0;
        // SAFETY: bytes is valid for len bytes; wc is a valid out pointer.
        let n = unsafe { ffi::mbtowc(&mut wc, bytes.as_ptr().cast(), len as libc::size_t) };
        (n, wc)
    };
    if n < 0 {
        reset_mbtowc();
        return None;
    }
    if n == 0 {
        return None;
    }
    Some(wc as u32)
}

/// `utf8proc_wctomb` (`compat/utf8proc.c:57-66`) or libc `wctomb`, with the
/// `utf8_fromwc` result rules (`utf8.c:683-694`): the byte count, `None` on
/// failure (state reset) or on a zero count.
pub fn wctomb(wc: u32, dst: &mut [u8; 32]) -> Option<usize> {
    #[cfg(target_os = "macos")]
    let size = {
        let wc = wc as i32;
        // SAFETY: utf8proc_encode_char writes at most 4 bytes into dst.
        unsafe {
            if utf8proc::utf8proc_codepoint_valid(wc) {
                utf8proc::utf8proc_encode_char(wc, dst.as_mut_ptr()) as i32
            } else {
                -1
            }
        }
    };
    #[cfg(not(target_os = "macos"))]
    // SAFETY: wctomb writes at most MB_CUR_MAX (<= 16) bytes into dst.
    let size = unsafe { ffi::wctomb(dst.as_mut_ptr().cast(), wc as libc::wchar_t) };
    match size {
        ..0 => {
            reset_wctomb();
            None
        }
        0 => None,
        n => Some(n as usize),
    }
}

/// `utf8proc_wcwidth` (`compat/utf8proc.c:23-37`) or libc `wcwidth`.
pub fn wcwidth(wc: u32) -> i32 {
    #[cfg(target_os = "macos")]
    // SAFETY: pure table lookups on an integer argument.
    unsafe {
        let wc = wc as i32;
        if utf8proc::utf8proc_category(wc) == utf8proc::CATEGORY_CO {
            1
        } else {
            utf8proc::utf8proc_charwidth(wc)
        }
    }
    #[cfg(not(target_os = "macos"))]
    // SAFETY: pure table lookup on an integer argument.
    unsafe {
        ffi::wcwidth(wc as libc::wchar_t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    const CHILD_VAR: &str = "RMUX_SYS_LOCALE_CHILD";

    /// Subprocess entry: runs the selected setup and exits 0 (Ok), 10
    /// (InvalidLocale) or 11 (NotUtf8), printing the message on stdout.
    #[test]
    fn locale_child() {
        let Ok(mode) = std::env::var(CHILD_VAR) else {
            return;
        };
        let result = match mode.as_str() {
            "default" => setup_ctype(),
            "named-missing" => setup_ctype_named(&[c"xx_XX.NOPE", c"yy_YY.NOPE"]),
            other => panic!("unknown child mode {other}"),
        };
        match result {
            Ok(()) => {
                println!("RESULT:ok");
                std::process::exit(0);
            }
            Err(error) => {
                println!("RESULT:{}", String::from_utf8_lossy(&error.message()));
                std::process::exit(match error {
                    LocaleError::InvalidLocale => 10,
                    LocaleError::NotUtf8(_) => 11,
                });
            }
        }
    }

    fn run_child(mode: &str, lc_all: &str) -> (i32, String) {
        let output = Command::new(std::env::current_exe().expect("test binary path"))
            .args(["--exact", "locale::tests::locale_child", "--nocapture"])
            .env(CHILD_VAR, mode)
            .env("LC_ALL", lc_all)
            .env_remove("LC_CTYPE")
            .env_remove("LANG")
            .output()
            .expect("spawn test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = stdout
            .lines()
            .find_map(|line| line.strip_prefix("RESULT:"))
            .unwrap_or_else(|| panic!("no RESULT line in child output:\n{stdout}"))
            .to_owned();
        (output.status.code().expect("exit code"), result)
    }

    #[test]
    fn setup_ctype_accepts_c_locale_environment() {
        assert_eq!(run_child("default", "C"), (0, "ok".to_owned()));
    }

    #[test]
    fn setup_ctype_accepts_c_utf8_environment() {
        assert_eq!(run_child("default", "C.UTF-8"), (0, "ok".to_owned()));
    }

    #[test]
    fn setup_ctype_ignores_bad_environment_when_named_locale_exists() {
        assert_eq!(
            run_child("default", "no_such_locale.XYZ"),
            (0, "ok".to_owned())
        );
    }

    #[test]
    fn named_failure_reports_invalid_environment() {
        let (code, message) = run_child("named-missing", "no_such_locale.XYZ");
        assert_eq!(code, 10);
        assert_eq!(message, "invalid LC_ALL, LC_CTYPE or LANG");
    }

    #[test]
    fn named_failure_reports_non_utf8_codeset() {
        let (code, message) = run_child("named-missing", "C");
        assert_eq!(code, 11);
        let codeset = message
            .strip_prefix("need UTF-8 locale (LC_CTYPE) but have ")
            .unwrap_or_else(|| panic!("unexpected message {message:?}"));
        assert!(!codeset.is_empty());
        assert!(!codeset.eq_ignore_ascii_case("UTF-8"));
    }

    #[test]
    fn named_failure_accepts_utf8_environment() {
        assert_eq!(run_child("named-missing", "C.UTF-8"), (0, "ok".to_owned()));
    }

    #[test]
    fn error_messages_match_tmux() {
        assert_eq!(
            LocaleError::InvalidLocale.to_string(),
            "invalid LC_ALL, LC_CTYPE or LANG"
        );
        assert_eq!(
            LocaleError::NotUtf8(b"US-ASCII".to_vec()).to_string(),
            "need UTF-8 locale (LC_CTYPE) but have US-ASCII"
        );
    }

    #[test]
    fn byte_classification_child() {
        let Ok(locale) = std::env::var("RMUX_SYS_CTYPE_BYTES_CHILD") else {
            return;
        };
        let locale = std::ffi::CString::new(locale).unwrap();
        assert!(setlocale_ctype(&locale));
        print!("BYTES:");
        for byte in 0..=255 {
            print!(
                "{}{}{}",
                u8::from(is_alnum(byte)),
                u8::from(is_digit(byte)),
                u8::from(is_alpha(byte))
            );
        }
        println!();
    }

    #[test]
    fn all_byte_classifications_match_c_and_utf8_locales() {
        let root = std::env::temp_dir().join(format!("rmux-ctype-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("ctype.c");
        let executable = root.join("ctype");
        std::fs::write(&source,b"#include <ctype.h>\n#include <locale.h>\n#include <stdio.h>\nint main(int argc,char **argv){if(argc!=2||!setlocale(LC_CTYPE,argv[1]))return 2;for(int i=0;i<256;i++)printf(\"%d%d%d\",!!isalnum((unsigned char)i),!!isdigit((unsigned char)i),!!isalpha((unsigned char)i));puts(\"\");return 0;}\n").unwrap();
        let status = match Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
        {
            Ok(status) => status,
            Err(error) => {
                eprintln!("skip: C compiler missing for byte classification: {error}");
                let _ = std::fs::remove_dir_all(root);
                return;
            }
        };
        assert!(status.success(), "ctype reference compilation failed");
        let utf8 = if cfg!(target_os = "macos") {
            "en_US.UTF-8"
        } else {
            "C.UTF-8"
        };
        for locale in ["C", utf8] {
            let reference = Command::new(&executable).arg(locale).output().unwrap();
            assert!(reference.status.success(), "missing test locale {locale}");
            let rust = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "locale::tests::byte_classification_child",
                    "--nocapture",
                ])
                .env("RMUX_SYS_CTYPE_BYTES_CHILD", locale)
                .output()
                .unwrap();
            assert!(rust.status.success());
            let stdout = String::from_utf8(rust.stdout).unwrap();
            let bytes = stdout
                .lines()
                .find_map(|line| line.strip_prefix("BYTES:"))
                .unwrap();
            assert_eq!(
                bytes.as_bytes(),
                reference.stdout.strip_suffix(b"\n").unwrap(),
                "locale {locale}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
