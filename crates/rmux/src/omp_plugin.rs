//! `rmux install-plugin`, `update-plugin` and `uninstall-plugin`: keep the
//! bundled omp plugin in a local directory and registered with omp.

use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const NAME: &str = "rmux-terminal";

pub const PACKAGE_JSON: &[u8] = include_bytes!("../omp/package.json");
pub const RMUX_TS: &[u8] = include_bytes!("../omp/rmux.ts");

/// The plugin files, by name inside the plugin directory.
pub const FILES: [(&str, &[u8]); 2] = [("package.json", PACKAGE_JSON), ("rmux.ts", RMUX_TS)];

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    check: bool,
    dir: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Help,
    Run(Options),
}

fn parse(args: &[Vec<u8>], allow_check: bool) -> Option<Parsed> {
    let mut opts = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_slice() {
            b"-h" | b"--help" => return Some(Parsed::Help),
            b"--check" if allow_check => opts.check = true,
            b"--dir" => {
                let path = it.next()?;
                if path.is_empty() {
                    return None;
                }
                opts.dir = Some(PathBuf::from(OsString::from_vec(path.clone())));
            }
            _ => return None,
        }
    }
    Some(Parsed::Run(opts))
}

/// Write the plugin files into `dir`, creating it.
pub fn export(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_files(dir).map_err(|e| e.to_string())
}

fn write_files(dir: &Path) -> std::io::Result<()> {
    for (name, contents) in FILES {
        std::fs::write(dir.join(name), contents)?;
    }
    Ok(())
}

pub fn install_main(args: &[Vec<u8>]) -> i32 {
    entry("install-plugin", false, args, |_, dir| install(dir))
}

pub fn update_main(args: &[Vec<u8>]) -> i32 {
    entry("update-plugin", true, args, |opts, dir| {
        update(opts.check, dir)
    })
}

pub fn uninstall_main(args: &[Vec<u8>]) -> i32 {
    entry("uninstall-plugin", false, args, |_, dir| uninstall(dir))
}

fn entry(
    cmd: &str,
    allow_check: bool,
    args: &[Vec<u8>],
    run: fn(&Options, &Path) -> Result<i32, String>,
) -> i32 {
    let check = if allow_check { " [--check]" } else { "" };
    let usage = format!("usage: rmux {cmd}{check} [--dir path]");
    let opts = match parse(args, allow_check) {
        Some(Parsed::Help) => {
            println!("{usage}");
            return 0;
        }
        Some(Parsed::Run(opts)) => opts,
        None => {
            eprintln!("{usage}");
            return 1;
        }
    };
    let result = match &opts.dir {
        Some(dir) => Ok(dir.clone()),
        None => default_dir(std::env::var_os("HOME")),
    }
    .and_then(|dir| run(&opts, &dir));
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rmux {cmd}: {e}");
            1
        }
    }
}

fn default_dir(home: Option<OsString>) -> Result<PathBuf, String> {
    match home {
        Some(home) if !home.is_empty() => Ok(PathBuf::from(home)
            .join(".omp")
            .join("local-plugins")
            .join(NAME)),
        _ => Err("HOME is not set; pass --dir path".to_owned()),
    }
}

/// A package.json field, if the bytes parse as a JSON object holding it as a string.
fn field(package_json: &[u8], key: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(package_json).ok()?;
    Some(value.get(key)?.as_str()?.to_owned())
}

fn version(package_json: &[u8]) -> String {
    field(package_json, "version").unwrap_or_else(|| "unknown".to_owned())
}

/// Run `omp` with `args`; `Ok(None)` when omp is not on PATH, `Ok(Some(stdout))` on success.
fn omp(args: &[&OsStr]) -> Result<Option<Vec<u8>>, String> {
    let command = || {
        let mut line = String::from("omp");
        for arg in args {
            line.push(' ');
            line.push_str(&arg.to_string_lossy());
        }
        line
    };
    let output = match Command::new("omp")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", command())),
    };
    if output.status.success() {
        return Ok(Some(output.stdout));
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let text = text.trim_end();
    let mut message = format!("{} failed ({})", command(), output.status);
    if !text.is_empty() {
        message.push_str(":\n");
        message.push_str(text);
    }
    Err(message)
}

fn omp_install(dir: &Path) -> Result<Option<Vec<u8>>, String> {
    omp(&[OsStr::new("plugin"), OsStr::new("install"), dir.as_os_str()])
}

fn install(dir: &Path) -> Result<i32, String> {
    export(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let shown = dir.display();
    if omp_install(dir)?.is_none() {
        return Err(format!(
            "omp not found on PATH; files are in {shown}, run: omp plugin install {shown}"
        ));
    }
    println!(
        "installed {NAME} {} to {shown}; start a fresh omp to load it",
        version(PACKAGE_JSON)
    );
    Ok(0)
}

fn update(check: bool, dir: &Path) -> Result<i32, String> {
    let shown = dir.display();
    let package_json = dir.join("package.json");
    let installed = match std::fs::read(&package_json) {
        Ok(bytes) => bytes,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            return Err(format!(
                "{NAME} is not installed in {shown}; run: rmux install-plugin"
            ));
        }
        Err(e) => return Err(format!("{}: {e}", package_json.display())),
    };
    if field(&installed, "name").as_deref() != Some(NAME) {
        return Err(format!(
            "{NAME} is not installed in {shown}; run: rmux install-plugin"
        ));
    }
    let new = version(PACKAGE_JSON);
    let same = matches(dir) == Some(true);
    if same {
        println!("{NAME} {new} is up to date");
        return Ok(0);
    }
    let old = version(&installed);
    if check {
        println!("{NAME} {new} is available (installed {old})");
        return Ok(10);
    }
    write_files(dir).map_err(|e| format!("{shown}: {e}"))?;
    let registered = omp_install(dir)?.is_some();
    println!("updated {NAME} {old} -> {new} in {shown}; start a fresh omp to load it");
    if !registered {
        eprintln!(
            "rmux update-plugin: omp not found on PATH; run: omp plugin install {shown} to refresh omp's plugin list"
        );
    }
    Ok(0)
}

/// Whether the plugin files in `dir` are byte-equal to the bundled ones;
/// `None` when a file can't be read.
pub fn matches(dir: &Path) -> Option<bool> {
    let mut same = true;
    for (name, contents) in FILES {
        same &= std::fs::read(dir.join(name)).ok()? == contents;
    }
    Some(same)
}

/// Whether `omp plugin list --json` output lists the plugin.
fn listed(list: &[u8]) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_slice(list).ok()?;
    let npm = value.get("npm")?.as_array()?;
    Some(
        npm.iter()
            .any(|plugin| plugin.get("name").and_then(serde_json::Value::as_str) == Some(NAME)),
    )
}

fn uninstall(dir: &Path) -> Result<i32, String> {
    let shown = dir.display();
    let package_json = dir.join("package.json");
    let link = match std::fs::symlink_metadata(dir) {
        Ok(meta) => Some(meta.file_type().is_symlink()),
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => None,
        Err(e) => return Err(format!("{shown}: {e}")),
    };
    if link.is_some() {
        let not_ours = || {
            format!(
                "{} is not the {NAME} package.json; not removing {shown}",
                package_json.display()
            )
        };
        match std::fs::metadata(dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(format!("{shown} is not a directory; not removing {shown}")),
            Err(e) => return Err(format!("{shown}: {e}")),
        }
        match std::fs::read(&package_json) {
            Ok(bytes) if field(&bytes, "name").as_deref() == Some(NAME) => {}
            Ok(_) => return Err(not_ours()),
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(format!(
                    "{} not found; not removing {shown}",
                    package_json.display()
                ));
            }
            Err(e) => return Err(format!("{}: {e}", package_json.display())),
        }
    }
    let ours = link.is_some();

    let mut registered = false;
    if let Some(list) = omp(&[
        OsStr::new("plugin"),
        OsStr::new("list"),
        OsStr::new("--json"),
    ])? {
        registered = listed(&list).ok_or_else(|| {
            format!(
                "cannot parse omp plugin list --json output:\n{}",
                String::from_utf8_lossy(&list).trim_end()
            )
        })?;
    }
    if registered {
        omp(&[
            OsStr::new("plugin"),
            OsStr::new("uninstall"),
            OsStr::new(NAME),
        ])?;
    }

    if ours {
        // package.json marks ownership, so it goes last.
        for (name, _) in FILES.iter().rev() {
            let path = dir.join(name);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        if link != Some(true) {
            match std::fs::remove_dir(dir) {
                Ok(()) => {}
                Err(e)
                    if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty) => {}
                Err(e) => return Err(format!("{shown}: {e}")),
            }
        }
    }

    if registered || ours {
        println!("removed {NAME} from {shown}");
    } else {
        println!("{NAME} is not installed");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|arg| arg.as_bytes().to_vec()).collect()
    }

    fn run(dir: Option<&str>, check: bool) -> Option<Parsed> {
        Some(Parsed::Run(Options {
            check,
            dir: dir.map(PathBuf::from),
        }))
    }

    #[test]
    fn parse_args() {
        assert_eq!(parse(&args(&[]), false), run(None, false));
        assert_eq!(
            parse(&args(&["--dir", "/p"]), false),
            run(Some("/p"), false)
        );
        assert_eq!(
            parse(&args(&["--dir", "/a", "--dir", "/b"]), false),
            run(Some("/b"), false)
        );
        assert_eq!(parse(&args(&["-h"]), false), Some(Parsed::Help));
        assert_eq!(
            parse(&args(&["--dir", "/p", "--help"]), false),
            Some(Parsed::Help)
        );
        assert_eq!(parse(&args(&["--dir"]), false), None);
        assert_eq!(parse(&args(&["--dir", ""]), false), None);
        assert_eq!(parse(&args(&["--force"]), false), None);
        assert_eq!(parse(&args(&["/p"]), false), None);
    }

    #[test]
    fn check_only_for_update() {
        assert_eq!(parse(&args(&["--check"]), false), None);
        assert_eq!(parse(&args(&["--check"]), true), run(None, true));
        assert_eq!(
            parse(&args(&["--dir", "/p", "--check"]), true),
            run(Some("/p"), true)
        );
    }

    #[test]
    fn default_dir_needs_home() {
        assert_eq!(
            default_dir(Some("/home/u".into())),
            Ok(PathBuf::from("/home/u/.omp/local-plugins/rmux-terminal"))
        );
        assert!(default_dir(Some(OsString::new())).is_err());
        assert!(default_dir(None).is_err());
    }

    #[test]
    fn versions() {
        assert!(field(PACKAGE_JSON, "version").is_some());
        assert_eq!(field(PACKAGE_JSON, "name").as_deref(), Some(NAME));
        assert_eq!(version(br#"{"name":"x","version":"2.0.0"}"#), "2.0.0");
        assert_eq!(version(br#"{"name":"x"}"#), "unknown");
        assert_eq!(version(br#"{"version":2}"#), "unknown");
        assert_eq!(version(b"not json"), "unknown");
        assert_eq!(version(b""), "unknown");
    }

    #[test]
    fn list_json() {
        let list = br#"{"npm":[{"name":"other"},{"name":"rmux-terminal","version":"0.1.2"}],"marketplace":[]}"#;
        assert_eq!(listed(list), Some(true));
        assert_eq!(listed(br#"{"npm":[],"marketplace":[]}"#), Some(false));
        assert_eq!(listed(b"nope"), None);
    }

    #[test]
    fn matches_bundle() {
        let dir = std::env::temp_dir().join(format!("rmux-matches-test.{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(matches(&dir), None);
        export(&dir).unwrap();
        assert_eq!(matches(&dir), Some(true));
        std::fs::write(dir.join("rmux.ts"), "changed").unwrap();
        assert_eq!(matches(&dir), Some(false));
        std::fs::remove_file(dir.join("rmux.ts")).unwrap();
        let missing = matches(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(missing, None);
    }
}
