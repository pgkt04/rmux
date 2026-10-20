//! End-to-end tests of `rmux install-plugin`, `update-plugin` and
//! `uninstall-plugin` against a temp HOME and a fake `omp` on PATH.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PACKAGE_JSON: &[u8] =
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/omp/package.json"));
const RMUX_TS: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/omp/rmux.ts"));

const OLD_PACKAGE_JSON: &str = "{\n  \"name\": \"rmux-terminal\",\n  \"version\": \"0.0.1\"\n}\n";
const OLD_RMUX_TS: &str = "// old plugin\n";

/// The `"version"` value of the bundled package.json.
fn bundled_version() -> String {
    let package: serde_json::Value = serde_json::from_slice(PACKAGE_JSON).unwrap();
    package["version"].as_str().expect("version").to_owned()
}

/// A temp dir removed on drop.
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rmux-plugin-{}-{name}", std::process::id()));
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

#[derive(Clone, Copy, PartialEq)]
enum Omp {
    /// A fake omp that emulates install/list/uninstall.
    Working,
    /// A fake omp whose `plugin install` fails.
    FailingInstall,
    /// A fake omp whose `plugin uninstall` fails.
    FailingUninstall,
    /// No omp on PATH.
    Missing,
}

/// One test's world: HOME is the temp dir, PATH holds the fake omp (or none).
struct World {
    tmp: Tmp,
    omp: Omp,
}

impl World {
    fn new(name: &str, omp: Omp) -> Self {
        let tmp = Tmp::new(name);
        let world = Self { tmp, omp };
        std::fs::create_dir_all(world.bin()).unwrap();
        if omp != Omp::Missing {
            world.write_fake_omp();
        }
        world
    }

    fn home(&self) -> &Path {
        &self.tmp.0
    }

    fn bin(&self) -> PathBuf {
        self.home().join("bin")
    }

    fn log_path(&self) -> PathBuf {
        self.home().join("omp.log")
    }

    fn link(&self) -> PathBuf {
        self.home().join(".omp/plugins/node_modules/rmux-terminal")
    }

    fn default_dir(&self) -> PathBuf {
        self.home().join(".omp/local-plugins/rmux-terminal")
    }

    fn write_fake_omp(&self) {
        let install = if self.omp == Omp::FailingInstall {
            "echo \"boom: install failed\" >&2; exit 1"
        } else {
            "mkdir -p \"$(dirname \"$link\")\" && rm -f \"$link\" && ln -s \"$3\" \"$link\" \
             && echo \"Linked rmux-terminal from $3\"; exit 0"
        };
        let uninstall = if self.omp == Omp::FailingUninstall {
            "echo \"boom: uninstall failed\" >&2; exit 1"
        } else {
            "if [ -L \"$link\" ]; then rm -f \"$link\"; echo \"Uninstalled $3\"; exit 0; fi\n  \
             echo \"rmux-terminal is not installed\" >&2; exit 1"
        };
        let script = format!(
            r#"#!/bin/sh
export PATH=/usr/bin:/bin
printf '%s\n' "$*" >> '{log}'
link='{link}'
case "$1 $2" in
"plugin install")
  {install};;
"plugin list")
  [ "$3" = "--json" ] || {{ echo "fake omp: list needs --json" >&2; exit 2; }}
  if [ -L "$link" ]; then
    printf '{{"npm":[{{"name":"rmux-terminal","version":"0.1.2","path":"%s"}}],"marketplace":[]}}\n' "$link"
  else
    echo '{{"npm":[],"marketplace":[]}}'
  fi
  exit 0;;
"plugin uninstall")
  {uninstall};;
esac
echo "fake omp: unexpected: $*" >&2
exit 2
"#,
            log = self.log_path().display(),
            link = self.link().display(),
        );
        let path = self.bin().join("omp");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rmux"));
        command
            .args(args)
            .env("HOME", self.home())
            .env("PATH", self.bin())
            .current_dir(self.home())
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Lines the fake omp logged, one per invocation.
    fn omp_log(&self) -> Vec<String> {
        std::fs::read_to_string(self.log_path())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn write_stale(&self, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("package.json"), OLD_PACKAGE_JSON).unwrap();
        std::fs::write(dir.join("rmux.ts"), OLD_RMUX_TS).unwrap();
    }

    fn write_bundled(&self, dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("package.json"), PACKAGE_JSON).unwrap();
        std::fs::write(dir.join("rmux.ts"), RMUX_TS).unwrap();
    }
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

fn assert_bundled(dir: &Path) {
    assert_eq!(
        std::fs::read(dir.join("package.json")).unwrap(),
        PACKAGE_JSON
    );
    assert_eq!(std::fs::read(dir.join("rmux.ts")).unwrap(), RMUX_TS);
}

fn assert_stale(dir: &Path) {
    assert_eq!(
        std::fs::read_to_string(dir.join("package.json")).unwrap(),
        OLD_PACKAGE_JSON
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("rmux.ts")).unwrap(),
        OLD_RMUX_TS
    );
}

fn install_line(dir: &Path) -> String {
    format!("plugin install {}", dir.display())
}

#[test]
fn install_writes_default_dir_and_registers_with_omp() {
    let world = World::new("install-default", Omp::Working);
    let dir = world.default_dir();
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
    assert!(world.link().is_symlink());
    let out = stdout(&output);
    let ver = bundled_version();
    assert!(
        out.contains(&format!(
            "installed rmux-terminal {ver} to {}",
            dir.display()
        )),
        "{}",
        describe(&output)
    );
    assert!(out.contains("start a fresh omp"), "{}", describe(&output));
}

#[test]
fn install_honors_dir_flag() {
    let world = World::new("install-dir", Omp::Working);
    let dir = world.home().join("custom/place");
    let dir_arg = dir.to_str().unwrap();
    let output = world.run(&["install-plugin", "--dir", dir_arg]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    assert!(!world.default_dir().exists());
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
    assert!(stdout(&output).contains(dir_arg), "{}", describe(&output));
}

#[test]
fn install_without_omp_writes_files_and_fails() {
    let world = World::new("install-no-omp", Omp::Missing);
    let dir = world.default_dir();
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert_bundled(&dir);
    let err = stderr(&output);
    assert!(err.contains("omp plugin install"), "{}", describe(&output));
    assert!(
        err.contains(&dir.display().to_string()),
        "{}",
        describe(&output)
    );
}

#[test]
fn install_reports_omp_failure() {
    let world = World::new("install-omp-fails", Omp::FailingInstall);
    let dir = world.default_dir();
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert_bundled(&dir);
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
    assert!(stderr(&output).contains("boom"), "{}", describe(&output));
}

#[test]
fn update_when_not_installed_fails_without_creating_dir() {
    let world = World::new("update-missing", Omp::Working);
    let dir = world.default_dir();
    for args in [&["update-plugin"][..], &["update-plugin", "--check"][..]] {
        let output = world.run(args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {}",
            describe(&output)
        );
        assert!(
            stderr(&output).contains("rmux install-plugin"),
            "{args:?}: {}",
            describe(&output)
        );
        assert!(!dir.exists(), "{args:?}");
    }
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn update_up_to_date_does_nothing() {
    let world = World::new("update-current", Omp::Working);
    let dir = world.default_dir();
    world.write_bundled(&dir);
    let output = world.run(&["update-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let ver = bundled_version();
    assert!(
        stdout(&output).contains(&format!("rmux-terminal {ver} is up to date")),
        "{}",
        describe(&output)
    );
    assert_bundled(&dir);
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn update_check_up_to_date_succeeds() {
    let world = World::new("update-check-current", Omp::Working);
    let dir = world.default_dir();
    world.write_bundled(&dir);
    let output = world.run(&["update-plugin", "--check"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        stdout(&output).contains("is up to date"),
        "{}",
        describe(&output)
    );
    assert_bundled(&dir);
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn update_detects_rmux_ts_only_change() {
    let world = World::new("update-ts-only", Omp::Working);
    let dir = world.default_dir();
    world.write_bundled(&dir);
    std::fs::write(dir.join("rmux.ts"), OLD_RMUX_TS).unwrap();
    let ver = bundled_version();

    let output = world.run(&["update-plugin", "--check"]);
    assert_eq!(output.status.code(), Some(10), "{}", describe(&output));
    assert!(
        stdout(&output).contains(&format!("is available (installed {ver})")),
        "{}",
        describe(&output)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("rmux.ts")).unwrap(),
        OLD_RMUX_TS
    );
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());

    let output = world.run(&["update-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
}

#[test]
fn update_refuses_foreign_package() {
    let world = World::new("update-foreign", Omp::Working);
    let dir = world.default_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let package = "{\"name\": \"other\", \"version\": \"1.0.0\"}\n";
    std::fs::write(dir.join("package.json"), package).unwrap();
    std::fs::write(dir.join("rmux.ts"), OLD_RMUX_TS).unwrap();
    for args in [&["update-plugin"][..], &["update-plugin", "--check"][..]] {
        let output = world.run(args);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {}",
            describe(&output)
        );
        assert!(
            stderr(&output).contains("rmux install-plugin"),
            "{args:?}: {}",
            describe(&output)
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("package.json")).unwrap(),
            package
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("rmux.ts")).unwrap(),
            OLD_RMUX_TS
        );
    }
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn update_check_reports_stale_without_writing() {
    let world = World::new("update-check", Omp::Working);
    let dir = world.default_dir();
    world.write_stale(&dir);
    let output = world.run(&["update-plugin", "--check"]);
    assert_eq!(output.status.code(), Some(10), "{}", describe(&output));
    let ver = bundled_version();
    assert!(
        stdout(&output).contains(&format!(
            "rmux-terminal {ver} is available (installed 0.0.1)"
        )),
        "{}",
        describe(&output)
    );
    assert_stale(&dir);
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn update_rewrites_stale_files_and_reinstalls() {
    let world = World::new("update-stale", Omp::Working);
    let dir = world.default_dir();
    world.write_stale(&dir);
    let output = world.run(&["update-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
    let ver = bundled_version();
    assert!(
        stdout(&output).contains(&format!("updated rmux-terminal 0.0.1 -> {ver}")),
        "{}",
        describe(&output)
    );
}

#[test]
fn update_honors_dir_flag() {
    let world = World::new("update-dir", Omp::Working);
    let dir = world.home().join("elsewhere");
    world.write_stale(&dir);
    let output = world.run(&["update-plugin", "--dir", dir.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    assert!(!world.default_dir().exists());
    assert_eq!(world.omp_log(), vec![install_line(&dir)]);
}

#[test]
fn update_without_omp_still_updates_and_warns() {
    let world = World::new("update-no-omp", Omp::Missing);
    let dir = world.default_dir();
    world.write_stale(&dir);
    let output = world.run(&["update-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_bundled(&dir);
    let ver = bundled_version();
    assert!(
        stdout(&output).contains(&format!("updated rmux-terminal 0.0.1 -> {ver}")),
        "{}",
        describe(&output)
    );
    assert!(
        stderr(&output).contains("omp plugin install"),
        "{}",
        describe(&output)
    );
}

#[test]
fn update_reports_omp_failure() {
    let world = World::new("update-omp-fails", Omp::FailingInstall);
    let dir = world.default_dir();
    world.write_stale(&dir);
    let output = world.run(&["update-plugin"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert_bundled(&dir);
    assert!(stderr(&output).contains("boom"), "{}", describe(&output));
    assert!(stdout(&output).is_empty(), "{}", describe(&output));
}

#[test]
fn uninstall_after_install_removes_everything() {
    let world = World::new("uninstall", Omp::Working);
    let dir = world.default_dir();
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(world.link().is_symlink());

    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert_eq!(
        world.omp_log(),
        vec![
            install_line(&dir),
            "plugin list --json".to_owned(),
            "plugin uninstall rmux-terminal".to_owned(),
        ]
    );
    assert!(!world.link().exists() && !world.link().is_symlink());
    assert!(!dir.exists());
    assert!(
        stdout(&output).contains(&format!("removed rmux-terminal from {}", dir.display())),
        "{}",
        describe(&output)
    );
}

#[test]
fn uninstall_without_omp_removes_files() {
    let world = World::new("uninstall-no-omp", Omp::Missing);
    let dir = world.default_dir();
    world.write_bundled(&dir);
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(!dir.exists());
    assert!(stdout(&output).contains("removed"), "{}", describe(&output));
}

#[test]
fn uninstall_registered_without_dir_unregisters() {
    let world = World::new("uninstall-no-dir", Omp::Working);
    let dir = world.default_dir();
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    std::fs::remove_dir_all(&dir).unwrap();
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        world
            .omp_log()
            .iter()
            .any(|line| line == "plugin uninstall rmux-terminal"),
        "{:?}",
        world.omp_log()
    );
    assert!(stdout(&output).contains("removed"), "{}", describe(&output));
    assert!(!world.link().is_symlink());
}

#[test]
fn uninstall_keeps_dir_with_extra_files() {
    let world = World::new("uninstall-extra", Omp::Working);
    let dir = world.default_dir();
    world.write_bundled(&dir);
    std::fs::write(dir.join("keep.txt"), "keep").unwrap();
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(!dir.join("package.json").exists());
    assert!(!dir.join("rmux.ts").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn uninstall_reports_omp_failure() {
    let world = World::new("uninstall-omp-fails", Omp::FailingUninstall);
    let output = world.run(&["install-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(stderr(&output).contains("boom"), "{}", describe(&output));
}

#[test]
fn uninstall_through_symlinked_dir_keeps_link_and_target() {
    let world = World::new("uninstall-symlink", Omp::Working);
    let target = world.home().join("real");
    world.write_bundled(&target);
    let dir = world.home().join("link");
    std::os::unix::fs::symlink(&target, &dir).unwrap();
    let output = world.run(&["uninstall-plugin", "--dir", dir.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        stdout(&output).contains(&format!("removed rmux-terminal from {}", dir.display())),
        "{}",
        describe(&output)
    );
    assert!(!target.join("package.json").exists());
    assert!(!target.join("rmux.ts").exists());
    assert!(dir.is_symlink());
    assert!(target.is_dir());
}

#[test]
fn uninstall_refuses_empty_dir() {
    let world = World::new("uninstall-empty", Omp::Working);
    let dir = world.default_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert!(dir.is_dir());
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn uninstall_refuses_foreign_dir() {
    let world = World::new("uninstall-foreign", Omp::Working);
    let dir = world.home().join("not-ours");
    std::fs::create_dir_all(&dir).unwrap();
    let package = "{\"name\": \"other\", \"version\": \"1.0.0\"}\n";
    std::fs::write(dir.join("package.json"), package).unwrap();
    std::fs::write(dir.join("rmux.ts"), OLD_RMUX_TS).unwrap();
    std::fs::write(dir.join("keep.txt"), "keep").unwrap();

    let output = world.run(&["uninstall-plugin", "--dir", dir.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1), "{}", describe(&output));
    assert_eq!(
        std::fs::read_to_string(dir.join("package.json")).unwrap(),
        package
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("rmux.ts")).unwrap(),
        OLD_RMUX_TS
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn uninstall_when_nothing_installed_succeeds() {
    let world = World::new("uninstall-nothing", Omp::Working);
    let output = world.run(&["uninstall-plugin"]);
    assert_eq!(output.status.code(), Some(0), "{}", describe(&output));
    assert!(
        stdout(&output).contains("not installed"),
        "{}",
        describe(&output)
    );
    assert!(!world.default_dir().exists());
    let log = world.omp_log();
    assert!(
        !log.iter().any(|line| line.contains("uninstall")),
        "{log:?}"
    );
}

#[test]
fn no_home_and_no_dir_fails() {
    let world = World::new("no-home", Omp::Working);
    for cmd in ["install-plugin", "update-plugin", "uninstall-plugin"] {
        let output = world.command(&[cmd]).env_remove("HOME").output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{cmd}: {}",
            describe(&output)
        );
        assert!(
            stderr(&output).contains("HOME"),
            "{cmd}: {}",
            describe(&output)
        );
    }
    assert!(!world.default_dir().exists());
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}

#[test]
fn bad_args_print_usage() {
    let world = World::new("bad-args", Omp::Working);
    for cmd in ["install-plugin", "update-plugin", "uninstall-plugin"] {
        for args in [&[cmd, "--bogus"][..], &[cmd, "--dir"][..]] {
            let output = world.run(args);
            assert_eq!(
                output.status.code(),
                Some(1),
                "{args:?}: {}",
                describe(&output)
            );
            assert!(
                stderr(&output).contains(cmd),
                "{args:?}: {}",
                describe(&output)
            );
            assert!(
                stdout(&output).is_empty(),
                "{args:?}: {}",
                describe(&output)
            );
        }

        let output = world.run(&[cmd, "--help"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{cmd}: {}",
            describe(&output)
        );
        assert!(
            stdout(&output).contains(cmd),
            "{cmd}: {}",
            describe(&output)
        );
    }
    assert!(!world.default_dir().exists());
    assert!(world.omp_log().is_empty(), "{:?}", world.omp_log());
}
