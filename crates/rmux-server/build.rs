// Ported from tmux configure.ac, Makefile.am @ 8f25579c
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for name in [
        "RMUX_DEFAULT_TERM",
        "RMUX_LOCK_COMMAND",
        "PATH",
        "TERMINFO",
        "TERMINFO_DIRS",
        "HOME",
        "PKG_CONFIG_PATH",
        "CC",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let native = std::env::var("HOST").unwrap() == std::env::var("TARGET").unwrap();
    let term = std::env::var("RMUX_DEFAULT_TERM").unwrap_or_else(|_| {
        if native {
            detect_term()
        } else {
            "screen".to_owned()
        }
    });
    assert!(
        term.starts_with("screen") || term.starts_with("tmux"),
        "RMUX_DEFAULT_TERM must start with screen or tmux"
    );
    let lock = std::env::var("RMUX_LOCK_COMMAND").unwrap_or_else(|_| {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") && on_path("vlock") {
            "vlock".to_owned()
        } else {
            "lock -np".to_owned()
        }
    });
    println!("cargo:rustc-env=RMUX_DEFAULT_TERM={term}");
    println!("cargo:rustc-env=RMUX_LOCK_COMMAND={lock}");
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(dir.join(program))
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
    })
}

fn detect_term() -> String {
    let out = std::env::var_os("OUT_DIR").unwrap();
    let source = Path::new(&out).join("default-term.c");
    let exe = Path::new(&out).join("default-term");
    std::fs::write(&source, "#include <curses.h>\n#include <term.h>\nint main(int argc, char **argv) { return argc != 2 || setupterm(argv[1], -1, 0) != OK; }\n").unwrap();
    let mut cc = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()));
    cc.arg(&source).arg("-o").arg(&exe);
    let flags = ["tinfow", "tinfo", "ncursesw", "ncurses", "curses"]
        .iter()
        .find_map(|library| {
            Command::new("pkg-config")
                .args(["--cflags", "--libs", library])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8(output.stdout).expect("pkg-config output"))
        });
    if let Some(flags) = flags {
        cc.args(flags.split_whitespace());
    } else {
        cc.arg("-lcurses");
    }
    assert!(
        cc.status()
            .expect("C compiler required for the default TERM probe")
            .success(),
        "default TERM probe failed to compile; set RMUX_DEFAULT_TERM for a configured build"
    );
    let mut term = "screen";
    for candidate in ["screen-256color", "tmux", "tmux-256color"] {
        if Command::new(&exe)
            .arg(candidate)
            .output()
            .expect("default TERM probe")
            .status
            .success()
        {
            term = candidate;
        }
    }
    term.to_owned()
}
