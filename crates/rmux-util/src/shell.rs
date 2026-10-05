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

#[cfg(test)]
mod tests {
    use super::*;

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
