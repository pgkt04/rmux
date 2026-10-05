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
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
    }
}

pub fn compare(left: &Path, right: &Path, commands: &[Vec<String>]) -> io::Result<Vec<Difference>> {
    let left = Server::new(left)?;
    let right = Server::new(right)?;
    let mut differences = Vec::new();
    for (command_index, command) in commands.iter().enumerate() {
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
        .map(|args| args.into_iter().map(String::from).collect())
        .collect::<Vec<_>>();
        let differences = compare(&oracle, &oracle, &commands).unwrap();
        println!("oracle-vs-oracle: {} differences", differences.len());
        assert!(differences.is_empty());
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
