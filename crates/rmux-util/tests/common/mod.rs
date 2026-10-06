//! Shared helpers for tests that compare rmux-util against C programs built
//! from the pinned tmux sources (`8f25579c`) or against the oracle binary.
//! Every helper returns `None` with a printed skip message when the
//! prerequisite is missing; the tests then return early.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;

pub const PIN: &str = "8f25579c";

/// Common tmux header feature macros (docs/p0-probes.md). The optional ones
/// are probed on this host: glibc has no `<bitstring.h>`, and only glibc
/// 2.38+ has strlcpy/strlcat (macOS fortify macros redeclare them, so they
/// must be declared there).
pub static HEADER_DEFINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut defines = vec![
        // AC_USE_SYSTEM_EXTENSIONS (configure.ac:53): glibc declares
        // wcwidth only with it.
        "-D_GNU_SOURCE",
        "-DHAVE_CLOCK_GETTIME",
        "-DHAVE_EVENT2_EVENT_H",
        "-DHAVE_SYS_QUEUE_H",
        "-DHAVE_SYS_TREE_H",
        "-DHAVE_U_INT",
        "-DHAVE_U_CHAR",
    ];
    if c_compiles("#include <bitstring.h>\n") {
        defines.push("-DHAVE_BITSTRING_H");
    }
    if c_compiles(
        "#include <string.h>\nint f(char *d) { return (int)strlcpy(d, \"a\", 2) + (int)strlcat(d, \"b\", 2); }\n",
    ) {
        defines.extend(["-DHAVE_STRLCPY", "-DHAVE_STRLCAT"]);
    }
    defines
});

fn c_compiles(source: &str) -> bool {
    use std::io::Write;
    let Ok(mut child) = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args([
            "-fsyntax-only",
            "-Werror=implicit-function-declaration",
            "-x",
            "c",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(source.as_bytes());
    }
    child.wait().is_ok_and(|status| status.success())
}

fn tmux_checkout() -> Option<PathBuf> {
    let path = std::env::var_os("TMUX_SRC")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("fun/tmux")))?;
    if path.join(".git").exists() || path.join("tmux.h").exists() {
        Some(path)
    } else {
        None
    }
}

fn extract_pinned() -> Option<PathBuf> {
    let checkout = tmux_checkout()?;
    let dir = std::env::temp_dir().join(format!("rmux-util-cref-{}-{}", PIN, std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let archive = Command::new("git")
        .arg("-C")
        .arg(&checkout)
        .args(["archive", PIN])
        .output()
        .ok()?;
    if !archive.status.success() {
        return None;
    }
    let mut tar = Command::new("tar")
        .arg("-x")
        .arg("-C")
        .arg(&dir)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    std::io::Write::write_all(tar.stdin.as_mut()?, &archive.stdout).ok()?;
    tar.wait().ok()?.success().then_some(dir)
}

static PINNED: LazyLock<Option<PathBuf>> = LazyLock::new(extract_pinned);

/// Directory holding the pinned tmux tree, extracted once per test process
/// with `git archive` (the checkout itself is never built in).
pub fn pinned_source() -> Option<&'static Path> {
    PINNED.as_deref().or_else(|| {
        eprintln!("C reference skipped: set TMUX_SRC to a tmux checkout containing {PIN}");
        None
    })
}

fn pkg_config(args: &[&str]) -> Vec<String> {
    Command::new("pkg-config")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Compile a C program against the pinned tree. `sources` are paths relative
/// to the pinned tree (for example `"utf8.c"`, `"compat/vis.c"`) or absolute
/// paths to test-owned files. `utf8proc` adds `-DHAVE_UTF8PROC` and links the
/// Homebrew libutf8proc the oracle uses (macOS only; required there).
pub fn build_c(name: &str, sources: &[&Path], defines: &[&str], utf8proc: bool) -> Option<PathBuf> {
    static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    let src = pinned_source()?;
    let out = src.join(format!("cref-{name}"));
    if out.exists() {
        return Some(out);
    }
    // nextest runs each test in its own process: build to a private path,
    // then rename, so a concurrent process never runs a half-written binary.
    let partial = src.join(format!("cref-{name}.{}.tmp", std::process::id()));
    let mut cc = Command::new(std::env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args(["-std=gnu99", "-w", "-O1", "-o"]).arg(&partial);
    cc.args(HEADER_DEFINES.iter()).args(defines);
    cc.arg("-I").arg(src);
    let mut libevent = pkg_config(&["--cflags", "libevent"]);
    if libevent.is_empty() && Path::new("/opt/homebrew/opt/libevent/include").exists() {
        libevent.push("-I/opt/homebrew/opt/libevent/include".into());
    }
    cc.args(&libevent);
    if utf8proc {
        let flags = pkg_config(&["--cflags", "--libs", "libutf8proc"]);
        if flags.is_empty() {
            eprintln!("C reference {name} skipped: pkg-config cannot find libutf8proc");
            return None;
        }
        cc.arg("-DHAVE_UTF8PROC").args(&flags);
    }
    for s in sources {
        // tmux compiles compat/utf8proc.c only with --enable-utf8proc.
        if !utf8proc && *s == Path::new("compat/utf8proc.c") {
            continue;
        }
        if s.is_absolute() {
            cc.arg(s);
        } else {
            cc.arg(src.join(s));
        }
    }
    // colour.c uses round(); glibc keeps libm separate.
    cc.arg("-lm");
    let output = match cc.output() {
        Ok(output) => output,
        Err(e) => {
            eprintln!("C reference {name} skipped: cannot run cc: {e}");
            return None;
        }
    };
    assert!(
        output.status.success(),
        "C reference {name}: cc failed\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::rename(&partial, &out).expect("install C reference binary");
    Some(out)
}

/// Write a test-owned C file next to the pinned tree and return its path.
pub fn write_c(name: &str, text: &str) -> Option<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = pinned_source()?.join(format!("{}-{serial}-{name}", std::process::id()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

/// Run a built reference program with `input` on stdin; returns stdout.
pub fn run(bin: &Path, args: &[&str], input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn C reference");
    let mut stdin = child.stdin.take().unwrap();
    let input = input.to_vec();
    // Large corpora fill the stdout pipe before stdin is consumed.
    let feeder = std::thread::spawn(move || std::io::Write::write_all(&mut stdin, &input));
    let out = child.wait_with_output().expect("C reference exit");
    feeder.join().unwrap().unwrap();
    assert!(
        out.status.success(),
        "C reference {bin:?} failed: {}",
        out.status
    );
    out.stdout
}

/// The pinned oracle `tmux` binary (`RMUX_ORACLE` or `oracle/bin/tmux`).
pub fn oracle() -> Option<PathBuf> {
    let path = std::env::var_os("RMUX_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../oracle/bin/tmux"));
    if path.exists() {
        Some(path)
    } else {
        eprintln!("oracle comparison skipped: build scripts/build-oracle.sh or set RMUX_ORACLE");
        None
    }
}

/// Deterministic xorshift generator for random corpora (no external crate).
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
