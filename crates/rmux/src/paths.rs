// Ported from tmux tmux.c @ 8f25579c
/*
Copyright (c) Various Authors
Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
Copyright (c) 2026 Jacky and rmux contributors

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
*/
//! Path helpers: `expand_path`, `expand_paths`, `check_socket_path`,
//! `make_label`, `find_cwd`, `find_home` (`tmux.c:126-296,382-426`).

use std::sync::LazyLock;

use rmux_sys::client as sys;

/// `TMUX_SOCK` (`tmux.h:96`) with the rmux variable name (spec 2.9).
pub const RMUX_SOCK: &[u8] = b"$RMUX_TMPDIR:/tmp";
/// `TMUX_SOCK_PERM` (`tmux.h:99`): `S_IRWXO`.
pub const RMUX_SOCK_PERM: u32 = 0o007;
/// `TMUX_CONF` (`Makefile.am:14`) with `/etc` as `sysconfdir`.
pub const RMUX_CONF: &[u8] =
    b"/etc/tmux.conf:~/.tmux.conf:$XDG_CONFIG_HOME/tmux/tmux.conf:~/.config/tmux/tmux.conf";

/// `expand_path` (`tmux.c:126-157`): only `~/` and a leading `$NAME` with an
/// optional `/suffix` are expanded. A missing home or variable drops the entry.
pub fn expand_path(
    path: &[u8],
    home: Option<&[u8]>,
    lookup: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    if let Some(rest) = path.strip_prefix(b"~/") {
        let home = home?;
        let mut out = Vec::with_capacity(home.len() + 1 + rest.len());
        out.extend_from_slice(home);
        out.push(b'/');
        out.extend_from_slice(rest);
        return Some(out);
    }
    if let Some(rest) = path.strip_prefix(b"$") {
        let (name, end): (&[u8], &[u8]) = match rest.iter().position(|&c| c == b'/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, b""),
        };
        let value = lookup(name)?;
        let mut out = Vec::with_capacity(value.len() + end.len());
        out.extend_from_slice(&value);
        out.extend_from_slice(end);
        return Some(out);
    }
    Some(path.to_vec())
}

/// `expand_paths` (`tmux.c:159-202`): split at `:`, expand, optionally
/// `realpath`, and keep only the first occurrence of each byte string.
pub fn expand_paths(
    s: &[u8],
    no_realpath: bool,
    home: Option<&[u8]>,
    lookup: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
) -> Vec<Vec<u8>> {
    let mut paths: Vec<Vec<u8>> = Vec::new();
    for next in s.split(|&c| c == b':') {
        let Some(expanded) = expand_path(next, home, lookup) else {
            continue;
        };
        let path = if no_realpath {
            expanded
        } else {
            match sys::realpath(&expanded) {
                Ok(resolved) => resolved,
                Err(_) => continue,
            }
        };
        if paths.contains(&path) {
            continue;
        }
        paths.push(path);
    }
    paths
}

/// Environment lookup for `$NAME` entries (`environ_find(global_environ)`).
pub fn env_lookup(name: &[u8]) -> Option<Vec<u8>> {
    let name = std::str::from_utf8(name).ok()?;
    sys::getenv(name)
}

/// `check_socket_path` (`tmux.c:204-225`).
pub fn check_socket_path(path: &[u8]) -> Result<(), Vec<u8>> {
    if path.first() != Some(&b'/') {
        return Err(format_bytes(&[
            b"socket directory ",
            path,
            b" is not an absolute path",
        ]));
    }
    if path.split(|&c| c == b'/').any(|part| part == b"..") {
        return Err(format_bytes(&[b"socket directory ", path, b" contains .."]));
    }
    Ok(())
}

/// `make_label` (`tmux.c:227-296`) with the rmux directory name `rmux-<uid>`.
/// `uid` is `getuid()`; `label` defaults to `default`.
pub fn make_label(label: Option<&[u8]>, uid: u32) -> Result<Vec<u8>, Vec<u8>> {
    make_label_in(label, uid, RMUX_SOCK, &env_lookup)
}

/// [`make_label`] with the socket directory list and lookup supplied.
pub fn make_label_in(
    label: Option<&[u8]>,
    uid: u32,
    sock_list: &[u8],
    lookup: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
) -> Result<Vec<u8>, Vec<u8>> {
    let label = label.unwrap_or(b"default");
    let mut path: Option<Vec<u8>> = None;
    let mut cause: Option<Vec<u8>> = None;
    // An unset variable has already been dropped and an empty one is skipped,
    // but anything else must be an existing absolute path with no ".." or it
    // is an error (tmux.c:240-259).
    for entry in expand_paths(sock_list, true, find_home(), lookup) {
        if entry.is_empty() {
            continue;
        }
        if let Err(c) = check_socket_path(&entry) {
            cause = Some(c);
            break;
        }
        match sys::realpath(&entry) {
            Ok(resolved) => path = Some(resolved),
            Err(e) => {
                cause = Some(format_bytes(&[
                    b"couldn't resolve socket directory ",
                    &entry,
                    b" (",
                    &rmux_sys::strerror(e.raw_os_error().unwrap_or(0)),
                    b")",
                ]));
            }
        }
        break;
    }
    let Some(path) = path else {
        return Err(cause.unwrap_or_else(|| b"no suitable socket path".to_vec()));
    };

    let base = format_bytes(&[&path, b"/rmux-", uid.to_string().as_bytes()]);
    if let Err(e) = sys::mkdir(&base, 0o700) {
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(format_bytes(&[
                b"couldn't create directory ",
                &base,
                b" (",
                &rmux_sys::strerror(e.raw_os_error().unwrap_or(0)),
                b")",
            ]));
        }
    }
    let sb = match sys::lstat_info(&base) {
        Ok(sb) => sb,
        Err(e) => {
            return Err(format_bytes(&[
                b"couldn't read directory ",
                &base,
                b" (",
                &rmux_sys::strerror(e.raw_os_error().unwrap_or(0)),
                b")",
            ]));
        }
    };
    if !sb.is_dir {
        return Err(format_bytes(&[&base, b" is not a directory"]));
    }
    if sb.uid != uid || (sb.mode & RMUX_SOCK_PERM) != 0 {
        return Err(format_bytes(&[
            b"directory ",
            &base,
            b" has unsafe permissions",
        ]));
    }
    Ok(format_bytes(&[&base, b"/", label]))
}

/// `find_cwd` (`tmux.c:382-405`): `getcwd`, but prefer a nonempty `PWD` whose
/// realpath equals the realpath of the cwd so symlink spellings survive.
pub fn find_cwd() -> Option<Vec<u8>> {
    let cwd = sys::getcwd()?;
    let Some(pwd) = sys::getenv("PWD").filter(|p| !p.is_empty()) else {
        return Some(cwd);
    };
    let Ok(resolved1) = sys::realpath(&pwd) else {
        return Some(cwd);
    };
    let Ok(resolved2) = sys::realpath(&cwd) else {
        return Some(cwd);
    };
    if resolved1 != resolved2 {
        return Some(cwd);
    }
    Some(pwd)
}

static HOME: LazyLock<Option<Vec<u8>>> = LazyLock::new(|| rmux_sys::proc::home_directory(None));

/// `find_home` (`tmux.c:407-426`): nonempty `HOME`, else the passwd directory;
/// the result is cached.
pub fn find_home() -> Option<&'static [u8]> {
    HOME.as_deref()
}

fn format_bytes(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(parts.iter().map(|p| p.len()).sum());
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn no_env(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rmux-paths-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bytes(path: &std::path::Path) -> Vec<u8> {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }

    #[test]
    fn expand_tilde_and_variables() {
        let lookup =
            |name: &[u8]| -> Option<Vec<u8>> { (name == b"XDG").then(|| b"/xdg".to_vec()) };
        assert_eq!(
            expand_path(b"~/.tmux.conf", Some(b"/home/u"), &lookup).unwrap(),
            b"/home/u/.tmux.conf"
        );
        assert_eq!(expand_path(b"~/x", None, &lookup), None);
        assert_eq!(
            expand_path(b"$XDG/tmux", Some(b"/h"), &lookup).unwrap(),
            b"/xdg/tmux"
        );
        assert_eq!(expand_path(b"$XDG", Some(b"/h"), &lookup).unwrap(), b"/xdg");
        assert_eq!(expand_path(b"$MISSING/tmux", Some(b"/h"), &lookup), None);
        assert_eq!(
            expand_path(b"~user/x", Some(b"/h"), &lookup).unwrap(),
            b"~user/x"
        );
        assert_eq!(
            expand_path(b"/etc/tmux.conf", None, &lookup).unwrap(),
            b"/etc/tmux.conf"
        );
    }

    #[test]
    fn expand_paths_dedupes_and_drops() {
        let lookup =
            |name: &[u8]| -> Option<Vec<u8>> { (name == b"V").then(|| b"/home/u".to_vec()) };
        let paths = expand_paths(
            b"/a:$V/b:~/b:$MISSING/c:/a::~/b",
            true,
            Some(b"/home/u"),
            &lookup,
        );
        assert_eq!(
            paths,
            vec![b"/a".to_vec(), b"/home/u/b".to_vec(), b"".to_vec()]
        );
        let resolved = expand_paths(b"/tmp:/nonexistent/zzz:/tmp/", false, None, &no_env);
        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn socket_path_checks() {
        assert_eq!(
            check_socket_path(b"relative").unwrap_err(),
            b"socket directory relative is not an absolute path"
        );
        assert_eq!(
            check_socket_path(b"/tmp/../x").unwrap_err(),
            b"socket directory /tmp/../x contains .."
        );
        assert!(check_socket_path(b"/tmp/a..b").is_ok());
    }

    #[test]
    fn make_label_error_texts() {
        let uid = rmux_sys::proc::getuid().0;

        // Not absolute.
        let lookup = |_: &[u8]| Some(b"relative/dir".to_vec());
        assert_eq!(
            make_label_in(None, uid, b"$D", &lookup).unwrap_err(),
            b"socket directory relative/dir is not an absolute path"
        );
        // Contains "..".
        let lookup = |_: &[u8]| Some(b"/tmp/../tmp".to_vec());
        assert_eq!(
            make_label_in(None, uid, b"$D", &lookup).unwrap_err(),
            b"socket directory /tmp/../tmp contains .."
        );
        // Unresolvable.
        let lookup = |_: &[u8]| Some(b"/nonexistent/rmux-dir".to_vec());
        let err = make_label_in(None, uid, b"$D", &lookup).unwrap_err();
        assert!(
            err.starts_with(b"couldn't resolve socket directory /nonexistent/rmux-dir ("),
            "{}",
            String::from_utf8_lossy(&err)
        );
        // Empty list entries are skipped; no path at all.
        assert_eq!(
            make_label_in(None, uid, b"", &no_env).unwrap_err(),
            b"no suitable socket path"
        );

        // Not a directory.
        let dir = temp_dir("notdir");
        std::fs::write(dir.join(format!("rmux-{uid}")), b"x").unwrap();
        let dir_bytes = bytes(&dir);
        let lookup = |_: &[u8]| Some(dir_bytes.clone());
        let err = make_label_in(None, uid, b"$D", &lookup).unwrap_err();
        let resolved = sys::realpath(&dir_bytes).unwrap();
        assert_eq!(
            err,
            format_bytes(&[
                &resolved,
                b"/rmux-",
                uid.to_string().as_bytes(),
                b" is not a directory"
            ])
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // Unsafe permissions.
        let dir = temp_dir("perm");
        let base = dir.join(format!("rmux-{uid}"));
        std::fs::create_dir(&base).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o707)).unwrap();
        let dir_bytes = bytes(&dir);
        let lookup = |_: &[u8]| Some(dir_bytes.clone());
        let err = make_label_in(None, uid, b"$D", &lookup).unwrap_err();
        let resolved = sys::realpath(&dir_bytes).unwrap();
        assert_eq!(
            err,
            format_bytes(&[
                b"directory ",
                &resolved,
                b"/rmux-",
                uid.to_string().as_bytes(),
                b" has unsafe permissions"
            ])
        );
        std::fs::remove_dir_all(&dir).unwrap();

        // Success creates the directory and appends the label unchanged.
        let dir = temp_dir("ok");
        let dir_bytes = bytes(&dir);
        let lookup = |_: &[u8]| Some(dir_bytes.clone());
        let path = make_label_in(Some(b"a/b"), uid, b"$D", &lookup).unwrap();
        let resolved = sys::realpath(&dir_bytes).unwrap();
        assert_eq!(
            path,
            format_bytes(&[&resolved, b"/rmux-", uid.to_string().as_bytes(), b"/a/b"])
        );
        let created = std::fs::metadata(dir.join(format!("rmux-{uid}"))).unwrap();
        assert_eq!(created.permissions().mode() & 0o777, 0o700);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_cwd_and_home() {
        assert!(find_cwd().is_some());
        assert_eq!(find_home(), find_home());
    }
}
