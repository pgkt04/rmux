// Ported from tmux control.c, control-notify.c @ 8f25579c
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
struct ServerCleanup<'a> {
    binary: &'a Path,
    socket: &'a Path,
}
impl Drop for ServerCleanup<'_> {
    fn drop(&mut self) {
        let _ = Command::new(self.binary)
            .arg("-S")
            .arg(self.socket)
            .arg("kill-server")
            .output();
    }
}
fn transcript(binary: &Path, socket: &Path, regular_attach: bool) -> Vec<u8> {
    let _cleanup = ServerCleanup { binary, socket };
    let output_path = socket.with_extension("stream");
    // An empty shell command exits before attach when child startup is fast.
    if regular_attach {
        let status = Command::new(binary)
            .args(["-f", "/dev/null", "-S"])
            .arg(socket)
            .args([
                "new-session",
                "-d",
                "-s",
                "control-parity",
                "-n",
                "control-window",
                "exec sleep 100",
            ])
            .status()
            .expect("detached session creation");
        assert!(status.success());
    }
    let mut command = Command::new(binary);
    command.args(["-f", "/dev/null", "-S"]).arg(socket);
    if regular_attach {
        command.args(["-C", "attach-session", "-t", "control-parity"]);
    } else {
        command.args([
            "-C",
            "new-session",
            "-s",
            "control-parity",
            "-n",
            "control-window",
            "exec sleep 100",
        ]);
    }
    let stdout = if regular_attach {
        Stdio::from(fs::File::create(&output_path).unwrap())
    } else {
        Stdio::piped()
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .expect("control process");
    let mut input = child.stdin.take().unwrap();
    // Commands have no pane output, hence no timing-dependent output grouping.
    input.write_all(b"display-message -p CONTROL_VALUE\ndisplay-message -p '#{session_name}'\nnot-a-command\ndisplay-message -p '\nrefresh-client -A '%0:pause'\nrefresh-client -A '%0:continue'\nrename-session renamed\nset-buffer -b control-buffer value\ndelete-buffer -b control-buffer\ndetach-client\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("control client timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    let stream = if regular_attach {
        fs::read(output_path).unwrap()
    } else {
        output.stdout
    };
    assert!(
        !stream.is_empty(),
        "empty control transcript: {:?}",
        output.stderr
    );
    stream
}
fn normalize(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut guards = Vec::new();
    let mut numbers = std::collections::BTreeMap::new();
    let mut base = None;
    for line in bytes.split(|b| *b == b'\n') {
        if line.starts_with(b"%begin ") {
            guards.push(line[7..].to_vec());
            let number: u32 =
                line.split(|b| *b == b' ')
                    .nth(2)
                    .unwrap()
                    .iter()
                    .fold(0u32, |n, b| {
                        n.checked_mul(10)
                            .and_then(|n| n.checked_add(u32::from(*b - b'0')))
                            .expect("guard number")
                    });
            let first = *base.get_or_insert(number);
            numbers.insert(
                number.to_string().into_bytes(),
                number.checked_sub(first).expect("ordered guard numbers"),
            );
        } else if line.starts_with(b"%end ") || line.starts_with(b"%error ") {
            let metadata = line.splitn(2, |b| *b == b' ').nth(1).unwrap();
            assert_eq!(
                guards.pop().as_deref(),
                Some(metadata),
                "guard pair metadata mismatch"
            );
        }
    }
    assert!(guards.is_empty(), "unclosed command guard");
    bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            if line.starts_with(b"%begin ")
                || line.starts_with(b"%end ")
                || line.starts_with(b"%error ")
            {
                let fields: Vec<_> = line.split(|b| *b == b' ').collect();
                assert_eq!(fields.len(), 4);
                let mut out = fields[0].to_vec();
                out.extend_from_slice(b" TIME ");
                out.extend_from_slice(numbers[fields[2]].to_string().as_bytes());
                out.push(b' ');
                out.extend_from_slice(fields[3]);
                out
            } else {
                line.to_vec()
            }
        })
        .collect()
}
#[test]
fn command_stream_matches_pinned_oracle() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let oracle = root.join("oracle/bin/tmux");
    let binary = if let Some(binary) = std::env::var_os("RMUX_CONTROL_BINARY") {
        std::path::PathBuf::from(binary)
    } else {
        let executable = std::env::current_exe().expect("control test executable");
        let profile = executable
            .parent()
            .and_then(Path::parent)
            .expect("Cargo test profile directory");
        let target = profile.parent().expect("Cargo target directory");
        let mut build = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        build
            .args(["build", "--package", "rmux", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(target);
        if profile.file_name().is_some_and(|name| name == "release") {
            build.arg("--release");
        }
        let output = build
            .output()
            .expect("building current-target control binary");
        assert!(
            output.status.success(),
            "current-target rmux build failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        profile.join("rmux")
    };
    if !oracle.exists() {
        eprintln!("SKIP control differential: pinned tmux oracle missing");
        return;
    }
    assert!(
        binary.is_file(),
        "control binary missing: {}",
        binary.display()
    );
    let dir = std::env::temp_dir().join(format!("rmux-control-oracle-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let expected = normalize(&transcript(&oracle, &dir.join("oracle.sock"), false));
    let actual = normalize(&transcript(&binary, &dir.join("rmux.sock"), false));
    // Prove this comparator rejects a byte difference without touching sources.
    let mut corrupted = actual.clone();
    corrupted.push(b"unexpected notification".to_vec());
    assert_ne!(expected, corrupted);
    assert_eq!(
        expected, actual,
        "control stream differs (only guard times/numbers normalized)"
    );
    let expected = normalize(&transcript(&oracle, &dir.join("oracle-regular.sock"), true));
    let actual = normalize(&transcript(&binary, &dir.join("rmux-regular.sock"), true));
    assert_eq!(
        expected, actual,
        "existing-session control attach to regular stdout differs"
    );
    fs::remove_dir_all(dir).unwrap();
}
