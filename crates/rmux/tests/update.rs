//! End-to-end tests of `rmux update` against a fake release served over
//! 127.0.0.1, with a copy of the built binary standing in for the installed one.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const NEW: &str = "99.0.0";
const NEW_TAG: &str = "v99.0.0";

fn have(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

fn curl_missing() -> bool {
    let missing = !have("curl");
    if missing {
        eprintln!("SKIP: curl not on PATH");
    }
    missing
}

/// A temp dir removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rmux-upd-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Routes {
    latest: String,
    files: HashMap<String, Vec<u8>>,
    log: Vec<String>,
}

/// Minimal GitHub-releases lookalike.
struct Fake {
    base: String,
    routes: Arc<Mutex<Routes>>,
}

impl Fake {
    fn new(latest: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let routes = Arc::new(Mutex::new(Routes {
            latest: latest.to_owned(),
            ..Routes::default()
        }));
        let shared = Arc::clone(&routes);
        let origin = base.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = serve(stream, &origin, &shared);
            }
        });
        Self { base, routes }
    }

    fn put(&self, tag: &str, name: &str, bytes: &[u8]) {
        self.routes
            .lock()
            .unwrap()
            .files
            .insert(format!("/releases/download/{tag}/{name}"), bytes.to_vec());
    }

    fn publish(&self, release: &Release) {
        self.put(&release.tag, &release.asset, &release.tarball);
        self.put(&release.tag, "SHA256SUMS", release.sums.as_bytes());
    }

    fn log(&self) -> Vec<String> {
        self.routes.lock().unwrap().log.clone()
    }
}

fn serve(mut stream: TcpStream, origin: &str, routes: &Mutex<Routes>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim_end().is_empty() {
            break;
        }
    }
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let (status, extra, body) = {
        let mut routes = routes.lock().unwrap();
        routes.log.push(format!("{method} {path}"));
        if method != "GET" && method != "HEAD" {
            ("405 Method Not Allowed", String::new(), Vec::new())
        } else if path == "/releases/latest" {
            let location = format!("Location: {origin}/releases/tag/{}\r\n", routes.latest);
            ("302 Found", location, Vec::new())
        } else if path.starts_with("/releases/tag/") {
            ("200 OK", String::new(), b"release page\n".to_vec())
        } else if let Some(bytes) = routes.files.get(&path) {
            ("200 OK", String::new(), bytes.clone())
        } else {
            ("404 Not Found", String::new(), b"not found\n".to_vec())
        }
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n{extra}\r\n",
        body.len()
    )?;
    if method != "HEAD" {
        stream.write_all(&body)?;
    }
    stream.flush()
}

fn sha256_hex(path: &Path) -> String {
    let output = if have("shasum") {
        Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
    } else {
        Command::new("sha256sum").arg(path).output()
    }
    .unwrap();
    assert!(output.status.success(), "hashing {}", path.display());
    let text = String::from_utf8(output.stdout).unwrap();
    text.split_whitespace().next().unwrap().to_owned()
}

/// A fake release: a shell script named `rmux` in a tarball, plus SHA256SUMS.
struct Release {
    tag: String,
    asset: String,
    tarball: Vec<u8>,
    sums: String,
    script: Vec<u8>,
}

impl Release {
    fn build(dir: &Path, real: &Path, target: &str) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        let plugin = dir.join("plugin");
        let output = Command::new(real)
            .arg("omp-plugin")
            .arg(&plugin)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let script = format!(
            "#!/bin/sh\n\
             case \"$1\" in\n\
             version) echo 'rmux {NEW} (fake, {target})' ;;\n\
             omp-plugin) mkdir -p \"$2\" && cp '{p}/package.json' '{p}/rmux.ts' \"$2/\" && echo \"$2\" ;;\n\
             *) echo 'fake rmux' >&2; exit 1 ;;\n\
             esac\n",
            p = plugin.display()
        );
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let file = stage.join("rmux");
        std::fs::write(&file, &script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let asset = format!("rmux-{NEW_TAG}-{target}.tar.gz");
        let tarball = dir.join(&asset);
        // COPYFILE_DISABLE keeps macOS tar from adding AppleDouble `._rmux` entries.
        let status = Command::new("tar")
            .arg("-czf")
            .arg(&tarball)
            .arg("-C")
            .arg(&stage)
            .arg("rmux")
            .env("COPYFILE_DISABLE", "1")
            .status()
            .unwrap();
        assert!(status.success(), "tar failed");
        let sums = format!("{}  {asset}\n", sha256_hex(&tarball));
        Self {
            tag: NEW_TAG.to_owned(),
            tarball: std::fs::read(&tarball).unwrap(),
            asset,
            sums,
            script: script.into_bytes(),
        }
    }
}

/// One test's world: an installed copy of rmux plus an isolated socket dir.
struct World {
    tmp: Tmp,
    bin: PathBuf,
    original: Vec<u8>,
    mode: u32,
    version: String,
    target: String,
    sock: PathBuf,
}

impl World {
    fn new(name: &str) -> Self {
        let tmp = Tmp::new(name);
        let bin = install(&tmp.0.join("bin"));
        let sock = tmp.0.join("sock");
        std::fs::create_dir_all(&sock).unwrap();
        let output = Command::new(&bin).arg("version").output().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let text = stdout(&output);
        // The first line names the binary; the lines after it, one per live server of this user, name their builds.
        let line = text.lines().next().unwrap_or("");
        let (version, target) = parse_version(line.trim())
            .unwrap_or_else(|| panic!("unexpected `rmux version` output: {text:?}"));
        Self {
            original: std::fs::read(&bin).unwrap(),
            mode: std::fs::metadata(&bin).unwrap().permissions().mode(),
            tmp,
            bin,
            version,
            target,
            sock,
        }
    }

    fn release(&self) -> Release {
        Release::build(&self.tmp.0.join("release"), &self.bin, &self.target)
    }

    fn command(&self, fake: &Fake, bin: &Path) -> Command {
        let mut command = Command::new(bin);
        command
            .env("RMUX_UPDATE_BASE_URL", &fake.base)
            .env("RMUX_TMPDIR", &self.sock)
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .env_remove("RMUX")
            .env_remove("TMUX")
            .stdin(Stdio::null());
        command
    }

    fn update(&self, fake: &Fake, args: &[&str]) -> Output {
        self.command(fake, &self.bin)
            .arg("update")
            .args(args)
            .output()
            .unwrap()
    }

    fn assert_unchanged(&self) {
        assert!(
            std::fs::read(&self.bin).unwrap() == self.original,
            "installed rmux changed"
        );
        assert!(rollbacks(self.bin.parent().unwrap(), &self.version).is_empty());
        assert_eq!(leftovers(self.bin.parent().unwrap()), Vec::<String>::new());
    }
}

/// `rmux <ver> (<sha>, <target>)` → (ver, target).
fn parse_version(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("rmux ")?;
    let (version, rest) = rest.split_once(' ')?;
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    let (_, target) = inner.rsplit_once(", ")?;
    Some((version.to_owned(), target.to_owned()))
}

fn install(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let bin = dir.join("rmux");
    std::fs::copy(env!("CARGO_BIN_EXE_rmux"), &bin).unwrap();
    bin
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn describe(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout(output),
        stderr(output)
    )
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn rollbacks(dir: &Path, version: &str) -> Vec<PathBuf> {
    let suffix = format!("-pre-v{version}");
    names(dir)
        .into_iter()
        .filter(|name| name.starts_with("rmux.rollback-") && name.ends_with(&suffix))
        .map(|name| dir.join(name))
        .collect()
}

fn leftovers(dir: &Path) -> Vec<String> {
    names(dir)
        .into_iter()
        .filter(|name| name.starts_with(".rmux.new.") || name.starts_with(".rmux-update."))
        .collect()
}

fn downloads(fake: &Fake) -> Vec<String> {
    fake.log()
        .into_iter()
        .filter(|line| line.contains("/releases/download/"))
        .collect()
}

fn assert_installed(world: &World, target: &Path, release: &Release) {
    assert!(
        std::fs::read(target).unwrap() == release.script,
        "{} was not replaced",
        target.display()
    );
    let mode = std::fs::metadata(target).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o7777,
        world.mode & 0o7777,
        "mode of {}",
        target.display()
    );
    let dir = target.parent().unwrap();
    let saved = rollbacks(dir, &world.version);
    assert_eq!(
        saved.len(),
        1,
        "rollbacks in {}: {:?}",
        dir.display(),
        names(dir)
    );
    assert!(
        std::fs::read(&saved[0]).unwrap() == world.original,
        "rollback does not hold the original binary"
    );
    assert_eq!(leftovers(dir), Vec::<String>::new());
}

#[test]
fn update_installs_the_latest_release() {
    if curl_missing() {
        return;
    }
    let world = World::new("happy");
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.publish(&release);
    let output = world.update(&fake, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let out = stdout(&output);
    assert!(
        out.contains(&format!("installed rmux {NEW}")),
        "{}",
        describe(&output)
    );
    assert!(!out.contains("omp plugin"), "{}", describe(&output));
    assert!(
        !out.contains("running rmux server"),
        "{}",
        describe(&output)
    );
    assert_installed(&world, &world.bin, &release);
}

#[test]
fn check_reports_a_newer_release_without_installing() {
    if curl_missing() {
        return;
    }
    let world = World::new("check");
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.publish(&release);
    let output = world.update(&fake, &["--check"]);
    assert_eq!(output.status.code(), Some(10), "{}", describe(&output));
    assert_eq!(
        stdout(&output).trim_end(),
        format!("rmux {NEW} is available (installed {})", world.version)
    );
    assert_eq!(downloads(&fake), Vec::<String>::new());
    world.assert_unchanged();
}

#[test]
fn up_to_date_downloads_nothing() {
    if curl_missing() {
        return;
    }
    let world = World::new("current");
    let fake = Fake::new(&format!("v{}", world.version));
    let output = world.update(&fake, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(
        stdout(&output).trim_end(),
        format!("rmux {} is up to date", world.version)
    );
    assert!(
        fake.log()
            .iter()
            .any(|line| line.ends_with(" /releases/latest"))
    );
    assert_eq!(downloads(&fake), Vec::<String>::new());
    world.assert_unchanged();
}

#[test]
fn checksum_mismatch_changes_nothing() {
    if curl_missing() {
        return;
    }
    let world = World::new("sum");
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.publish(&release);
    let wrong = format!("{}  {}\n", "0".repeat(64), release.asset);
    fake.put(NEW_TAG, "SHA256SUMS", wrong.as_bytes());
    let output = world.update(&fake, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        stderr(&output).contains("checksum mismatch"),
        "{}",
        describe(&output)
    );
    world.assert_unchanged();
}

#[test]
fn missing_asset_is_reported() {
    if curl_missing() {
        return;
    }
    let world = World::new("missing");
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.put(NEW_TAG, "SHA256SUMS", release.sums.as_bytes());
    let output = world.update(&fake, &[]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        stderr(&output).contains("no rmux build for"),
        "{}",
        describe(&output)
    );
    world.assert_unchanged();
}

#[test]
fn version_flag_installs_that_tag_without_asking_for_latest() {
    if curl_missing() {
        return;
    }
    let world = World::new("pinned");
    let release = world.release();
    let fake = Fake::new("v98.0.0");
    fake.publish(&release);
    let output = world.update(&fake, &["--version", NEW_TAG]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        stdout(&output).contains(&format!("installed rmux {NEW}")),
        "{}",
        describe(&output)
    );
    assert!(
        !fake
            .log()
            .iter()
            .any(|line| line.contains("/releases/latest")),
        "{:?}",
        fake.log()
    );
    assert_installed(&world, &world.bin, &release);
}

#[test]
fn to_replaces_every_target() {
    if curl_missing() {
        return;
    }
    let world = World::new("to");
    let second = install(&world.tmp.0.join("other"));
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.publish(&release);
    let output = world.update(&fake, &["--to", second.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(
        stdout(&output)
            .matches(&format!("installed rmux {NEW} to "))
            .count(),
        2,
        "{}",
        describe(&output)
    );
    assert_installed(&world, &world.bin, &release);
    assert_installed(&world, &second, &release);
}

/// Kills the test server if the test did not.
struct ServerGuard(Option<rmux_sys::ProcessId>);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.0.take() {
            let _ = rmux_sys::client::kill(pid, rmux_sys::client::SIGTERM);
        }
    }
}

#[test]
fn running_servers_keep_the_old_build() {
    if curl_missing() {
        return;
    }
    let world = World::new("srv");
    let release = world.release();
    let fake = Fake::new(NEW_TAG);
    fake.publish(&release);

    let started = world
        .command(&fake, &world.bin)
        .args([
            "-L",
            "upd",
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "sleep 600",
        ])
        .output()
        .unwrap();
    assert!(started.status.success(), "{}", describe(&started));
    let shown = world
        .command(&fake, &world.bin)
        .args(["-L", "upd", "display-message", "-p", "#{pid}"])
        .output()
        .unwrap();
    assert!(shown.status.success(), "{}", describe(&shown));
    let pid = rmux_sys::ProcessId(stdout(&shown).trim().parse().unwrap());
    let mut guard = ServerGuard(Some(pid));

    let output = world.update(&fake, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let out = stdout(&output);
    assert!(
        out.contains("1 running rmux server(s) keep the previous build:"),
        "{}",
        describe(&output)
    );
    assert!(
        out.contains(&format!("(pid {}, started ", pid.0)),
        "{}",
        describe(&output)
    );
    assert_installed(&world, &world.bin, &release);
    assert!(
        rmux_sys::client::kill(pid, 0).is_ok(),
        "the old server died during the update"
    );

    let rollback = rollbacks(world.bin.parent().unwrap(), &world.version).remove(0);
    let killed = world
        .command(&fake, &rollback)
        .args(["-L", "upd", "kill-server"])
        .output()
        .unwrap();
    assert!(killed.status.success(), "{}", describe(&killed));
    let deadline = Instant::now() + Duration::from_secs(10);
    while rmux_sys::client::kill(pid, 0).is_ok() {
        assert!(
            Instant::now() < deadline,
            "server {} survived kill-server",
            pid.0
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    guard.0 = None;
}

#[test]
fn unknown_flag_prints_usage() {
    let world = World::new("bogus");
    let fake = Fake::new(NEW_TAG);
    let output = world.update(&fake, &["--bogus"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(
        stderr(&output).contains("usage: rmux update"),
        "{}",
        describe(&output)
    );
    assert!(fake.log().is_empty(), "{:?}", fake.log());
    world.assert_unchanged();
}
