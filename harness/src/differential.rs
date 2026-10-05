use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
#[derive(Debug)]
pub struct Difference {
    pub command_index: usize,
    pub left: CommandOutput,
    pub right: CommandOutput,
}

#[derive(Debug)]
pub enum Step {
    Command(Vec<String>),
    WaitForPaneOutput { target: String, expected: Vec<u8> },
}
struct Server {
    binary: PathBuf,
    socket: PathBuf,
    directory: tempfile::TempDir,
}
impl Server {
    fn new(binary: &Path) -> io::Result<Self> {
        let temp_root = std::env::var_os("RMUX_TEST_TMPDIR").unwrap_or_else(|| "/tmp".into());
        let directory = tempfile::Builder::new()
            .prefix("rd-")
            .tempdir_in(temp_root)?;
        let server = Self {
            binary: fs::canonicalize(binary)?,
            socket: directory.path().join("socket"),
            directory,
        };
        let output = server.command(&[
            "new-session",
            "-d",
            "-s",
            "test",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 300",
        ])?;
        if output.status != Some(0) {
            return Err(io::Error::other(format!(
                "server startup failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(server)
    }
    fn command(&self, args: &[&str]) -> io::Result<CommandOutput> {
        let stdout = tempfile::tempfile()?;
        let stderr = tempfile::tempfile()?;
        let mut child = Command::new(&self.binary)
            .arg("-S")
            .arg(&self.socket)
            .args(["-f", "/dev/null"])
            .args(args)
            .current_dir(self.directory.path())
            .env_remove("TMUX")
            .env_remove("RMUX")
            .stdout(Stdio::from(stdout.try_clone()?))
            .stderr(Stdio::from(stderr.try_clone()?))
            .spawn()?;
        let start = Instant::now();
        let status: ExitStatus = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if start.elapsed() > Duration::from_secs(10) {
                child.kill()?;
                child.wait()?;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "differential command timeout",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        };
        use std::io::{Read, Seek};
        let read = |mut file: fs::File| -> io::Result<Vec<u8>> {
            file.rewind()?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        Ok(CommandOutput {
            status: status.code(),
            stdout: read(stdout)?,
            stderr: read(stderr)?,
        })
    }

    fn wait_for_pane_output(
        &self,
        target: &str,
        expected: &[u8],
        timeout: Duration,
    ) -> io::Result<()> {
        let start = Instant::now();
        loop {
            let output = self.command(&["capture-pane", "-p", "-t", target])?;
            if output.status != Some(0) {
                return Err(io::Error::other(format!(
                    "{}: capture of pane {target} failed: {}",
                    self.binary.display(),
                    String::from_utf8_lossy(&output.stderr),
                )));
            }
            if expected.is_empty() || output.stdout.windows(expected.len()).any(|s| s == expected) {
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "{}: pane {target} did not output {:?}; last capture: {:?}",
                        self.binary.display(),
                        String::from_utf8_lossy(expected),
                        String::from_utf8_lossy(&output.stdout),
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
    }
}

pub fn compare(left: &Path, right: &Path, steps: &[Step]) -> io::Result<Vec<Difference>> {
    let left = Server::new(left)?;
    let right = Server::new(right)?;
    let mut differences = Vec::new();
    let mut command_index = 0;
    for step in steps {
        let command = match step {
            Step::Command(command) => command,
            Step::WaitForPaneOutput { target, expected } => {
                left.wait_for_pane_output(target, expected, Duration::from_secs(5))?;
                right.wait_for_pane_output(target, expected, Duration::from_secs(5))?;
                continue;
            }
        };
        let args: Vec<_> = command.iter().map(String::as_str).collect();
        let left = left.command(&args)?;
        let right = right.command(&args)?;
        if left != right {
            differences.push(Difference {
                command_index,
                left,
                right,
            });
        }
        command_index += 1;
    }
    Ok(differences)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn oracle() -> Option<PathBuf> {
        let path = std::env::var_os("RMUX_ORACLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/bin/tmux"));
        if path.exists() {
            Some(path)
        } else {
            eprintln!("oracle smoke skipped: build scripts/build-oracle.sh or set RMUX_ORACLE");
            None
        }
    }
    #[test]
    fn oracle_against_oracle() {
        let Some(oracle) = oracle() else {
            return;
        };
        let commands = [
            vec!["display", "-p", "#{session_name}"],
            vec!["capture-pane", "-p", "-e"],
            vec![
                "list-panes",
                "-F",
                "#{pane_index}:#{pane_width}:#{pane_height}",
            ],
        ]
        .into_iter()
        .map(|args| Step::Command(args.into_iter().map(String::from).collect()))
        .collect::<Vec<_>>();
        let differences = compare(&oracle, &oracle, &commands).unwrap();
        println!("oracle-vs-oracle: {} differences", differences.len());
        assert!(differences.is_empty());
    }
    #[test]
    fn pane_output_wait_is_bounded() {
        let Some(oracle) = oracle() else {
            return;
        };
        let server = Server::new(&oracle).unwrap();
        server
            .command(&[
                "new-window",
                "-d",
                "-n",
                "ready",
                "printf READY; exec sleep 60",
            ])
            .unwrap();
        server
            .wait_for_pane_output("ready", b"READY", Duration::from_secs(5))
            .unwrap();
        let start = Instant::now();
        let error = server
            .wait_for_pane_output("ready", b"NEVER", Duration::from_millis(50))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    fn rmux() -> Option<PathBuf> {
        let path = std::env::var_os("RMUX_BINARY")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("CARGO_TARGET_DIR")
                    .map(|dir| PathBuf::from(dir).join("debug/rmux"))
            })
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/debug/rmux"));
        if path.is_file() {
            Some(path)
        } else {
            eprintln!("server teardown comparison skipped: rmux binary missing (set RMUX_BINARY)");
            None
        }
    }

    fn process_ids(server: &Server) -> Vec<rmux_sys::ProcessId> {
        let output = server
            .command(&["display", "-p", "#{pid} #{pane_pid}"])
            .unwrap();
        assert_eq!(output.status, Some(0));
        String::from_utf8(output.stdout)
            .unwrap()
            .split_whitespace()
            .map(|pid| rmux_sys::ProcessId(pid.parse().unwrap()))
            .collect()
    }

    fn assert_processes_exit(pids: &[rmux_sys::ProcessId]) {
        let start = Instant::now();
        while pids
            .iter()
            .any(|pid| rmux_sys::client::kill(*pid, 0).is_ok())
        {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "server or pane leaked: {pids:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn servers_exit_and_release_panes() {
        let (Some(oracle), Some(rmux)) = (oracle(), rmux()) else {
            return;
        };
        for binary in [&oracle, &rmux] {
            let live = Server::new(binary).unwrap();
            let old = process_ids(&live);
            let output = live
                .command(&["respawn-pane", "-k", "exec sleep 300"])
                .unwrap();
            assert_eq!(output.status, Some(0), "{binary:?}: {output:?}");
            assert_processes_exit(&old[1..]);
            assert_eq!(live.command(&["has-session"]).unwrap().status, Some(0));
            let current = process_ids(&live);
            assert_eq!(current[0], old[0]);
            assert_ne!(current[1], old[1]);
            drop(live);
            assert_processes_exit(&current);
            for command in ["kill-server", "kill-session"] {
                let server = Server::new(binary).unwrap();
                let pids = process_ids(&server);
                assert_eq!(pids.len(), 2);
                let output = server.command(&[command]).unwrap();
                assert_eq!(output.status, Some(0), "{binary:?}: {output:?}");
                assert_processes_exit(&pids);
            }
            let server = Server::new(binary).unwrap();
            let pids = process_ids(&server);
            let output = server
                .command(&["kill-server", ";", "display", "-p", "bye"])
                .unwrap();
            assert_eq!(output.status, Some(0), "{binary:?}: {output:?}");
            assert_eq!(output.stdout, b"bye\n");
            assert_processes_exit(&pids);

            let mut child_pids = Vec::new();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let server = Server::new(binary).unwrap();
                child_pids = process_ids(&server);
                panic!("exercise panic-safe socket cleanup");
            }));
            assert!(result.is_err());
            assert_processes_exit(&child_pids);
        }
    }
    #[test]
    fn deliberate_mismatch() {
        let Some(oracle) = oracle() else {
            return;
        };
        let left = Server::new(&oracle).unwrap();
        let right = Server::new(&oracle).unwrap();
        right.command(&["rename-session", "changed"]).unwrap();
        assert_ne!(
            left.command(&["display", "-p", "#{session_name}"]).unwrap(),
            right
                .command(&["display", "-p", "#{session_name}"])
                .unwrap()
        );
        println!("deliberate mismatch: detected");
    }
}
