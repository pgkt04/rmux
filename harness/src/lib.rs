#![forbid(unsafe_code)]

pub mod differential;

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs, io,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const PIN: &str = include_str!("../../oracle/PIN");

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub tests: Vec<Test>,
}
#[derive(Debug, Deserialize)]
pub struct Test {
    pub name: String,
    pub category: Category,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    Unchanged,
    Mapped,
    Adapted,
}

pub fn manifest() -> Result<Manifest, toml::de::Error> {
    toml::from_str(include_str!("../regress-manifest.toml"))
}

pub fn extract(source: &Path, destination: &Path) -> io::Result<()> {
    let archive = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["archive", PIN.trim(), "regress", "tmux-protocol.h"])
        .output()?;
    if !archive.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&archive.stderr).into_owned(),
        ));
    }
    let mut tar = Command::new("tar")
        .args(["-x", "-C"])
        .arg(destination)
        .stdin(Stdio::piped())
        .spawn()?;
    use std::io::Write;
    tar.stdin
        .take()
        .ok_or_else(|| io::Error::other("tar stdin missing"))?
        .write_all(&archive.stdout)?;
    if !tar.wait()?.success() {
        return Err(io::Error::other("git archive extraction failed"));
    }
    Ok(())
}

pub fn validate(manifest: &Manifest, regress: &Path) -> io::Result<()> {
    let actual: BTreeSet<_> = fs::read_dir(regress)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<BTreeSet<_>>>()?
        .into_iter()
        .filter(|name| name.ends_with(".sh"))
        .collect();
    let listed: BTreeSet<_> = manifest
        .tests
        .iter()
        .map(|test| test.name.clone())
        .collect();
    if listed != actual || listed.len() != manifest.tests.len() {
        return Err(io::Error::other(format!(
            "manifest mismatch: missing {:?}, extra {:?}, duplicates {}",
            actual.difference(&listed).collect::<Vec<_>>(),
            listed.difference(&actual).collect::<Vec<_>>(),
            manifest.tests.len() - listed.len()
        )));
    }
    Ok(())
}

pub fn map_namespace(text: &str) -> String {
    // Keep TEST_TMUX and tmux terminal names unchanged; only namespace tokens move.
    let mut output = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if boundary && text[i..].starts_with("TMUX") {
            let end = i + 4;
            if end == bytes.len() || !bytes[end].is_ascii_alphanumeric() {
                output.push_str("RMUX");
                i = end;
                continue;
            }
        }
        let mut matched = false;
        for (from, to) in [
            ("tmux-server-", "rmux-server-"),
            ("tmux-client-", "rmux-client-"),
            ("tmux-out-", "rmux-out-"),
            ("tmux-$(id -u)", "rmux-$(id -u)"),
            ("^tmux ", "^rmux "),
        ] {
            if text[i..].starts_with(from) {
                output.push_str(to);
                i += from.len();
                matched = true;
                break;
            }
        }
        if !matched {
            let ch = text[i..]
                .chars()
                .next()
                .expect("character at valid boundary");
            output.push(ch);
            i += ch.len_utf8();
        }
    }
    output
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResultEntry {
    pub name: String,
    pub category: Category,
    pub status: String,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub mapped: bool,
}

struct SocketCleanup<'a> {
    binary: &'a Path,
    sockets: &'a Path,
    test_name: &'a str,
    finished: bool,
}

impl SocketCleanup<'_> {
    fn finish(&mut self) -> io::Result<()> {
        if let Ok(recorded) = fs::read_to_string(self.sockets) {
            for socket in recorded.lines().collect::<BTreeSet<_>>() {
                let mut cleanup = Command::new(self.binary)
                    .args(["-S", socket, "kill-server"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?;
                let start = Instant::now();
                while cleanup.try_wait()?.is_none() {
                    if start.elapsed() >= Duration::from_secs(5) {
                        cleanup.kill()?;
                        cleanup.wait()?;
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("socket cleanup timed out: {socket}"),
                        ));
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                if self.test_name == "socket-path.sh" && socket.contains("/testSP") {
                    let _ = fs::remove_file(socket);
                    let _ = fs::remove_file(format!("{socket}.lock"));
                }
            }
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for SocketCleanup<'_> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.finish();
        }
    }
}

pub fn run_test(
    binary: &Path,
    script: &Path,
    work: &Path,
    timeout: Duration,
    rmux: bool,
    test: &Test,
) -> io::Result<ResultEntry> {
    let adapted_script = if rmux && test.category == Category::Adapted {
        let source = match test.name.as_str() {
            "cfg-client-lost-before-wait.sh" => {
                include_str!("../fixtures/cfg-client-lost-before-wait.sh")
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unknown adapted regression",
                ));
            }
        };
        fs::create_dir_all(work)?;
        let path = work.join("adapted-test.sh");
        fs::write(&path, source)?;
        Some(path)
    } else {
        None
    };
    let script = adapted_script.as_deref().unwrap_or(script);
    fs::create_dir_all(work)?;
    let out_path = work.join("stdout");
    let err_path = work.join("stderr");
    let sockets = work.join("sockets");
    let wrapper = work.join("test-tmux");
    let namespace = if rmux { "RMUX" } else { "TMUX" };
    let prefix = if rmux { "rmux" } else { "tmux" };
    let quote = |path: &Path| format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
    let binary_path = quote(binary);
    let sockets_path = quote(&sockets);
    let work_path = quote(work);
    // `env -i` in some scripts drops the isolation dir; an unset (not empty)
    // namespace TMPDIR is restored so clients find servers started under it.
    let tmpdir_isolation = format!(
        "if [ -z \"${{{namespace}_TMPDIR+x}}\" ]; then\n {namespace}_TMPDIR={work_path}; export {namespace}_TMPDIR\nfi\n"
    );
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nrecord_socket() {{\nlabel=default\nsocket=\nwhile [ $# -gt 0 ]; do\n case \"$1\" in\n -L) label=$2; shift;; -L*) label=${{1#-L}};;\n -S) socket=$2; shift;; -S*) socket=${{1#-S}};;\n -f|-T|-c) shift;; --) break;; -*) ;; *) break;;\n esac\n shift\ndone\n[ -n \"$socket\" ] || socket=\"${{{namespace}_TMPDIR:-/tmp}}/{prefix}-$(/usr/bin/id -u)/$label\"\nprintf '%s\\n' \"$socket\" >> {sockets_path}\n}}\n{tmpdir_isolation}record_socket \"$@\"\nexec {binary_path} \"$@\"\n"
        ),
    )?;
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700))?;
    let mut cleanup = SocketCleanup {
        binary,
        sockets: &sockets,
        test_name: &test.name,
        finished: false,
    };
    let start = Instant::now();
    let mut child = Command::new("sh")
        .arg(script)
        .current_dir(script.parent().expect("script directory"))
        .env("TEST_TMUX", &wrapper)
        .env("TMPDIR", work)
        .env("TMUX_TMPDIR", work)
        .env("RMUX_TMPDIR", work)
        .env_remove("TMUX")
        .env_remove("TMUX_PANE")
        .env_remove("RMUX")
        .env_remove("RMUX_PANE")
        .stdout(fs::File::create(&out_path)?)
        .stderr(fs::File::create(&err_path)?)
        .process_group(0)
        .spawn()?;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if start.elapsed() >= timeout {
            if !rmux_sys::terminate_process_group(&mut child)? {
                eprintln!(
                    "{}: process-group termination denied; killing script and cleaning exact sockets",
                    test.name
                );
            }
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            break (child.wait()?, true);
        }
        thread::sleep(Duration::from_millis(20));
    };
    cleanup.finish()?;
    Ok(ResultEntry {
        name: test.name.clone(),
        category: test.category,
        status: if timed_out {
            "timeout"
        } else if status.success() {
            "pass"
        } else {
            "fail"
        }
        .into(),
        exit_code: status.code(),
        elapsed_ms: start.elapsed().as_millis(),
        stdout: fs::read_to_string(out_path)?,
        stderr: fs::read_to_string(err_path)?,
        mapped: rmux && test.category == Category::Mapped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_covers_pinned_scripts() {
        let source = std::env::var_os("TMUX_SRC")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join("fun/tmux")
            });
        let tmp = tempfile::tempdir().unwrap();
        extract(&source, tmp.path()).unwrap();
        let manifest = manifest().unwrap();
        assert_eq!(manifest.tests.len(), 201);
        validate(&manifest, &tmp.path().join("regress")).unwrap();
        let mut incomplete = manifest;
        incomplete.tests.pop();
        assert!(validate(&incomplete, &tmp.path().join("regress")).is_err());
    }
    #[test]
    fn mapping_is_namespace_only() {
        assert_eq!(
            map_namespace(
                "TEST_TMUX $TMUX ${TMUX_PANE} TMUX_TMPDIR tmux-$(id -u) tmux-server-*.log tmux-256color"
            ),
            "TEST_TMUX $RMUX ${RMUX_PANE} RMUX_TMPDIR rmux-$(id -u) rmux-server-*.log tmux-256color"
        );
    }
    #[test]
    fn mapping_version_prefix_preserves_version_and_terminal_names() {
        assert_eq!(
            map_namespace("VER=$($TMUX -V | sed 's/^tmux //')\n#{version} tmux-256color TEST_TMUX"),
            "VER=$($RMUX -V | sed 's/^rmux //')\n#{version} tmux-256color TEST_TMUX"
        );
        assert_eq!(map_namespace("tmux next-3.9"), "tmux next-3.9");
    }
    #[test]
    fn timeout_is_reported_and_unknown_fixture_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let script = tmp.path().join("test.sh");
        fs::write(&script, "sleep 1\n").unwrap();
        let mut test = Test {
            name: "test.sh".into(),
            category: Category::Unchanged,
        };
        let result = run_test(
            Path::new("/bin/false"),
            &script,
            &tmp.path().join("run"),
            Duration::from_millis(50),
            false,
            &test,
        )
        .unwrap();
        assert_eq!(result.status, "timeout");
        test.category = Category::Adapted;
        let result = run_test(
            Path::new("/nonexistent"),
            &script,
            &tmp.path().join("fixture"),
            Duration::from_millis(50),
            true,
            &test,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(!tmp.path().join("fixture").exists());
    }
}
