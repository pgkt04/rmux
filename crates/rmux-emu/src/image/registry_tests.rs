// Ported from tmux image.c and image-sixel.c @ 8f25579c
use super::*;
use std::fmt::Write;
use std::num::NonZeroU32;
use std::process::{Command, Stdio};

fn snapshot(registry: &ImageRegistry, owners: &[ImageOwnerId; 3], out: &mut String) {
    out.push_str("global");
    for &(owner, id) in &registry.fifo {
        write!(out, " {}", registry.get(owner, id).unwrap().data.p2).unwrap();
    }
    out.push('\n');
    for (s, &owner) in owners.iter().enumerate() {
        write!(out, "local{s}").unwrap();
        for &id in registry.ordered(owner) {
            write!(out, " {}", registry.get(owner, id).unwrap().data.p2).unwrap();
        }
        out.push('\n');
        for &id in registry.ordered(owner) {
            let im = registry.get(owner, id).unwrap();
            writeln!(
                out,
                "image {} {} {} {} {} {} {} {} {} {}",
                im.data.p2,
                im.px,
                im.py,
                im.sx,
                im.sy,
                im.data.x,
                im.data.y,
                im.data.ra_x,
                im.data.ra_y,
                im.data.used_colours
            )
            .unwrap();
            for c in &im.fallback {
                write!(out, "{c:02x}").unwrap();
            }
            out.push('\n');
        }
    }
}

#[test]
fn pinned_registry_c_differential() {
    let root = std::env::var("RMUX_TMUX_SOURCE").unwrap_or_else(|_| "/Users/j/fun/tmux".into());
    let pin = |file: &str| {
        Command::new("git")
            .args(["-C", &root, "show", &format!("8f25579c:{file}")])
            .output()
    };
    let Ok(image) = pin("image.c") else {
        eprintln!("SKIP registry C differential: git unavailable");
        return;
    };
    if !image.status.success() {
        eprintln!("SKIP registry C differential: pinned source unavailable at {root}");
        return;
    }
    let codec = pin("image-sixel.c").unwrap();
    assert!(codec.status.success());
    let image = String::from_utf8(image.stdout).unwrap();
    let codec = String::from_utf8(codec.stdout).unwrap();
    let codec = codec
        .split("#define SIXEL_WIDTH_LIMIT")
        .nth(1)
        .unwrap()
        .split("struct screen *\nsixel_to_screen")
        .next()
        .unwrap();
    let dir = std::env::temp_dir().join(format!(
        "rmux-image-registry-reference-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("codec.c"),
        format!("#define SIXEL_WIDTH_LIMIT{codec}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("registry.c"),
        image.split("#include \"tmux.h\"").nth(1).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("sixel_reference.c"),
        include_str!("../../tests/sixel_reference.c"),
    )
    .unwrap();
    std::fs::write(
        dir.join("driver.c"),
        include_str!("../../tests/image_registry_reference.c"),
    )
    .unwrap();
    let binary = dir.join("reference");
    let Ok(compile) = Command::new("cc")
        .args(["-std=c99", "-O2", "-fsigned-char"])
        .arg(dir.join("driver.c"))
        .arg("-o")
        .arg(&binary)
        .output()
    else {
        eprintln!("SKIP registry C differential: compiler unavailable");
        std::fs::remove_dir_all(dir).unwrap();
        return;
    };
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let mut script = String::new();
    for id in 1..=20 {
        writeln!(script, "store {} {id} {} {} 4 12", id % 3, id % 5, id % 7).unwrap();
    }
    script.push_str("remove 2 5\nscroll 0 0\nscroll 1 1\nscroll 2 3\nstore 0 21 3 2 50 15\narea 0 4 3 0 0\nline 1 3 0\nline 2 4294967295 2\narea 1 4294967295 1 2 5\nfree 2\nfree 2\nstore 2 22 1 0 4 15\nstore 2 23 1 10 4 6\nscroll 2 2\nscroll 2 10\n");
    for id in 24..=44 {
        writeln!(script, "store {} {id} 2 4 4 12", id % 3).unwrap();
    }
    script.push_str("free 0\nfree 1\nfree 2\nstore 0 45 4294967294 4294967294 4 12\nline 0 4294967295 0\narea 0 4294967295 4294967295 0 0\nfree 0\n");
    let mut registry = ImageRegistry::default();
    let owners = [
        registry.create_owner(),
        registry.create_owner(),
        registry.create_owner(),
    ];
    let mut actual = String::new();
    for line in script.lines() {
        let mut fields = line.split_whitespace();
        let op = fields.next().unwrap();
        let numbers: Vec<u32> = fields.map(|s| s.parse().unwrap()).collect();
        let owner = owners[numbers[0] as usize];
        let result = match op {
            "store" => {
                let bytes = format!("q#0;2;1;2;3\"1;1;{};{}@", numbers[4], numbers[5]);
                let si = SixelImage::parse(
                    bytes.as_bytes(),
                    numbers[1],
                    NonZeroU32::new(2).unwrap(),
                    NonZeroU32::new(3).unwrap(),
                )
                .unwrap();
                registry.store(owner, si, numbers[2], numbers[3]);
                false
            }
            "remove" => {
                let id = registry
                    .ordered(owner)
                    .iter()
                    .find(|&&id| registry.get(owner, id).unwrap().data.p2 == numbers[1])
                    .copied();
                id.is_some_and(|id| registry.remove(owner, id))
            }
            "free" => registry.free_all(owner),
            "line" => registry.check_line(owner, numbers[1], numbers[2]),
            "area" => registry.check_area(owner, numbers[1], numbers[2], numbers[3], numbers[4]),
            "scroll" => registry.scroll_up(owner, numbers[1]),
            _ => panic!("unknown operation"),
        };
        writeln!(actual, "result {}", u8::from(result)).unwrap();
        snapshot(&registry, &owners, &mut actual);
    }
    let script_path = dir.join("operations");
    std::fs::write(&script_path, &script).unwrap();
    let child = Command::new(&binary)
        .stdin(Stdio::from(std::fs::File::open(script_path).unwrap()))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(result.status.success());
    assert_eq!(actual, String::from_utf8(result.stdout).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}
