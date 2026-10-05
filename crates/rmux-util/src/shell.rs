// Ported from tmux tmux.c @ 8f25579c (checkshell, areshell)
//! Shell path validation shared by the `default-shell` option and startup.

/// `areshell`: the last path component equals the program name, after a
/// leading `-` (login shell marker) is stripped from the program name.
fn is_own_name(shell: &[u8], program: &[u8]) -> bool {
    let base = match shell.iter().rposition(|&c| c == b'/') {
        Some(i) => &shell[i + 1..],
        None => shell,
    };
    let program = program.strip_prefix(b"-").unwrap_or(program);
    base == program
}

/// `checkshell`: absolute, not this program itself, and executable
/// (`access(X_OK)`). `program` is `getprogname()`.
pub fn check_shell(path: &[u8], program: &[u8]) -> bool {
    let path = crate::bytes::cstr(path);
    if path.first() != Some(&b'/') {
        return false;
    }
    if is_own_name(path, crate::bytes::cstr(program)) {
        return false;
    }
    rmux_sys::access_executable(path)
}

/// `clean_name`: valid UTF-8, visible controls, and untrusted format-job filtering.
pub fn clean_name(name: &[u8], untrusted: bool) -> Option<Vec<u8>> {
    let name = crate::bytes::cstr(name);
    std::str::from_utf8(name).ok()?;
    let mut bytes = name.to_vec();
    if untrusted {
        for i in 0..bytes.len().saturating_sub(1) {
            if bytes[i] == b'#' && bytes[i + 1] == b'(' {
                bytes[i] = b'_';
            }
        }
    }
    let mut out = Vec::new();
    crate::utf8::strvis(
        &mut out,
        &bytes,
        crate::vis::VisFlags::OCTAL
            | crate::vis::VisFlags::CSTYLE
            | crate::vis::VisFlags::TAB
            | crate::vis::VisFlags::NL,
    );
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_validate_escape_and_filter_jobs() {
        assert!(clean_name(&[0xff], true).is_none());
        assert_eq!(
            clean_name(b"x#(whoami)\t\n", true).unwrap(),
            b"x_(whoami)\\t\\n"
        );
        assert_eq!(
            clean_name("é#(job)".as_bytes(), false).unwrap(),
            "é#(job)".as_bytes()
        );
        assert_eq!(clean_name(b"name\0ignored", true).unwrap(), b"name");
    }

    #[test]
    fn bin_sh_is_a_shell() {
        assert!(check_shell(b"/bin/sh", b"tmux"));
        assert!(check_shell(b"/bin/sh", b"-tmux"));
    }

    #[test]
    fn relative_and_empty_paths_are_rejected() {
        assert!(!check_shell(b"sh", b"tmux"));
        assert!(!check_shell(b"bin/sh", b"tmux"));
        assert!(!check_shell(b"", b"tmux"));
        assert!(!check_shell(b"\0/bin/sh", b"tmux"));
    }

    #[test]
    fn nonexistent_is_rejected() {
        assert!(!check_shell(b"/nonexistent/rmux-shell", b"tmux"));
    }

    #[test]
    fn own_basename_is_rejected() {
        assert!(!check_shell(b"/bin/sh", b"sh"));
        assert!(!check_shell(b"/bin/sh", b"-sh"));
        assert!(is_own_name(b"/usr/local/bin/tmux", b"tmux"));
        assert!(is_own_name(b"tmux", b"-tmux"));
        assert!(!is_own_name(b"/bin/sh", b"tmux"));
    }

    #[test]
    fn not_executable_is_rejected() {
        let dir = std::env::temp_dir().join(format!("rmux-shell-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("plain");
        std::fs::write(&file, b"#!/bin/sh\n").unwrap();
        let path = file.as_os_str().as_encoded_bytes();
        assert!(!check_shell(path, b"tmux"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
