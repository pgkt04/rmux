// Ported from tmux format.c @ 8f25579c
//! Platform `basename(3)` and `dirname(3)`, not Rust path normalisation
//! (`format.c:4715-4723`).

use crate::cstring::{copy_cstr, nul_terminated};

// With <libgen.h> included, glibc's basename is the POSIX __xpg_basename.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
use libc::basename as libc_basename;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use libc::posix_basename as libc_basename;

/// `basename(3)` on a NUL-terminated mutable copy; the result is copied
/// before libc can reuse its static storage.
#[must_use]
pub fn basename(path: &[u8]) -> Vec<u8> {
    let mut copy = nul_terminated(path);
    // SAFETY: `copy` is a writable NUL-terminated buffer that outlives the
    // call, and basename returns either a pointer into it or a static string.
    unsafe { copy_cstr(libc_basename(copy.as_mut_ptr().cast())) }
}

/// `dirname(3)` on a NUL-terminated mutable copy; the result is copied
/// before libc can reuse its static storage.
#[must_use]
pub fn dirname(path: &[u8]) -> Vec<u8> {
    let mut copy = nul_terminated(path);
    // SAFETY: `copy` is a writable NUL-terminated buffer that outlives the
    // call, and dirname returns either a pointer into it or a static string.
    unsafe { copy_cstr(libc::dirname(copy.as_mut_ptr().cast())) }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    const FIXTURES: [&[u8]; 6] = [b"", b"/", b"//a//b//", b"a/", b"a", b"/a"];

    const C_CHECK: &str = r#"
#include <libgen.h>
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
    char b[64], d[64];
    for (int i = 1; i < argc; i++) {
        strcpy(b, argv[i]); strcpy(d, argv[i]);
        printf("%s\n%s\n", basename(b), dirname(d));
    }
    return 0;
}
"#;

    #[test]
    fn matches_platform_libc() {
        let dir = std::env::temp_dir().join(format!("rmux-sys-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("check.c");
        let bin = dir.join("check");
        std::fs::write(&src, C_CHECK).unwrap();
        let compiled = Command::new("cc")
            .arg("-o")
            .arg(&bin)
            .arg(&src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !compiled {
            eprintln!("skipping path::tests::matches_platform_libc: cc unavailable");
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        let output = Command::new(&bin)
            .args(FIXTURES.iter().map(|f| std::str::from_utf8(f).unwrap()))
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(output.status.success());
        let mut lines = output.stdout.split(|&b| b == b'\n');
        for fixture in FIXTURES {
            let expect_base = lines.next().unwrap();
            let expect_dir = lines.next().unwrap();
            assert_eq!(basename(fixture), expect_base, "basename({fixture:?})");
            assert_eq!(dirname(fixture), expect_dir, "dirname({fixture:?})");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_fixtures() {
        let expected: [(&[u8], &[u8]); 6] = [
            (b".", b"."),
            (b"/", b"/"),
            (b"b", b"//a"),
            (b"a", b"."),
            (b"a", b"."),
            (b"a", b"/"),
        ];
        for (fixture, (base, dir)) in FIXTURES.iter().zip(expected) {
            assert_eq!(basename(fixture), base, "basename({fixture:?})");
            assert_eq!(dirname(fixture), dir, "dirname({fixture:?})");
        }
    }

    #[test]
    fn stops_at_nul() {
        assert_eq!(basename(b"/x/y\0/z"), b"y");
        assert_eq!(dirname(b"/x/y\0/z"), b"/x");
    }
}
