// Ported from tmux cmd-new-session.c, cmd-new-window.c, cmd-capture-pane.c, cmd-list-sessions.c, cmd-list-windows.c, cmd-list-panes.c, cmd-list-buffers.c, cmd-if-shell.c, cmd-break-pane.c, cmd-join-pane.c @ 8f25579c
use std::{
    path::{Path, PathBuf},
    process::Command,
};

struct Socket<'a> {
    binary: &'a Path,
    path: PathBuf,
}
impl Socket<'_> {
    fn run(&self, args: &[&str]) -> (bool, Vec<u8>, Vec<u8>) {
        let result = Command::new(self.binary)
            .args(["-f", "/dev/null", "-S"])
            .arg(&self.path)
            .args(args)
            .output()
            .expect("command launch");
        (result.status.success(), result.stdout, result.stderr)
    }
}
impl Drop for Socket<'_> {
    fn drop(&mut self) {
        let _ = Command::new(self.binary)
            .arg("-S")
            .arg(&self.path)
            .arg("kill-server")
            .output();
    }
}

fn scenario(binary: &Path, path: PathBuf) -> Vec<(bool, Vec<u8>, Vec<u8>)> {
    let socket = Socket { binary, path };
    assert!(
        socket
            .run(&[
                "new-session",
                "-d",
                "-s",
                "parity",
                "-x",
                "40",
                "-y",
                "8",
                "sleep 60"
            ])
            .0
    );
    let mut outputs = Vec::new();
    for args in [
        vec!["has-session", "-t", "parity"],
        vec!["list-sessions", "-F", "#{session_name} #{session_windows}"],
        vec![
            "list-windows",
            "-F",
            "#{window_index} #{window_width}x#{window_height}",
        ],
        vec![
            "list-panes",
            "-F",
            "#{pane_index} #{pane_width}x#{pane_height}",
        ],
        vec!["capture-pane", "-p"],
        vec!["capture-pane", "-p", "-R"],
        vec!["capture-pane", "-p", "-H"],
        vec!["capture-pane", "-p", "-M"],
        vec!["capture-pane", "-p", "-a"],
        vec!["clear-history"],
        vec![
            "new-window",
            "-d",
            "-n",
            "second",
            "-P",
            "-F",
            "#{window_index}",
            "sleep 60",
        ],
        vec![
            "if-shell",
            "-F",
            "1",
            "display-message -p YES",
            "display-message -p NO",
        ],
        vec!["list-buffers", "-F", "#{buffer_name}"],
        vec!["break-pane", "-s", "parity:0.0", "-t", "parity:2", "-d"],
        vec!["join-pane", "-s", "parity:2.0", "-t", "parity:1.0", "-d"],
        vec!["list-windows", "-F", "#{window_index} #{window_panes}"],
    ] {
        outputs.push(socket.run(&args));
    }
    outputs
}

#[test]
fn commands_a_private_socket_oracle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle = root.join("oracle/bin/tmux");
    let Some(binary) = std::env::var_os("RMUX_COMMANDS_A_BINARY").map(PathBuf::from) else {
        eprintln!("SKIP: set RMUX_COMMANDS_A_BINARY to built rmux");
        return;
    };
    if !oracle.is_file() {
        eprintln!("SKIP: pinned tmux oracle missing");
        return;
    }
    let temp = std::env::temp_dir().join(format!("rmux-g20-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("private socket directory");
    let actual = scenario(&binary, temp.join("rmux"));
    let expected = scenario(&oracle, temp.join("tmux"));
    let _ = std::fs::remove_dir_all(&temp);
    assert_eq!(actual, expected);
}
