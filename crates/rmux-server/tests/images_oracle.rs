//! Compare pane DA and XTSMGRAPHICS replies with both pinned feature builds.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

struct Mux {
    binary: PathBuf,
    directory: PathBuf,
}
impl Mux {
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.binary)
            .arg("-S")
            .arg(self.directory.join("socket"))
            .arg("-f")
            .arg("/dev/null")
            .args(args)
            .env("TERM", "screen")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .output()
            .unwrap()
    }
}
impl Drop for Mux {
    fn drop(&mut self) {
        let _ = Command::new(&self.binary)
            .arg("-S")
            .arg(self.directory.join("socket"))
            .arg("-f")
            .arg("/dev/null")
            .arg("kill-server")
            .env("TERM", "screen")
            .env_remove("TMUX")
            .env_remove("RMUX")
            .output();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn capture(binary: &Path, tag: &str) -> Vec<u8> {
    let directory = std::env::temp_dir().join(format!("rmux-images-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mux = Mux {
        binary: binary.to_owned(),
        directory,
    };
    let script = mux.directory.join("reply.py");
    let result = mux.directory.join("reply.bin");
    // Use the pane's raw tty, not capture-pane: replies are input bytes.
    std::fs::write(
        &script,
        r#"import os, select, sys, termios, tty, time
old = termios.tcgetattr(0)
tty.setraw(0)
out = bytearray()
try:
    for query in [b'\x1b[c', b'\x1b[?1;1S', b'\x1b[?1;2S', b'\x1b[?1;3;512S', b'\x1b[?1;4S']:
        os.write(1, query)
        reply = bytearray()
        until = time.monotonic() + 1
        while time.monotonic() < until:
            if select.select([0], [], [], max(0, until - time.monotonic()))[0]:
                reply.extend(os.read(0, 4096))
                if reply.endswith((b'c', b'S')):
                    break
        out.extend(len(reply).to_bytes(4, 'little'))
        out.extend(reply)
    open(sys.argv[1], 'wb').write(out)
    time.sleep(10)
finally:
    termios.tcsetattr(0, termios.TCSANOW, old)
"#,
    )
    .unwrap();
    let shell = format!("python3 '{}' '{}'", script.display(), result.display());
    let started = mux.run(&["new-session", "-d", "-s", "images", &shell]);
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(bytes) = std::fs::read(&result) {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "pane reply fixture timed out: {}",
            binary.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn rmux_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RMUX_BIN") {
        return Some(PathBuf::from(path));
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let binary = target.join("debug/rmux");
    binary.is_file().then_some(binary)
}
#[test]
fn graphics_capability_replies_match_pinned_build_configuration() {
    let enabled = cfg!(feature = "sixel");
    let oracle = if enabled {
        std::env::var_os("RMUX_SIXEL_ORACLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp/swarm-rmux-build/oracle-sixel/tmux"))
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux")
    };
    let Some(rmux) = rmux_path().filter(|p| p.is_file()) else {
        eprintln!("SKIP image replies: build matching rmux binary or set RMUX_BIN");
        return;
    };
    if !oracle.is_file() {
        eprintln!(
            "SKIP image replies: pinned oracle missing at {}",
            oracle.display()
        );
        return;
    }
    if !Command::new("python3")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP image replies: python3 raw-tty fixture unavailable");
        return;
    }
    let expected = capture(&oracle, "oracle");
    let mut actual = capture(&rmux, "rmux");
    if std::env::var_os("RMUX_IMAGES_ORACLE_MUTATE").is_some() {
        actual.push(0);
    }
    assert_eq!(
        actual, expected,
        "DA/XTSMGRAPHICS wire mismatch; sixel={enabled}"
    );
    let replies: [&[u8]; 5] = if enabled {
        [
            b"\x1b[?1;2;4c",
            b"\x1b[?1;0;1024S",
            b"\x1b[?1;0;1024S",
            b"\x1b[?1;3;512S",
            b"\x1b[?1;0;1024S",
        ]
    } else {
        [b"\x1b[?1;2c", b"", b"", b"", b""]
    };
    let mut framed = Vec::new();
    for reply in replies {
        framed.extend_from_slice(&(reply.len() as u32).to_le_bytes());
        framed.extend_from_slice(reply);
    }
    assert_eq!(
        expected, framed,
        "oracle was not built with sixel={enabled}"
    );
}

#[cfg(feature = "sixel")]
fn outer_capture(
    binary: &Path,
    tag: &str,
    clients: &[(bool, (u16, u16))],
    clipped: bool,
) -> Vec<Vec<u8>> {
    use rmux_sys::pty::{LaunchOptions, PreparedLaunch, Winsize};
    use std::os::fd::AsFd;
    let directory =
        std::env::temp_dir().join(format!("rmux-image-output-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let mux = Mux {
        binary: binary.to_owned(),
        directory,
    };
    let source = mux.directory.join("image.py");
    std::fs::write(&source, r#"import os, time
time.sleep(1)
os.write(1, b'READY')
time.sleep(1)
os.write(1, b'\x1b[1;8H\x1bP0;2q"1;1;32;64#0;2;100;0;0' + b'!32~-!32~-!32~-!32~-!32~-!32~-!32~-!32~-!32~-!32~-!32N\x1b\\')
time.sleep(30)
"#).unwrap();
    let shell = format!("python3 '{}'", source.display());
    assert!(
        mux.run(&[
            "new-session",
            "-d",
            "-s",
            "images",
            "-x",
            "8",
            "-y",
            "4",
            &shell
        ])
        .status
        .success()
    );
    for args in [
        vec!["set-option", "-g", "status", "off"],
        vec![
            "set-option",
            "-g",
            "terminal-overrides",
            "screen:Sxl@,xterm:Sxl",
        ],
        vec!["set-option", "-g", "terminal-features", ""],
    ] {
        assert!(mux.run(&args).status.success());
    }
    if clipped {
        assert!(
            mux.run(&["set-window-option", "window-size", "manual"])
                .status
                .success()
        );
        assert!(
            mux.run(&["resize-window", "-x", "10", "-y", "6"])
                .status
                .success()
        );
    }
    let launches: Vec<_> = clients
        .iter()
        .map(|&(capable, metrics)| {
            PreparedLaunch::new(LaunchOptions {
                shell: b"/bin/sh".to_vec(),
                argv: vec![
                    binary.as_os_str().as_encoded_bytes().to_vec(),
                    b"-S".to_vec(),
                    mux.directory
                        .join("socket")
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                    b"-f".to_vec(),
                    b"/dev/null".to_vec(),
                    b"attach-session".to_vec(),
                ],
                environment: vec![
                    if capable {
                        b"TERM=xterm".to_vec()
                    } else {
                        b"TERM=screen".to_vec()
                    },
                    b"PATH=/bin:/usr/bin:/usr/local/bin".to_vec(),
                    b"HOME=/tmp".to_vec(),
                    b"LC_ALL=C.UTF-8".to_vec(),
                ],
                cwd: b"/".to_vec(),
                home: Some(b"/tmp".to_vec()),
                termios: None,
                backspace: 0x7f,
                size: Winsize {
                    rows: 4,
                    cols: 8,
                    xpixel: metrics.0,
                    ypixel: metrics.1,
                },
            })
            .unwrap()
            .launch()
            .unwrap()
        })
        .collect();
    let mut streams = vec![Vec::new(); clients.len()];
    let mut ready = vec![false; clients.len()];
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut buffer = [0; 8192];
    while Instant::now() < deadline {
        for (index, launch) in launches.iter().enumerate() {
            match rmux_sys::fd::read(launch.master.as_fd(), &mut buffer) {
                Ok(n) => {
                    streams[index].extend_from_slice(&buffer[..n]);
                    if !ready[index] && streams[index].windows(5).any(|w| w == b"READY") {
                        streams[index].clear();
                        ready[index] = true;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(mux.run(&["detach-client", "-s", "images"]).status.success());
    for launch in launches {
        let _ = rmux_sys::proc::wait_process(launch.pid, false);
    }
    streams
        .into_iter()
        .zip(clients)
        .zip(ready)
        .map(|((bytes, &(capable, metrics)), armed)| {
            assert!(armed, "image readiness missing: binary={} capable={capable} metrics={metrics:?} clipped={clipped} bytes={bytes:?}", binary.display());
            let marker: &[u8] = if capable && metrics.0 != 0 && metrics.1 != 0 {
                b"\x1bP9;2q"
            } else {
                b"SIXEL IMAGE"
            };
            let start = bytes
                .windows(marker.len())
                .position(|w| w == marker)
                .unwrap_or_else(|| panic!("image output missing: binary={} capable={capable} metrics={metrics:?} clipped={clipped} bytes={bytes:?}", binary.display()));
            let end = if capable && metrics.0 != 0 && metrics.1 != 0 {
                start
                    + bytes[start..]
                        .windows(2)
                        .position(|w| w == b"\x1b\\")
                        .unwrap()
                    + 2
            } else {
                let label_end = start
                    + bytes[start..]
                        .windows(2)
                        .position(|w| w == b"\r\n")
                        .unwrap()
                    + 2;
                let label = std::str::from_utf8(&bytes[start..label_end]).unwrap();
                let rows: usize = label
                    .trim()
                    .trim_end_matches(')')
                    .rsplit_once('x')
                    .unwrap()
                    .1
                    .parse()
                    .unwrap();
                let mut end = label_end;
                for _ in 1..rows {
                    end += bytes[end..].windows(2).position(|w| w == b"\r\n").unwrap() + 2;
                }
                end
            };
            bytes[..end].to_vec()
        })
        .collect()
}

#[cfg(feature = "sixel")]
#[test]
fn outer_sixel_and_fallback_match_enabled_oracle() {
    let oracle = std::env::var_os("RMUX_SIXEL_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/swarm-rmux-build/oracle-sixel/tmux"));
    let Some(rmux) = rmux_path().filter(|p| p.is_file()) else {
        eprintln!("SKIP outer SIXEL: matching RMUX_BIN missing");
        return;
    };
    if !oracle.is_file()
        || !Command::new("python3")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP outer SIXEL: enabled pinned oracle/python3 missing");
        return;
    }
    for (capable, clipped, metrics) in [
        (true, false, (128, 128)),
        (false, false, (128, 128)),
        (true, true, (128, 128)),
        (false, true, (128, 128)),
        (true, false, (0, 0)),
        (true, false, (64, 64)),
    ] {
        let expected = outer_capture(&oracle, "oracle", &[(capable, metrics)], clipped);
        let mut actual = outer_capture(&rmux, "rmux", &[(capable, metrics)], clipped);
        if std::env::var_os("RMUX_IMAGES_ORACLE_MUTATE").is_some() {
            actual[0].push(0);
        }
        assert_eq!(
            actual, expected,
            "outer image wire mismatch capable={capable} clipped={clipped} metrics={metrics:?}"
        );
    }
    let clients = [(true, (128, 128)), (false, (128, 128)), (true, (64, 64))];
    let expected = outer_capture(&oracle, "oracle-multi", &clients, false);
    let mut actual = outer_capture(&rmux, "rmux-multi", &clients, false);
    if std::env::var_os("RMUX_IMAGES_ORACLE_MUTATE").is_some() {
        actual[0].push(0);
    }
    assert_eq!(
        actual, expected,
        "mixed-capability simultaneous client image output"
    );
}
