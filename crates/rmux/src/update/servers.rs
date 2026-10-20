//! Live rmux servers that keep running the binary they were started from.

use std::ffi::OsString;
use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::paths;

const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

pub struct RunningServer {
    pub socket: PathBuf,
    pub pid: u32,
    pub start_time: u64,
}

/// This user's live rmux servers, asked through `exe` (the binary they run).
pub fn running(exe: &Path) -> Vec<RunningServer> {
    socket_dir().map_or_else(Vec::new, |dir| running_in(&dir, exe))
}

/// Lines telling the user these servers keep the previous build until they
/// restart (which ends their sessions). Empty when `servers` is empty.
pub fn report(servers: &[RunningServer]) -> Vec<String> {
    if servers.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::with_capacity(servers.len() + 1);
    lines.push(format!(
        "{} running rmux server(s) keep the previous build:",
        servers.len()
    ));
    for server in servers {
        lines.push(format!(
            "  server {} (pid {}, started {}) still runs the previous build; it moves to the new one only after kill-server, which ends its sessions",
            server.socket.display(),
            server.pid,
            local_time(server.start_time),
        ));
    }
    lines
}

/// The `rmux-<uid>` directory `make_label` would use, without creating it.
fn socket_dir() -> Option<PathBuf> {
    let entry = paths::expand_paths(
        paths::RMUX_SOCK,
        true,
        paths::find_home(),
        &paths::env_lookup,
    )
    .into_iter()
    .find(|entry| !entry.is_empty())?;
    paths::check_socket_path(&entry).ok()?;
    let base = rmux_sys::client::realpath(&entry).ok()?;
    Some(PathBuf::from(OsString::from_vec(base)).join(format!("rmux-{}", socket_uid())))
}

/// Under sudo the servers to report are the invoking user's, not root's.
fn socket_uid() -> u32 {
    let uid = rmux_sys::proc::getuid().0;
    if uid != 0 {
        return uid;
    }
    std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(uid)
}

fn running_in(dir: &Path, exe: &Path) -> Vec<RunningServer> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sockets: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_socket()))
        .map(|entry| entry.path())
        .collect();
    sockets.sort();
    sockets
        .into_iter()
        .filter_map(|socket| {
            let (pid, start_time) = query(exe, &socket)?;
            Some(RunningServer {
                socket,
                pid,
                start_time,
            })
        })
        .collect()
}

fn query(exe: &Path, socket: &Path) -> Option<(u32, u64)> {
    let mut child = Command::new(exe)
        .arg("-N")
        .arg("-S")
        .arg(socket)
        .args(["display-message", "-p", "#{pid} #{start_time}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + QUERY_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if !status.success() {
        return None;
    }
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    parse_query(&out)
}

fn parse_query(out: &[u8]) -> Option<(u32, u64)> {
    let text = std::str::from_utf8(out).ok()?.trim();
    let (pid, start_time) = text.split_once(' ')?;
    Some((pid.parse().ok()?, start_time.parse().ok()?))
}

fn local_time(seconds: u64) -> String {
    i64::try_from(seconds)
        .ok()
        .and_then(rmux_sys::time::localtime)
        .map_or_else(
            || seconds.to_string(),
            |tm| {
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}",
                    tm.year() + 1900,
                    tm.mon() + 1,
                    tm.mday(),
                    tm.hour(),
                    tm.min()
                )
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(socket: &str, pid: u32) -> RunningServer {
        RunningServer {
            socket: PathBuf::from(socket),
            pid,
            start_time: 1_700_000_000,
        }
    }

    fn split_time(line: &str) -> (&str, &str, &str) {
        let (head, rest) = line.split_once("started ").unwrap();
        let (time, tail) = rest.split_once(')').unwrap();
        (head, time, tail)
    }

    #[test]
    fn report_none() {
        assert!(report(&[]).is_empty());
    }

    #[test]
    fn report_one() {
        let lines = report(&[server("/tmp/rmux-501/default", 42)]);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "1 running rmux server(s) keep the previous build:"
        );
        let (head, time, tail) = split_time(&lines[1]);
        assert_eq!(head, "  server /tmp/rmux-501/default (pid 42, ");
        assert_eq!(time, local_time(1_700_000_000));
        assert_eq!(time.len(), "2023-11-14 22:13".len());
        assert_eq!(
            tail,
            " still runs the previous build; it moves to the new one only after kill-server, which ends its sessions"
        );
    }

    #[test]
    fn report_two() {
        let lines = report(&[server("/tmp/rmux-501/a", 1), server("/tmp/rmux-501/b", 2)]);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "2 running rmux server(s) keep the previous build:"
        );
        assert_eq!(split_time(&lines[1]).0, "  server /tmp/rmux-501/a (pid 1, ");
        assert_eq!(split_time(&lines[2]).0, "  server /tmp/rmux-501/b (pid 2, ");
    }

    #[test]
    fn parse_query_output() {
        assert_eq!(
            parse_query(b"1234 1700000000\n"),
            Some((1234, 1_700_000_000))
        );
        assert_eq!(parse_query(b"1 2"), Some((1, 2)));
        assert_eq!(parse_query(b""), None);
        assert_eq!(parse_query(b"1234\n"), None);
        assert_eq!(parse_query(b"abc 1\n"), None);
        assert_eq!(parse_query(b"1 2 3\n"), None);
        assert_eq!(parse_query(b"-1 2\n"), None);
    }

    #[test]
    fn running_without_sockets() {
        let exe = Path::new("/nonexistent/rmux");
        let dir = std::env::temp_dir().join(format!("rmux-servers-test.{}", std::process::id()));
        assert!(running_in(&dir, exe).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("not-a-socket"), b"").unwrap();
        let found = running_in(&dir, exe);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(found.is_empty());
    }
}
