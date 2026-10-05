// Ported from tmux tty-features.c @ 8f25579c
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;
use rmux_tty::features::{FEATURES, TtyFeatures, default_features, parse_features};
use std::fmt::Write;
use std::path::Path;
#[test]
fn features_and_defaults_match_pinned_c() {
    if cfg!(target_os = "macos")
        && !Path::new("/opt/homebrew/opt/ncurses/include/curses.h").exists()
    {
        eprintln!("Feature C reference skipped: Homebrew ncurses headers unavailable");
        return;
    }
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/features_reference.c");
    let section_flag = if cfg!(target_os = "macos") {
        "-Wl,-dead_strip"
    } else {
        "-Wl,--gc-sections"
    };
    let Some(binary) = common::build_c(
        "tty-features",
        &[
            fixture.as_path(),
            Path::new("xmalloc.c"),
            Path::new("compat/reallocarray.c"),
        ],
        &[
            "-ffunction-sections",
            section_flag,
            "-DHAVE_CURSES_H",
            "-I/opt/homebrew/opt/ncurses/include",
        ],
        false,
    ) else {
        return;
    };
    let output = std::process::Command::new(binary).output().unwrap();
    assert!(output.status.success());
    let mut expected = String::new();
    for name in [
        "mintty",
        "tmux",
        "rxvt-unicode",
        "iTerm2",
        "foot",
        "WezTerm",
        "ghostty",
        "Rio",
        "XTerm",
        "unknown",
    ] {
        let mut f = TtyFeatures::default();
        default_features(name, 1, &mut f);
        writeln!(expected, "default {name} {} {}", f.enabled, f.disabled).unwrap();
    }
    for feature in FEATURES {
        writeln!(
            expected,
            "feature {} {}",
            feature.name,
            feature.flags.bits()
        )
        .unwrap();
        for cap in feature.capabilities {
            writeln!(expected, "cap {cap}").unwrap();
        }
    }
    let mut f = TtyFeatures::default();
    parse_features("rGb:RGB@:RGB:utf8:unknown:mouse", ":", &mut f);
    writeln!(expected, "parse {} {}", f.enabled, f.disabled).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
}
