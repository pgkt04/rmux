// Ported from tmux image-sixel.c @ 8f25579c
use super::*;
use std::fmt::Write;
use std::process::Command;

fn nz(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|c| format!("{c:02x}")).collect()
}
fn dump(image: Option<&SixelImage>, map: Option<&SixelImage>) -> String {
    let Some(si) = image else {
        return "bad\n".into();
    };
    let (cx, cy) = si.size_in_cells();
    let mut out = format!(
        "{} {} {} {} {} {} {} {} {} {cx} {cy}\n",
        si.x,
        si.y,
        si.xpixel,
        si.ypixel,
        u8::from(si.set_ra),
        si.ra_x,
        si.ra_y,
        si.used_colours,
        si.p2
    );
    for c in &si.colours {
        write!(out, "{c},").unwrap();
    }
    out.push('\n');
    let mut hash = 14695981039346656037u64;
    for row in &si.lines {
        hash = (hash ^ row.len() as u64).wrapping_mul(1099511628211);
        for &c in row {
            hash = (hash ^ u64::from(c)).wrapping_mul(1099511628211);
        }
    }
    writeln!(out, "{hash}").unwrap();
    match si.print(map) {
        None => out.push_str("none\n"),
        Some(bytes) => {
            out.push_str(&hex(&bytes));
            out.push('\n');
        }
    }
    out
}
fn corpus() -> Vec<Vec<u8>> {
    let mut cases: Vec<Vec<u8>> = [
        "",
        "q",
        "x#0@",
        "q?",
        "q#0@",
        "q@",
        "q\"1;1;2;3",
        "q#0\"1;1;2;13",
        "q#0@-?-$@",
        "q#0@????@",
        "q#0~\"1;1;1;1@@@@",
        "q#0!10000@",
        "q#0!10000?@",
        "q#0\"1;1;10000;1",
        "q#0\"1;1;1;10000",
        "q#0\"1;1;10001;1",
        "q#0\"1;1;1;10001",
        "q#1023@",
        "q#1024@",
        "q#1025@",
        "q#3;1;360;100;100@",
        "q#3;2;100;100;100@",
        "q#1;2;0;0;0@#1;1;1;2;3@",
        "q#0;1;361;0;0@",
        "q#0;1;0;101;0@",
        "q#0;2;101;0;0@",
        "q#0;2;0;0;101@",
        "q#0;3;0;0;0@",
        "q#0;2;0;0@",
        "q#0;2;0;0;0;0@",
        "q#;2;;;@",
        "q#@",
        "q#0@\"",
        "q#0@\"1",
        "q#0@\"1;",
        "q#0@\"1;1",
        "q#0@\"1;1;",
        "q#0@\"1;1;;",
        "q#0@\"1;1;0;0",
        "q#0@\"1;1;1;1;",
        "q#4294967296@",
        "q#18446744073709551616@",
        "q\"0;0;4294967297;1#0@",
        "q# 0@",
        "q#+0@",
        "q#-0@",
        "q#0!0@",
        "q#0!10001@",
        "q#0!@",
        "q#0!1",
        "q#0!000000000000000000000000000001@",
        "q#0!0000000000000000000000000000001@",
        "q#0!1 ",
        "q#0!1!",
        "q#0!1\u{7f}",
        "q#0@ #0@",
        "q#0\u{7f}",
        "q#2@#0A$#1B-#2~",
        "q#0!4@-!5A-!3B-!2C",
        "q#0@--@",
        "q#0@\"1;1;1;20",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    for pattern in b'?'..=b'~' {
        cases.push(vec![b'q', b'#', b'0', pattern]);
    }
    for count in 1..=5 {
        cases.push(format!("q#0!{count}@").into_bytes());
    }
    for ch in 0..=255 {
        if !(32..128).contains(&ch) {
            cases.push(vec![b'q', b'#', b'0', b'@', ch]);
        }
    }
    cases.push(format!("q#0{}@", "-".repeat(1667)).into_bytes());
    cases.push(format!("q#0{}~", "-".repeat(1666)).into_bytes());
    cases
}

#[test]
fn exact_units() {
    let si = SixelImage::parse(b"q#0@", 42, nz(16), nz(32)).unwrap();
    assert_eq!(si.pixel(0, 0), 1);
    assert_eq!(si.size_in_cells(), (1, 1));
    assert_eq!(si.print(None).unwrap(), b"\x1bP9;42q#0@\x1b\\");
    let blank = SixelImage::parse(b"q\"1;1;3;4", 0, nz(2), nz(3)).unwrap();
    assert_eq!(blank.size_in_cells(), (2, 2));
    assert!(blank.print(None).is_none());
    let shrunk = SixelImage::parse(b"q#0@@@@\"1;1;1;1", 0, nz(1), nz(1)).unwrap();
    assert_eq!(shrunk.x, 1);
    assert_eq!(shrunk.pixel(3, 0), 1);
    let zero = si.scale(None, None, 0, 0, 0, 1, true).unwrap();
    assert_eq!((zero.x, zero.y), (0, 0));
    assert_eq!(zero.print(None).unwrap(), b"\x1bP9;42q\x1b\\");
}

#[test]
fn pinned_c_differential() {
    let root = std::env::var("RMUX_TMUX_SOURCE").unwrap_or_else(|_| "/Users/j/fun/tmux".into());
    let source = Command::new("git")
        .args(["-C", &root, "show", "8f25579c:image-sixel.c"])
        .output();
    let Ok(source) = source else {
        eprintln!("SKIP sixel C differential: git unavailable");
        return;
    };
    if !source.status.success() {
        eprintln!("SKIP sixel C differential: pinned tmux source missing at {root}");
        return;
    }
    let source = String::from_utf8(source.stdout).unwrap();
    let codec = source
        .split("#define SIXEL_WIDTH_LIMIT")
        .nth(1)
        .unwrap()
        .split("struct screen *\nsixel_to_screen")
        .next()
        .unwrap();
    let dir = std::env::temp_dir().join(format!("rmux-sixel-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("codec.c"),
        format!("#define SIXEL_WIDTH_LIMIT{codec}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("driver.c"),
        include_str!("../../tests/sixel_reference.c"),
    )
    .unwrap();
    let binary = dir.join("reference");
    let compile = Command::new("cc")
        .args(["-std=c99", "-fsigned-char", "-O2"])
        .arg(dir.join("driver.c"))
        .arg("-o")
        .arg(&binary)
        .output();
    let Ok(compile) = compile else {
        eprintln!("SKIP sixel C differential: C compiler unavailable");
        std::fs::remove_dir_all(dir).unwrap();
        return;
    };
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let crops = [
        [0, 0, 0, 0, 0, 0, 0, 0],
        [1, 0, 0, 0, 0, 100, 100, 1],
        [1, 3, 5, 0, 0, 100, 100, 0],
        [1, 0, 7, 1, 0, 1, 1, 1],
        [1, 5, 0, 0, 1, 2, 1, 1],
        [1, 0, 0, 0, 0, 0, 1, 1],
        [1, 0, 0, 0, 0, 1, 0, 0],
        [1, 0, 0, 100, 0, 1, 1, 1],
        [1, 10001, 1, 0, 0, 1, 1, 1],
        [1, 1, 10001, 0, 0, 1, 1, 0],
    ];
    for bytes in corpus() {
        for crop in crops {
            let parsed = SixelImage::parse(&bytes, u32::MAX, nz(2), nz(3)).ok();
            let scaled = if crop[0] != 0 {
                parsed.as_ref().and_then(|si| {
                    si.scale(
                        NonZeroU32::new(crop[1]),
                        NonZeroU32::new(crop[2]),
                        crop[3],
                        crop[4],
                        crop[5],
                        crop[6],
                        crop[7] != 0,
                    )
                })
            } else {
                None
            };
            let actual = if crop[0] != 0 {
                dump(
                    scaled.as_ref(),
                    if crop[7] == 0 { parsed.as_ref() } else { None },
                )
            } else {
                dump(parsed.as_ref(), None)
            };
            let mut args = vec![hex(&bytes), u32::MAX.to_string(), "2".into(), "3".into()];
            args.extend(crop.map(|n| n.to_string()));
            let result = Command::new(&binary).args(args).output().unwrap();
            assert!(
                result.status.success(),
                "C failure for {} {crop:?}",
                hex(&bytes)
            );
            assert_eq!(
                actual,
                String::from_utf8(result.stdout).unwrap(),
                "payload={} crop={crop:?}",
                hex(&bytes)
            );
        }
    }
    let sample = Command::new("git")
        .args(["-C", &root, "show", "8f25579c:tools/image.sixel"])
        .output()
        .unwrap();
    assert!(
        sample.status.success(),
        "pinned tools/image.sixel unavailable"
    );
    let start = sample.stdout.iter().position(|c| *c == b'q').unwrap();
    let end = sample
        .stdout
        .windows(2)
        .position(|s| s == b"\x1b\\")
        .unwrap();
    let payload = &sample.stdout[start..end];
    let sample_path = dir.join("sample.payload");
    std::fs::write(&sample_path, payload).unwrap();
    let si = SixelImage::parse(payload, 0, nz(16), nz(32)).unwrap();
    for crop in [[0, 0, 0, 0, 0, 0, 0, 0], [1, 9, 17, 1, 1, 3, 4, 0]] {
        let scaled = si.scale(
            NonZeroU32::new(crop[1]),
            NonZeroU32::new(crop[2]),
            crop[3],
            crop[4],
            crop[5],
            crop[6],
            crop[7] != 0,
        );
        let actual = if crop[0] == 0 {
            dump(Some(&si), None)
        } else {
            dump(scaled.as_ref(), Some(&si))
        };
        let mut args = vec![
            format!("@{}", sample_path.display()),
            "0".into(),
            "16".into(),
            "32".into(),
        ];
        args.extend(crop.map(|n| n.to_string()));
        let result = Command::new(&binary).args(args).output().unwrap();
        assert!(result.status.success());
        assert_eq!(
            actual,
            String::from_utf8(result.stdout).unwrap(),
            "tools/image.sixel {crop:?}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[path = "../../../rmux-util/tests/common/mod.rs"]
mod common;

// The repo's fuzz convention is reproducible bounded random tests, not libFuzzer.
#[test]
fn bounded_parse_scale_print_fuzz() {
    let dictionary: &[&[u8]] = &[
        b"#0",
        b"#3;2;1;2;3",
        b"\"1;1;20;30",
        b"!4~",
        b"$",
        b"-",
        b"@",
        b"?",
        b"~",
    ];
    for seed in 1..=2000 {
        let mut rng = common::Rng::new(seed);
        let mut bytes = b"q#0".to_vec();
        for _ in 0..64 {
            if rng.below(4) == 0 {
                bytes.push(rng.next_u64() as u8);
            } else {
                bytes.extend_from_slice(dictionary[rng.below(dictionary.len() as u64) as usize]);
            }
        }
        let metrics = (nz(1 + rng.below(16) as u32), nz(1 + rng.below(32) as u32));
        let parsed = SixelImage::parse(&bytes, seed as u32, metrics.0, metrics.1);
        assert_eq!(
            parsed,
            SixelImage::parse(&bytes, seed as u32, metrics.0, metrics.1)
        );
        if let Ok(si) = parsed {
            assert!(si.x <= LIMIT && si.y <= LIMIT);
            assert!(si.lines.iter().all(|row| row.len() <= LIMIT as usize));
            assert_eq!(si.print(None), si.print(None));
            let scaled = si.scale(
                Some(nz(1 + rng.below(16) as u32)),
                Some(nz(1 + rng.below(32) as u32)),
                rng.below(8) as u32,
                rng.below(8) as u32,
                rng.below(8) as u32,
                rng.below(8) as u32,
                false,
            );
            if let Some(new) = scaled {
                assert!(new.x <= LIMIT && new.y <= LIMIT);
                assert!(new.lines.iter().all(|row| row.len() <= LIMIT as usize));
                assert_eq!(new.print(Some(&si)), new.print(Some(&si)));
            }
        }
    }
}
