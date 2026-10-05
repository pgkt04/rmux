// Ported from tmux tty-term.c and tty-features.c @ 8f25579c
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;
use rmux_tty::features::parse_features;
use rmux_tty::term::{CODES, CodeKind, TtyTerm, TtyTermFlags, tparm::TparmState};
use rmux_tty::tty::{TtyHostInfo, TtyOptions};
use rmux_util::bytes::ByteString;
use std::fmt::Write;
use std::path::Path;
#[test]
fn applied_capabilities_and_overrides_match_pinned_c() {
    let Some(draw) = common::write_c("features-draw.c", include_str!("draw_reference.c")) else {
        return;
    };
    let text = include_str!("features_apply_reference.c")
        .replace("\"draw_reference.c\"", &format!("\"{}\"", draw.display()));
    let Some(driver) = common::write_c("features-apply.c", &text) else {
        return;
    };
    let mut flags = vec!["-ffunction-sections", "-DHAVE_CURSES_H", "-DHAVE_TIPARM_S"];
    if cfg!(target_os = "macos") {
        if !Path::new("/opt/homebrew/opt/ncurses/include/term.h").exists() {
            eprintln!("Feature apply reference skipped: Homebrew ncurses unavailable");
            return;
        }
        flags.extend([
            "-Wl,-dead_strip",
            "-I/opt/homebrew/opt/ncurses/include",
            "-L/opt/homebrew/opt/ncurses/lib",
            "-L/opt/homebrew/opt/libevent/lib",
        ]);
    } else {
        flags.extend(["-Wl,--gc-sections", "-Wl,--no-as-needed"]);
    }
    flags.extend(["-levent", "-lncurses"]);
    let Some(binary) = common::build_c(
        "features-apply",
        &[
            &driver,
            Path::new("utf8.c"),
            Path::new("utf8-combined.c"),
            Path::new("xmalloc.c"),
            Path::new("compat/utf8proc.c"),
            Path::new("compat/vis.c"),
            Path::new("compat/strtonum.c"),
            Path::new("compat/reallocarray.c"),
            Path::new("compat/recallocarray.c"),
            Path::new("compat/explicit_bzero.c"),
        ],
        &flags,
        cfg!(target_os = "macos"),
    ) else {
        return;
    };
    let output = std::process::Command::new(binary).output().unwrap();
    assert!(output.status.success());
    let mut expected = String::new();
    let caps: Vec<_> = [
        "clear=x",
        "cup=%i%p1%d;%p2%d",
        "am=1",
        "colors=8",
        "Ss=original",
    ]
    .iter()
    .map(|s| ByteString::from(*s))
    .collect();
    for (index, (features, overrides)) in [
        ("RGB,usstyle,cstyle,utf8,hyperlinks", ""),
        ("RGB@,RGB,mouse", ""),
        ("clipboard", "Ms="),
        ("256,margins,rectfill", "setrgbf@:Clmg@:Rect@:am@"),
        ("cstyle", "Ss=new:colors=bad:bel=\\E[1::2m"),
    ]
    .iter()
    .enumerate()
    {
        let mut state = TparmState::default();
        let mut host = TtyHostInfo::default();
        let mut term = TtyTerm::create(
            &mut state,
            b"test",
            &caps,
            &mut host,
            &TtyOptions::default(),
            None,
        )
        .unwrap();
        parse_features(features, ",", &mut host.features);
        term.apply_features(&mut host);
        term.apply(overrides.as_bytes(), true, TtyTermFlags(0));
        term.apply_overrides(&mut state, &[]);
        writeln!(
            expected,
            "case {index} {} {}",
            term.flags().bits(),
            u8::from(host.utf8)
        )
        .unwrap();
        for entry in CODES {
            if !term.has(entry.code) {
                continue;
            }
            write!(expected, "{}=", entry.name).unwrap();
            match entry.kind {
                CodeKind::String => {
                    for byte in term.string(entry.code) {
                        write!(expected, "{byte:02x}").unwrap();
                    }
                }
                CodeKind::Number => write!(expected, "{}", term.number(entry.code)).unwrap(),
                CodeKind::Flag => write!(expected, "{}", u8::from(term.flag(entry.code))).unwrap(),
            }
            expected.push('\n');
        }
    }
    assert_eq!(output.stdout, expected.as_bytes());
}
