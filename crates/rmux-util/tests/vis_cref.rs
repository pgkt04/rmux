//! Raw vis/unvis corpus against the pinned `compat/vis.c` and `compat/unvis.c`
//! (spec g01 section 6 test 4, work item 6). The macOS oracle uses these
//! compatibility files (configure rejects the macOS strnvis signature).

mod common;

use std::path::Path;

use rmux_util::vis::{VisFlags, strnvis, strunvis, strvis};

const HARNESS: &str = r#"
#include <sys/types.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <locale.h>
#include "compat.h"

/* stdin lines: "<op> <flags> <hex>"; op v = strvis, n<max> = strnvis, u = strunvis */
static int hexval(int c) { return c >= 'a' ? c - 'a' + 10 : c - '0'; }

int main(void)
{
	char line[8192], in[1024], out[8192];
	int n, flags, max, r, i;
	char op[16];
	char *hex;

	if (setlocale(LC_CTYPE, "en_US.UTF-8") == NULL)
		setlocale(LC_CTYPE, "C.UTF-8");
	while (fgets(line, sizeof line, stdin) != NULL) {
		if (sscanf(line, "%15s %d", op, &flags) != 2)
			return 2;
		hex = strrchr(line, ' ') + 1;
		for (n = 0; hex[0] != '\n' && hex[0] != '\0' && hex[1] != '\n' && hex[1] != '\0'; hex += 2)
			in[n++] = (hexval(hex[0]) << 4) | hexval(hex[1]);
		in[n] = '\0';
		if (op[0] == 'v') {
			r = strvis(out, in, flags);
			for (i = 0; i < r; i++)
				printf("%02x", (unsigned char)out[i]);
		} else if (op[0] == 'n') {
			max = atoi(op + 1);
			memset(out, 'X', sizeof out);
			r = strnvis(out, in, max, flags);
			if (max > 0)
				for (i = 0; out[i] != '\0'; i++)
					printf("%02x", (unsigned char)out[i]);
		} else {
			r = strunvis(out, in);
			if (r < 0)
				printf("ERR");
			else {
				for (i = 0; i < r; i++)
					printf("%02x", (unsigned char)out[i]);
			}
		}
		putchar('\n');
	}
	return 0;
}
"#;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn build() -> Option<std::path::PathBuf> {
    let main = common::write_c("vis_harness.c", HARNESS)?;
    common::build_c(
        "vis",
        &[
            &main,
            Path::new("compat/vis.c"),
            Path::new("compat/unvis.c"),
        ],
        &[],
        false,
    )
}

const FLAG_SETS: &[VisFlags] = &[
    VisFlags::NONE,
    VisFlags(0x01 | 0x02 | 0x08 | 0x10),
    VisFlags(0x01 | 0x02 | 0x08 | 0x10 | 0x200),
    VisFlags(0x01 | 0x02),
    VisFlags(0x01 | 0x02 | 0x40),
    VisFlags(0x20 | 0x40),
    VisFlags(0x01),
    VisFlags(0x02),
    VisFlags(0x400),
    VisFlags(0x100 | 0x01),
    VisFlags(0x04 | 0x08 | 0x10 | 0x02),
    VisFlags(0x20),
];

const NEXT_BYTES: &[u8] = &[0, b'0', b'7', b'8', b'a', b' ', b'\n', 0xff];

#[test]
fn strvis_corpus_matches_compat_vis() {
    let Some(bin) = build() else {
        return;
    };
    let mut input = String::new();
    let mut expected = Vec::new();
    for &flags in FLAG_SETS {
        for c in 1u16..=255 {
            for &next in NEXT_BYTES {
                let src = if next == 0 {
                    vec![c as u8]
                } else {
                    vec![c as u8, next]
                };
                input.push_str(&format!("v {} {}\n", flags.0, hex(&src)));
                let mut out = Vec::new();
                strvis(&mut out, &src, flags);
                expected.push(hex(&out).into_bytes());
            }
        }
    }
    let got = common::run(&bin, &[], input.as_bytes());
    let lines: Vec<&[u8]> = got.split(|&b| b == b'\n').collect();
    assert_eq!(lines.len() - 1, expected.len());
    let mut diffs = 0;
    for (i, (want, exp)) in lines.iter().zip(&expected).enumerate() {
        if want != exp {
            diffs += 1;
            if diffs < 20 {
                eprintln!(
                    "case {i}: C {:?} rust {:?}",
                    String::from_utf8_lossy(want),
                    String::from_utf8_lossy(exp)
                );
            }
        }
    }
    assert_eq!(diffs, 0, "strvis differences");
}

#[test]
fn strvis_random_strings_match() {
    let Some(bin) = build() else {
        return;
    };
    let mut rng = common::Rng::new(0x5eed_0001);
    let mut input = String::new();
    let mut expected = Vec::new();
    for _ in 0..5000 {
        let len = 1 + rng.below(12) as usize;
        let src: Vec<u8> = (0..len)
            .map(|_| {
                let b = rng.next_u64() as u8;
                if b == 0 { b'a' } else { b }
            })
            .collect();
        let flags = FLAG_SETS[rng.below(FLAG_SETS.len() as u64) as usize];
        input.push_str(&format!("v {} {}\n", flags.0, hex(&src)));
        let mut out = Vec::new();
        strvis(&mut out, &src, flags);
        expected.push(hex(&out).into_bytes());
    }
    let got = common::run(&bin, &[], input.as_bytes());
    let lines: Vec<&[u8]> = got.split(|&b| b == b'\n').collect();
    assert_eq!(lines.len() - 1, expected.len());
    for (i, (want, exp)) in lines.iter().zip(&expected).enumerate() {
        assert_eq!(want, exp, "random case {i}");
    }
}

#[test]
fn strnvis_bounded_output_matches() {
    let Some(bin) = build() else {
        return;
    };
    let srcs: &[&[u8]] = &[
        b"a\x01b",
        b"\\\\",
        b"\"q\"",
        b"\xe9\xe9\xe9",
        b"abc\tdef",
        b"\x7f",
    ];
    let flags = [
        VisFlags(0x01 | 0x02 | 0x08 | 0x10),
        VisFlags(0x01 | 0x02 | 0x08 | 0x10 | 0x200),
        VisFlags::NONE,
        VisFlags(0x400),
    ];
    let mut input = String::new();
    let mut expected = Vec::new();
    for src in srcs {
        for &f in &flags {
            for max in 0..16usize {
                input.push_str(&format!("n{} {} {}\n", max, f.0, hex(src)));
                expected.push(hex(&strnvis(src, max, f)).into_bytes());
            }
        }
    }
    let got = common::run(&bin, &[], input.as_bytes());
    let lines: Vec<&[u8]> = got.split(|&b| b == b'\n').collect();
    assert_eq!(lines.len() - 1, expected.len());
    for (i, (want, exp)) in lines.iter().zip(&expected).enumerate() {
        assert_eq!(
            String::from_utf8_lossy(want),
            String::from_utf8_lossy(exp),
            "strnvis case {i}"
        );
    }
}

#[test]
fn strunvis_matches_compat_unvis() {
    let Some(bin) = build() else {
        return;
    };
    let fixed: &[&[u8]] = &[
        b"plain",
        b"a\\tb\\n\\033\\303\\251\\\\",
        b"\\1x\\12y\\123z\\1234",
        b"\\M-i\\M^A\\^?\\^A\\E",
        b"a\\\nb\\$c",
        b"\\q",
        b"\\Mx",
        b"ab\\",
        b"ab\\M",
        b"ab\\M-",
        b"ab\\^",
        b"ab\\7",
        b"ab\\77",
        b"\\777",
        b"\\400\\0",
        b"\\s\\b\\a\\v\\f\\r",
    ];
    let mut rng = common::Rng::new(0x5eed_0002);
    let alphabet = b"\\0127M^-?nrEtabvfs$\nxyz";
    let mut cases: Vec<Vec<u8>> = fixed.iter().map(|s| s.to_vec()).collect();
    for _ in 0..3000 {
        let len = 1 + rng.below(10) as usize;
        cases.push(
            (0..len)
                .map(|_| alphabet[rng.below(alphabet.len() as u64) as usize])
                .collect(),
        );
    }
    let mut input = String::new();
    for c in &cases {
        input.push_str(&format!("u 0 {}\n", hex(c)));
    }
    let got = common::run(&bin, &[], input.as_bytes());
    let lines: Vec<&[u8]> = got.split(|&b| b == b'\n').collect();
    assert_eq!(lines.len() - 1, cases.len());
    for (i, (want, src)) in lines.iter().zip(&cases).enumerate() {
        let exp = match strunvis(src) {
            Some(out) => hex(&out).into_bytes(),
            None => b"ERR".to_vec(),
        };
        assert_eq!(
            String::from_utf8_lossy(want),
            String::from_utf8_lossy(&exp),
            "strunvis case {i} {:?}",
            String::from_utf8_lossy(src)
        );
    }
}
