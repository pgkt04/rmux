use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=RMUX_UTF8PROC_STATIC_DIR");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    // Homebrew's utf8proc has no .a, so release builds point here at one
    // they built, keeping the shipped binary free of /opt/homebrew.
    if let Some(dir) = std::env::var_os("RMUX_UTF8PROC_STATIC_DIR").filter(|dir| !dir.is_empty()) {
        println!("cargo:rustc-link-search=native={}", dir.to_string_lossy());
        println!("cargo:rustc-link-lib=static=utf8proc");
        return;
    }
    // The oracle tmux on macOS is built with HAVE_UTF8PROC, so the width
    // and conversion helpers link the same library.
    let output = Command::new("pkg-config")
        .args(["--cflags", "--libs", "libutf8proc"])
        .output();
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => panic!(
            "pkg-config could not find libutf8proc (needed on macOS): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => panic!("pkg-config is required on macOS to locate libutf8proc: {error}"),
    };
    let flags = String::from_utf8(output.stdout).expect("pkg-config output is UTF-8");
    let mut linked = false;
    for flag in flags.split_whitespace() {
        if let Some(dir) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
            linked = true;
        }
    }
    if !linked {
        println!("cargo:rustc-link-lib=utf8proc");
    }
}
