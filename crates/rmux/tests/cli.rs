// Ported from tmux tmux.c @ 8f25579c (CLI behaviour differential against the oracle)
//! Spec g15_client.md section 6, differential test 1: the CLI matrix. The
//! oracle (`oracle/bin/tmux`) output is mapped `tmux` -> `rmux` and
//! `TMUX_TMPDIR`/`tmux-<uid>` -> `RMUX_TMPDIR`/`rmux-<uid>` before comparison.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn oracle() -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux");
    path.is_file().then_some(path)
}

fn rmux() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rmux"))
}

fn run(bin: &Path, args: &[&str], env: &[(&str, &str)], clear: &[&str]) -> Output {
    let mut command = Command::new(bin);
    command.args(args);
    for name in clear {
        command.env_remove(name);
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("spawn")
}

fn map(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("tmux: ", "rmux: ")
        .replace("usage: tmux", "usage: rmux")
        .replace("tmux next-", "rmux next-")
        .replace("tmux-", "rmux-")
        .replace("TMUX_TMPDIR", "RMUX_TMPDIR")
}

/// Compare one invocation: status, mapped stdout and mapped stderr.
fn compare(args: &[&str], tmp: &Path) {
    let Some(oracle) = oracle() else {
        eprintln!("skipping: oracle binary missing");
        return;
    };
    let clear = ["TMUX", "RMUX", "TMUX_TMPDIR", "RMUX_TMPDIR"];
    let tmp = tmp.to_str().unwrap();
    let theirs = run(&oracle, args, &[("TMUX_TMPDIR", tmp)], &clear);
    let ours = run(&rmux(), args, &[("RMUX_TMPDIR", tmp)], &clear);
    assert_eq!(
        ours.status.code(),
        theirs.status.code(),
        "exit status for {args:?}"
    );
    assert_eq!(
        map(&ours.stdout),
        map(&theirs.stdout),
        "stdout for {args:?}"
    );
    assert_eq!(
        map(&ours.stderr),
        map(&theirs.stderr),
        "stderr for {args:?}"
    );
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmux-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn socket_path_errors_match_the_oracle() {
    let Some(oracle) = oracle() else {
        eprintln!("skipping: oracle binary missing");
        return;
    };
    let clear = ["TMUX", "RMUX", "TMUX_TMPDIR", "RMUX_TMPDIR"];
    let cases: &[(&str, &[&str])] = &[
        ("relative", &["-N", "ls"]),
        ("/a/../b", &["-N", "ls"]),
        ("/nonexistent/rmux-cli-dir", &["-N", "ls"]),
    ];
    for (dir, args) in cases {
        let theirs = run(&oracle, args, &[("TMUX_TMPDIR", dir)], &clear);
        let ours = run(&rmux(), args, &[("RMUX_TMPDIR", dir)], &clear);
        assert_eq!(ours.status.code(), theirs.status.code(), "status for {dir}");
        assert_eq!(map(&ours.stderr), map(&theirs.stderr), "stderr for {dir}");
    }
    // Not a directory, then unsafe permissions.
    let base = temp_dir("sock");
    let uid = rmux_sys::proc::getuid().0;
    std::fs::write(base.join(format!("tmux-{uid}")), b"").unwrap();
    std::fs::write(base.join(format!("rmux-{uid}")), b"").unwrap();
    let dir = base.to_str().unwrap();
    let theirs = run(&oracle, &["-N", "ls"], &[("TMUX_TMPDIR", dir)], &clear);
    let ours = run(&rmux(), &["-N", "ls"], &[("RMUX_TMPDIR", dir)], &clear);
    assert_eq!(ours.status.code(), theirs.status.code());
    assert_eq!(map(&ours.stderr), map(&theirs.stderr));

    let base = temp_dir("perm");
    use std::os::unix::fs::PermissionsExt;
    for name in [format!("tmux-{uid}"), format!("rmux-{uid}")] {
        let d = base.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o707)).unwrap();
    }
    let dir = base.to_str().unwrap();
    let theirs = run(&oracle, &["-N", "ls"], &[("TMUX_TMPDIR", dir)], &clear);
    let ours = run(&rmux(), &["-N", "ls"], &[("RMUX_TMPDIR", dir)], &clear);
    assert_eq!(ours.status.code(), theirs.status.code());
    assert_eq!(map(&ours.stderr), map(&theirs.stderr));
}

#[test]
fn no_server_running_text_matches_the_oracle() {
    let tmp = temp_dir("noserver");
    // -N: never start a server; the connect error text is printed.
    compare(&["-N", "-L", "cli-none", "ls"], &tmp);
}

#[test]
fn idle_server_handles_sigterm_inherited_blocked_and_ignored() {
    const CHILD: &str = "RMUX_CLI_SIGNAL_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "idle_server_handles_sigterm_inherited_blocked_and_ignored",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let dir = temp_dir("inherited-signals");
    let socket = dir.join("socket");
    let invoke = |args: &[&str]| {
        Command::new(rmux())
            .args(["-S", socket.to_str().unwrap(), "-f", "/dev/null"])
            .args(args)
            .env_remove("RMUX")
            .env_remove("TMUX")
            .output()
            .unwrap()
    };
    let mask = rmux_sys::server::SignalMask::block().unwrap();
    rmux_sys::client::set_signal_disposition(
        rmux_sys::client::SIGTERM,
        rmux_sys::client::Disposition::Ignore,
    )
    .unwrap();
    let started = invoke(&["new-session", "-d", "-s", "idle", "sleep 120"]);
    drop(mask);
    rmux_sys::client::set_signal_disposition(
        rmux_sys::client::SIGTERM,
        rmux_sys::client::Disposition::Default,
    )
    .unwrap();
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let output = invoke(&["display-message", "-p", "#{pid}"]);
    let pid = rmux_sys::ProcessId(
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
    rmux_sys::client::kill(pid, rmux_sys::client::SIGTERM).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while rmux_sys::client::kill(pid, 0).is_ok() {
        if std::time::Instant::now() >= deadline {
            let _ = invoke(&["kill-server"]);
            let _ = rmux_sys::proc::terminate_process(pid);
            panic!("idle server kept inherited blocked/ignored SIGTERM");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unanswered_terminal_draws_small_screen_before_probe_deadline() {
    use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
    use std::os::fd::AsFd;
    use std::time::{Duration, Instant};

    for enabled in [false, true] {
        let dir = temp_dir(if enabled {
            "unanswered-on"
        } else {
            "unanswered-off"
        });
        let socket = dir.join("socket");
        let invoke = |args: &[&str]| {
            Command::new(rmux())
                .args(["-S", socket.to_str().unwrap(), "-f", "/dev/null"])
                .args(args)
                .env_remove("RMUX")
                .env_remove("TMUX")
                .env("RMUX_TSP_BROKER", if enabled { "1" } else { "0" })
                .output()
                .unwrap()
        };
        let started = invoke(&[
            "new-session",
            "-d",
            "-s",
            "plain",
            "-x",
            "32",
            "-y",
            "6",
            "printf 'SCREEN-0\\r\\nSCREEN-1\\r\\nSCREEN-2\\r\\nSCREEN-3\\r\\nSCREEN-4\\r\\nSCREEN-5'; exec sleep 60",
            ";",
            "set-option",
            "-g",
            "status",
            "off",
            ";",
            "set-option",
            "-g",
            "window-size",
            "manual",
            ";",
            "set-option",
            "-g",
            "terminal-features",
            "screen*:sync:clipboard",
        ]);
        assert!(started.status.success(), "{:?}", started.stderr);
        let ready = Instant::now() + Duration::from_secs(5);
        while !invoke(&["capture-pane", "-p"])
            .stdout
            .windows(8)
            .any(|bytes| bytes == b"SCREEN-5")
        {
            if Instant::now() >= ready {
                let _ = invoke(&["kill-server"]);
                panic!("pane did not produce its full grid");
            }
        }
        let launch = PreparedLaunch::new(LaunchOptions {
            shell: b"/bin/sh".to_vec(),
            argv: vec![
                rmux().as_os_str().as_encoded_bytes().to_vec(),
                b"-S".to_vec(),
                socket.as_os_str().as_encoded_bytes().to_vec(),
                b"-f".to_vec(),
                b"/dev/null".to_vec(),
                b"attach-session".to_vec(),
                b"-t".to_vec(),
                b"plain".to_vec(),
            ],
            environment: vec![
                b"TERM=screen-256color".to_vec(),
                b"LC_ALL=C.UTF-8".to_vec(),
                b"PATH=/bin:/usr/bin".to_vec(),
                b"HOME=/tmp".to_vec(),
            ],
            cwd: b"/".to_vec(),
            home: Some(b"/tmp".to_vec()),
            termios: None,
            backspace: 0x7f,
            size: Winsize {
                rows: 6,
                cols: 32,
                xpixel: 0,
                ypixel: 0,
            },
        })
        .unwrap();
        let child = launch.launch().unwrap();
        let mut wire = Vec::new();
        let mut error = None;
        let startup_deadline = Instant::now() + Duration::from_secs(10);
        let mut tty_started = None;
        let complete = |wire: &[u8]| {
            (0..6).all(|row| {
                let marker = format!("SCREEN-{row}");
                wire.windows(marker.len())
                    .any(|bytes| bytes == marker.as_bytes())
            })
        };
        while !complete(&wire) {
            if tty_started
                .is_some_and(|start: Instant| start.elapsed() >= Duration::from_millis(500))
                || tty_started.is_none() && Instant::now() >= startup_deadline
            {
                break;
            }
            let mut buffer = [0; 8192];
            match rmux_sys::fd::read(child.master.as_fd(), &mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    tty_started.get_or_insert_with(Instant::now);
                    wire.extend_from_slice(&buffer[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        let elapsed = tty_started.map(|start| start.elapsed());
        let _ = invoke(&["kill-server"]);
        let _ = rmux_sys::proc::terminate_process(child.pid);
        let _ = rmux_sys::proc::wait_process(child.pid, false);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            error.is_none() && complete(&wire),
            "broker={enabled} no-answer screen after {elapsed:?}: error={error:?}, wire={:?}",
            String::from_utf8_lossy(&wire)
        );
        assert_eq!(
            wire.windows(6).any(|bytes| bytes == b"\x1b_tsp;"),
            enabled,
            "disabled broker must send no TSP probe"
        );
    }
}
