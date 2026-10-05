//! C reference comparison for `strtonum` and `base64` against the pinned
//! `compat/strtonum.c` and `compat/base64.c`. Skips when the pinned tree or a
//! C compiler is missing.

mod common;

use std::path::Path;

use rmux_util::base64;
use rmux_util::strtonum::{StrtonumError, strtonum};

const STRTONUM_MAIN: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "compat.h"

/* Each case: u32 len, len bytes, i64 min, i64 max. Output: "<code> <value>\n". */
int main(void)
{
	uint32_t len;
	int64_t minval, maxval;
	char *buf;
	const char *errstr;
	long long ll;
	int code;

	while (fread(&len, sizeof len, 1, stdin) == 1) {
		buf = malloc(len + 1);
		if (len != 0 && fread(buf, 1, len, stdin) != len)
			return (2);
		buf[len] = '\0';
		if (fread(&minval, sizeof minval, 1, stdin) != 1 ||
		    fread(&maxval, sizeof maxval, 1, stdin) != 1)
			return (2);
		ll = strtonum(buf, minval, maxval, &errstr);
		if (errstr == NULL)
			code = 0;
		else if (strcmp(errstr, "invalid") == 0)
			code = 1;
		else if (strcmp(errstr, "too small") == 0)
			code = 2;
		else if (strcmp(errstr, "too large") == 0)
			code = 3;
		else
			code = 9;
		printf("%d %lld\n", code, ll);
		free(buf);
	}
	return (0);
}
"#;

const BASE64_MAIN: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "compat.h"

/* Each case: u32 len, len bytes. Output: "-1\n" or "<n> <hex>\n". */
int main(void)
{
	uint32_t len;
	char *buf;
	unsigned char *out;
	size_t need;
	int n, i;

	while (fread(&len, sizeof len, 1, stdin) == 1) {
		buf = malloc(len + 1);
		if (len != 0 && fread(buf, 1, len, stdin) != len)
			return (2);
		buf[len] = '\0';
		need = ((strlen(buf) + 3) / 4) * 3;
		out = malloc(need + 1);
		n = b64_pton(buf, out, need);
		if (n == -1)
			printf("-1\n");
		else {
			printf("%d ", n);
			for (i = 0; i < n; i++)
				printf("%02x", out[i]);
			printf("\n");
		}
		free(out);
		free(buf);
	}
	return (0);
}
"#;

fn push_case(input: &mut Vec<u8>, bytes: &[u8]) {
    input.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_ne_bytes());
    input.extend_from_slice(bytes);
}
/// The target libc provides these; compat.h would otherwise redeclare the
/// fortified macOS builtins and fail to compile.
const DEFINES: &[&str] = &["-DHAVE_STRLCPY", "-DHAVE_STRLCAT"];

fn random_strtonum_input(rng: &mut common::Rng) -> Vec<u8> {
    const PIECES: &[&[u8]] = &[
        b" ",
        b"\t",
        b"\n",
        b"+",
        b"-",
        b"0",
        b"1",
        b"9",
        b"x",
        b"\0",
        b"\x0b",
        b"\x0c",
        b"\r",
        b"9223372036854775807",
        b"9223372036854775808",
        b"922337203685477580",
    ];
    let n = rng.below(6);
    let mut s = Vec::new();
    for _ in 0..n {
        let piece = PIECES[usize::try_from(rng.below(PIECES.len() as u64)).unwrap()];
        s.extend_from_slice(piece);
        if rng.below(2) == 0 {
            let digits = rng.below(21);
            for _ in 0..digits {
                s.push(b'0' + u8::try_from(rng.below(10)).unwrap());
            }
        }
    }
    s
}

fn random_bound(rng: &mut common::Rng) -> i64 {
    match rng.below(6) {
        0 => i64::MIN,
        1 => i64::MAX,
        2 => 0,
        3 => i64::try_from(rng.below(100)).unwrap() - 50,
        4 => (rng.next_u64() as i64) / 1_000_000,
        _ => rng.next_u64() as i64,
    }
}

#[test]
fn strtonum_matches_c() {
    let Some(main) = common::write_c("cref-strtonum-main.c", STRTONUM_MAIN) else {
        eprintln!("strtonum C reference skipped: pinned source unavailable");
        return;
    };
    let Some(bin) = common::build_c(
        "strtonum",
        &[Path::new("compat/strtonum.c"), &main],
        DEFINES,
        false,
    ) else {
        return;
    };

    let mut rng = common::Rng::new(0x5742_0001);
    let mut cases: Vec<(Vec<u8>, i64, i64)> = Vec::with_capacity(2000);
    let mut input = Vec::new();
    for _ in 0..2000 {
        let s = random_strtonum_input(&mut rng);
        let min = random_bound(&mut rng);
        let max = random_bound(&mut rng);
        push_case(&mut input, &s);
        input.extend_from_slice(&min.to_ne_bytes());
        input.extend_from_slice(&max.to_ne_bytes());
        cases.push((s, min, max));
    }
    let out = common::run(&bin, &[], &input);
    let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "C reference printed {} lines",
        lines.len()
    );

    for ((s, min, max), line) in cases.iter().zip(lines) {
        let (code, value) = line.split_once(' ').unwrap();
        let expected = match code {
            "0" => Ok(value.parse::<i64>().unwrap()),
            "1" => Err(StrtonumError::Invalid),
            "2" => Err(StrtonumError::TooSmall),
            "3" => Err(StrtonumError::TooLarge),
            other => panic!("unexpected C code {other}"),
        };
        let got = strtonum(s, *min, *max);
        assert_eq!(
            got,
            expected,
            "strtonum({:?}, {min}, {max})",
            String::from_utf8_lossy(s)
        );
    }
}

fn random_base64_input(rng: &mut common::Rng) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const NOISE: &[u8] = b" \t\n\x0b\x0c\r=*-_\0\x80\xff.";
    let mode = rng.below(4);
    let mut s = Vec::new();
    if mode == 0 {
        // Valid encoding of random bytes, then optional noise appended.
        let n = usize::try_from(rng.below(12)).unwrap();
        let plain: Vec<u8> = (0..n).map(|_| rng.next_u64() as u8).collect();
        s.extend_from_slice(base64::ntop(&plain).as_bytes());
        if rng.below(3) == 0 {
            s.push(NOISE[usize::try_from(rng.below(NOISE.len() as u64)).unwrap()]);
        }
        return s;
    }
    let len = rng.below(16);
    for _ in 0..len {
        let r = rng.below(10);
        let c = if r < 7 || mode == 1 {
            ALPHABET[usize::try_from(rng.below(64)).unwrap()]
        } else if r < 9 {
            // Padding and whitespace are the interesting tail cases.
            match rng.below(3) {
                0 => b'=',
                1 => b' ',
                _ => b'\n',
            }
        } else {
            NOISE[usize::try_from(rng.below(NOISE.len() as u64)).unwrap()]
        };
        s.push(c);
    }
    s
}

#[test]
fn base64_pton_matches_c() {
    let Some(main) = common::write_c("cref-base64-main.c", BASE64_MAIN) else {
        eprintln!("base64 C reference skipped: pinned source unavailable");
        return;
    };
    let Some(bin) = common::build_c(
        "base64",
        &[Path::new("compat/base64.c"), &main],
        DEFINES,
        false,
    ) else {
        return;
    };

    let mut rng = common::Rng::new(0x6234_0002);
    let mut cases: Vec<Vec<u8>> = Vec::with_capacity(2000);
    let mut input = Vec::new();
    for _ in 0..2000 {
        let s = random_base64_input(&mut rng);
        push_case(&mut input, &s);
        cases.push(s);
    }
    let out = common::run(&bin, &[], &input);
    let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "C reference printed {} lines",
        lines.len()
    );

    let mut valid = 0;
    for (s, line) in cases.iter().zip(lines) {
        let expected = if line == "-1" {
            None
        } else {
            let (n, hex) = line.split_once(' ').unwrap();
            let n: usize = n.parse().unwrap();
            let bytes: Vec<u8> = (0..n)
                .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
                .collect();
            valid += 1;
            Some(bytes)
        };
        assert_eq!(
            base64::pton(s),
            expected,
            "pton({:?})",
            String::from_utf8_lossy(s)
        );
    }
    assert!(valid > 200, "corpus has only {valid} valid decodes");
}
