// Ported from tmux tmux.c @ 8f25579c (getshell, checkshell, areshell, shell_argv0)
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

/// `getshell` (`tmux.c:80-95`): `$SHELL` if `check_shell` accepts it, then
/// the passwd shell, then `/bin/sh`. `program` is `getprogname()`.
pub fn get_shell(program: &[u8]) -> Vec<u8> {
    get_shell_from(
        rmux_sys::client::getenv("SHELL").as_deref(),
        rmux_sys::client::passwd_shell().as_deref(),
        program,
    )
}

/// Pure core of [`get_shell`] with the environment and passwd values supplied.
pub fn get_shell_from(
    env_shell: Option<&[u8]>,
    pw_shell: Option<&[u8]>,
    program: &[u8],
) -> Vec<u8> {
    if let Some(shell) = env_shell.filter(|s| check_shell(s, program)) {
        return crate::bytes::cstr(shell).to_vec();
    }
    if let Some(shell) = pw_shell.filter(|s| check_shell(s, program)) {
        return crate::bytes::cstr(shell).to_vec();
    }
    b"/bin/sh".to_vec()
}

/// `shell_argv0` (`tmux.c:298-314`): the basename unless the path ends with
/// `/`, prefixed with `-` for a login shell.
pub fn shell_argv0(shell: &[u8], is_login: bool) -> Vec<u8> {
    let shell = crate::bytes::cstr(shell);
    let name = match shell.iter().rposition(|&c| c == b'/') {
        Some(i) if i + 1 < shell.len() => &shell[i + 1..],
        _ => shell,
    };
    let mut argv0 = Vec::with_capacity(name.len() + 1);
    if is_login {
        argv0.push(b'-');
    }
    argv0.extend_from_slice(name);
    argv0
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

    #[test]
    fn getshell_prefers_env_then_passwd_then_sh() {
        assert_eq!(get_shell_from(Some(b"/bin/sh"), None, b"rmux"), b"/bin/sh");
        assert_eq!(
            get_shell_from(Some(b"sh"), Some(b"/bin/sh"), b"rmux"),
            b"/bin/sh"
        );
        assert_eq!(
            get_shell_from(Some(b"/bin/sh"), None, b"sh"),
            b"/bin/sh".to_vec()
        );
        assert_eq!(
            get_shell_from(None, Some(b"/nonexistent/x"), b"rmux"),
            b"/bin/sh"
        );
        assert_eq!(get_shell_from(None, None, b"rmux"), b"/bin/sh");
        assert!(get_shell(b"rmux").starts_with(b"/"));
    }

    #[test]
    fn argv0_login_and_non_login() {
        assert_eq!(shell_argv0(b"/bin/sh", false), b"sh");
        assert_eq!(shell_argv0(b"/bin/sh", true), b"-sh");
        assert_eq!(shell_argv0(b"sh", true), b"-sh");
        assert_eq!(shell_argv0(b"/bin/", false), b"/bin/");
        assert_eq!(shell_argv0(b"/usr/bin/zsh\0junk", true), b"-zsh");
    }
}
