//! C-reference and oracle comparisons for `rmux_util::utf8` (spec section 6,
//! tests 1-3 and work items 9-14). The harness links the pinned `utf8.c`,
//! `utf8-combined.c`, and `compat/utf8proc.c` with `-DHAVE_UTF8PROC` like the
//! oracle build.

mod common;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use rmux_util::utf8::combined::{
    HangulJamoState, hanguljamo_check_state, has_zwj, is_hangul_filler, is_vs, is_zwj,
    should_combine,
};
use rmux_util::utf8::{
    Utf8Data, Utf8State, cstr_width, from_cstr, is_valid, pad_left, pad_right, sanitize, strvis,
    with_width_cache,
};
use rmux_util::vis::VisFlags;

const HARNESS: &str = r#"
#include <sys/types.h>
#include <locale.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>
#include <langinfo.h>

#include "compat.h"
#include "tmux.h"

struct options *global_options = NULL;

void log_debug(const char *fmt, ...) { (void)fmt; }
__dead void fatalx(const char *fmt, ...) { (void)fmt; fprintf(stderr, "fatalx\n"); abort(); }
__dead void fatal(const char *fmt, ...) { (void)fmt; fprintf(stderr, "fatal\n"); abort(); }
struct options_entry *options_get(struct options *o, const char *n) { (void)o; (void)n; return NULL; }
struct options_array_item *options_array_first(struct options_entry *o) { (void)o; return NULL; }
struct options_array_item *options_array_next(struct options_array_item *a) { (void)a; return NULL; }
union options_value *options_array_item_value(struct options_array_item *a) { (void)a; return NULL; }

static int
hexval(int c)
{
	if (c >= '0' && c <= '9') return c - '0';
	if (c >= 'a' && c <= 'f') return c - 'a' + 10;
	if (c >= 'A' && c <= 'F') return c - 'A' + 10;
	return -1;
}

/* Decode a hex token into buf (NUL terminated); returns length. */
static size_t
unhex(const char *s, u_char *buf, size_t max)
{
	size_t n = 0;
	while (hexval(s[0]) >= 0 && hexval(s[1]) >= 0 && n < max) {
		buf[n++] = (hexval(s[0]) << 4) | hexval(s[1]);
		s += 2;
	}
	buf[n] = '\0';
	return n;
}

static void
puthex(const u_char *p, size_t n)
{
	size_t i;
	for (i = 0; i < n; i++)
		printf("%02x", p[i]);
}

static void
putud(const struct utf8_data *ud)
{
	printf("%u:%u:", ud->size, ud->width);
	puthex(ud->data, ud->size <= UTF8_SIZE ? ud->size : UTF8_SIZE);
}

static void
mkud(struct utf8_data *ud, const u_char *p, size_t n)
{
	memset(ud, 0, sizeof *ud);
	if (n > UTF8_SIZE)
		n = UTF8_SIZE;
	memcpy(ud->data, p, n);
	ud->size = ud->have = n;
	ud->width = 1;
}

/* Feed bytes through utf8_open/utf8_append; print state have size width bytes. */
static void
decode(const u_char *p, size_t n)
{
	struct utf8_data ud;
	enum utf8_state st;
	size_t i;

	st = utf8_open(&ud, p[0]);
	if (st != UTF8_MORE) {
		printf("O\n");
		return;
	}
	for (i = 1; i < n && st == UTF8_MORE; i++)
		st = utf8_append(&ud, p[i]);
	printf("%c %u ", st == UTF8_MORE ? 'M' : st == UTF8_DONE ? 'D' : 'E', ud.have);
	putud(&ud);
	printf("\n");
}

static void
fromwc(unsigned long wc)
{
	struct utf8_data ud;

	memset(&ud, 0, sizeof ud);
	if (utf8_fromwc((wchar_t)wc, &ud) != UTF8_DONE) {
		printf("%lx E\n", wc);
		return;
	}
	printf("%lx ", wc);
	putud(&ud);
	printf("\n");
}

int
main(void)
{
	char line[1024], *cmd, *a1, *a2, *a3;
	u_char b1[600], b2[600];
	size_t n1, n2;
	struct utf8_data ud, ud2, *s, *loop;
	wchar_t wc;
	char *out;
	size_t len;
	unsigned long lo, hi, i;

	if (setlocale(LC_CTYPE, "en_US.UTF-8") == NULL &&
	    setlocale(LC_CTYPE, "C.UTF-8") == NULL) {
		fprintf(stderr, "no UTF-8 locale\n");
		return 1;
	}
	utf8_update_width_cache();
#ifdef HAVE_UTF8PROC
    printf("locale %s codeset %s utf8proc %s\n", setlocale(LC_CTYPE, NULL),
        nl_langinfo(CODESET), utf8proc_version());
#else
    printf("locale %s codeset %s libc wcwidth\n", setlocale(LC_CTYPE, NULL),
        nl_langinfo(CODESET));
#endif

	while (fgets(line, sizeof line, stdin) != NULL) {
		line[strcspn(line, "\n")] = '\0';
		cmd = strtok(line, " ");
		if (cmd == NULL)
			continue;
		a1 = strtok(NULL, " ");
		a2 = strtok(NULL, " ");
		a3 = strtok(NULL, " ");
		n1 = a1 != NULL ? unhex(a1, b1, sizeof b1 - 1) : 0;
		n2 = a2 != NULL ? unhex(a2, b2, sizeof b2 - 1) : 0;
		switch (cmd[0]) {
		case 'R': /* R lo hi: utf8_fromwc for a range */
			lo = strtoul(a1, NULL, 16);
			hi = strtoul(a2, NULL, 16);
			for (i = lo; i <= hi; i++)
				fromwc(i);
			break;
		case 'd':
			decode(b1, n1);
			break;
		case 't':
			mkud(&ud, b1, n1);
			if (utf8_towc(&ud, &wc) == UTF8_DONE)
				printf("%x\n", (unsigned)wc);
			else
				printf("E\n");
			break;
		case 'W':
			mkud(&ud, b1, n1);
			printf("%d\n", utf8_has_whitespace(&ud));
			break;
		case 'v':
			printf("%d\n", utf8_isvalid((char *)b1));
			break;
		case 's':
			out = utf8_sanitize((char *)b1);
			puthex((u_char *)out, strlen(out));
			printf("\n");
			free(out);
			break;
		case 'c':
			printf("%u\n", utf8_cstrwidth((char *)b1));
			break;
		case 'p':
		case 'r':
			if (cmd[0] == 'p')
				out = utf8_padcstr((char *)b1, strtoul(a2, NULL, 10));
			else
				out = utf8_rpadcstr((char *)b1, strtoul(a2, NULL, 10));
			puthex((u_char *)out, strlen(out));
			printf("\n");
			free(out);
			break;
		case 'f':
			s = utf8_fromcstr((char *)b1);
			printf("%zu %u", utf8_strlen(s), utf8_strwidth(s, -1));
			for (loop = s; loop->size != 0; loop++) {
				printf(" ");
				putud(loop);
			}
			out = utf8_tocstr(s);
			printf(" |");
			puthex((u_char *)out, strlen(out));
			printf("\n");
			free(out);
			free(s);
			break;
		case 'h':
			mkud(&ud, b2, n2);
			printf("%d\n", utf8_cstrhas((char *)b1, &ud));
			break;
		case 'V': /* V flags hex */
			len = utf8_stravisx(&out, (char *)b2, n2, (int)strtoul(a1, NULL, 16));
			puthex((u_char *)out, len);
			printf("\n");
			free(out);
			break;
        case 'P': {
            utf8_char packed;
            enum utf8_state state;
            mkud(&ud, b1, n1);
            ud.width = strtoul(a2, NULL, 10);
            state = utf8_from_data(&ud, &packed);
            utf8_to_data(packed, &ud2);
            printf("%d %08x ", state, packed);
            putud(&ud2);
            printf("\n");
            break;
        }
		case 'C':
			mkud(&ud, b1, n1);
			mkud(&ud2, b2, n2);
			printf("%d %d %d %d %d %d %d\n",
			    utf8_should_combine(&ud, &ud2),
			    utf8_should_combine(&ud2, &ud),
			    utf8_has_zwj(&ud), utf8_is_zwj(&ud), utf8_is_vs(&ud),
			    utf8_is_hangul_filler(&ud),
			    (int)hanguljamo_check_state(&ud, &ud2));
			break;
		default:
			printf("?\n");
		}
		(void)a3;
	}
	return 0;
}
"#;

fn harness() -> Option<PathBuf> {
    let src = common::write_c("utf8_cref_harness.c", HARNESS)?;
    common::build_c(
        "utf8",
        &[
            src.as_path(),
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
        // Both targets provide these; compat.h would otherwise redeclare the
        // macOS fortified string macros.
        &[
            "-DHAVE_STRLCPY",
            "-DHAVE_STRLCAT",
            "-DHAVE_STRNLEN",
            "-DHAVE_STRNDUP",
            "-DHAVE_STRCASESTR",
            "-DHAVE_STRSEP",
            "-DHAVE_MEMMEM",
        ],
        cfg!(target_os = "macos"),
    )
}

fn setup() {
    rmux_sys::locale::setup_ctype().expect("UTF-8 locale");
    with_width_cache(|cache| cache.rebuild(std::iter::empty()));
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Input token for the harness: hex bytes, or `-` for an empty string.
fn arg(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "-".to_owned()
    } else {
        hex(bytes)
    }
}

fn ud_text(ud: &Utf8Data) -> String {
    format!("{}:{}:{}", ud.size, ud.width, hex(ud.bytes()))
}

/// A `utf8_data` holding `bytes` (at most `UTF8_SIZE`), like the harness `mkud`.
fn data(bytes: &[u8]) -> Utf8Data {
    let bytes = &bytes[..bytes.len().min(32)];
    let mut ud = Utf8Data::default();
    ud.data[..bytes.len()].copy_from_slice(bytes);
    ud.size = bytes.len() as u8;
    ud.have = ud.size;
    ud.width = 1;
    ud
}

/// Run the harness and return its output lines after the banner.
fn run_harness(bin: &Path, input: &str) -> Vec<String> {
    // Large corpora exceed the pipe buffers in both directions, so feed stdin
    // from a thread while the parent drains stdout.
    let mut child = Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn C reference");
    let mut stdin = child.stdin.take().unwrap();
    let input = input.as_bytes().to_vec();
    let writer = std::thread::spawn(move || std::io::Write::write_all(&mut stdin, &input));
    let out = child.wait_with_output().expect("C reference exit");
    writer.join().unwrap().expect("write harness input");
    assert!(
        out.status.success(),
        "C reference {bin:?} failed: {}",
        out.status
    );
    let out = out.stdout;
    let text = String::from_utf8(out).expect("harness output is ASCII");
    let mut lines = text.lines().map(str::to_owned);
    let banner = lines.next().expect("banner");
    eprintln!(
        "C reference: {banner}; platform {} {}; char is {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        if std::ffi::c_char::MIN == 0 {
            "unsigned"
        } else {
            "signed"
        }
    );
    lines.collect()
}

fn rust_decode(bytes: &[u8]) -> String {
    let Ok(mut ud) = Utf8Data::open(bytes[0]) else {
        return "O".to_owned();
    };
    let mut st = Utf8State::More;
    for &b in &bytes[1..] {
        if st != Utf8State::More {
            break;
        }
        st = ud.append(b);
    }
    let c = match st {
        Utf8State::More => 'M',
        Utf8State::Done => 'D',
        Utf8State::Error => 'E',
    };
    format!("{c} {} {}", ud.have, ud_text(&ud))
}

fn rust_fromwc(wc: u32) -> String {
    match Utf8Data::from_wc(wc) {
        Some(ud) => format!("{wc:x} {}", ud_text(&ud)),
        None => format!("{wc:x} E"),
    }
}

fn compare(label: &str, inputs: &[String], expected: &[String], actual: &[String]) {
    assert_eq!(expected.len(), actual.len(), "{label}: line count");
    let mut differences = 0;
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if e != a {
            differences += 1;
            if differences <= 20 {
                eprintln!("{label}: input {:?}: C {e:?} rmux {a:?}", inputs.get(i));
            }
        }
    }
    assert_eq!(differences, 0, "{label}: {differences} differences");
}

#[test]
fn width_corpus_all_code_points_and_malformed() {
    let Some(bin) = harness() else {
        return;
    };
    setup();

    // Every code point, in 64 K ranges to keep one input line per range.
    let mut input = String::new();
    for lo in (0u32..=0x10FFFF).step_by(0x10000) {
        let hi = (lo + 0xFFFF).min(0x10FFFF);
        let _ = writeln!(input, "R {lo:x} {hi:x}");
    }
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = (0u32..=0x10FFFF).map(rust_fromwc).collect();
    let inputs: Vec<String> = (0u32..=0x10FFFF).map(|wc| format!("U+{wc:04X}")).collect();
    compare("fromwc", &inputs, &expected, &actual);
    let valid = actual.iter().filter(|l| !l.ends_with(" E")).count();
    eprintln!("fromwc: {valid} encodable code points, defaults loaded: 162");

    // Malformed sequences through open/append.
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for a in 0u8..=0xff {
        for b in 0u8..=0xff {
            cases.push(vec![a, b]);
        }
    }
    for lead in 0xe0u8..=0xef {
        for b in [0x7f, 0x80, 0x9f, 0xa0, 0xbf, 0xc0, 0xff] {
            for c in [0x7f, 0x80, 0xbf, 0xc0] {
                cases.push(vec![lead, b, c]);
            }
        }
    }
    for lead in 0xf0u8..=0xf5 {
        for b in [0x7f, 0x80, 0x8f, 0x90, 0xbf, 0xc0] {
            for c in [0x80, 0xbf, 0x41] {
                for d in [0x80, 0xbf, 0x41] {
                    cases.push(vec![lead, b, c, d]);
                }
            }
        }
    }
    for b in 0xa0u8..=0xbf {
        for c in [0x80, 0xbf] {
            cases.push(vec![0xed, b, c]); // surrogates
        }
    }
    for b in 0x90u8..=0xbf {
        cases.push(vec![0xf4, b, 0x80, 0x80]); // above U+10FFFF
    }
    for lead in [0xc0u8, 0xc1, 0xf5, 0xf8, 0xfe, 0xff, 0x80, 0xbf] {
        cases.push(vec![lead, 0x80]);
        cases.push(vec![lead]);
    }
    let input: String = cases.iter().map(|c| format!("d {}\n", hex(c))).collect();
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = cases.iter().map(|c| rust_decode(c)).collect();
    let inputs: Vec<String> = cases.iter().map(|c| hex(c)).collect();
    compare("decode", &inputs, &expected, &actual);
}

fn random_bytes(rng: &mut common::Rng, len: usize) -> Vec<u8> {
    (0..len)
        .map(|_| match rng.below(10) {
            0..=3 => rng.below(0x80) as u8,
            4..=5 => 0xc2 + rng.below(0x33) as u8,
            6..=8 => 0x80 + rng.below(0x40) as u8,
            _ => rng.below(0x100) as u8,
        })
        .collect()
}

#[test]
fn random_byte_strings_through_decoder() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    let mut rng = common::Rng::new(0x9e37_79b9);
    let cases: Vec<Vec<u8>> = (0..10_000)
        .map(|_| {
            let len = 1 + rng.below(8) as usize;
            random_bytes(&mut rng, len)
        })
        .collect();
    let input: String = cases.iter().map(|c| format!("d {}\n", hex(c))).collect();
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = cases.iter().map(|c| rust_decode(c)).collect();
    let inputs: Vec<String> = cases.iter().map(|c| hex(c)).collect();
    compare("random decode", &inputs, &expected, &actual);
}

/// The four fixed inputs of spec section 6: ASCII, mixed, invalid byte, and
/// combined (base plus combining mark plus variation selector).
fn fixed_inputs() -> Vec<Vec<u8>> {
    vec![
        b"hello world 0x7e~".to_vec(),
        "a\u{e9}\u{4e2d}\u{1F600} b\u{200b}".as_bytes().to_vec(),
        b"ab\xff\xc3\x41\xe2\x82 \x7f\x01z".to_vec(),
        "e\u{301}\u{fe0f}\u{1F1E9}\u{1F1EA}x".as_bytes().to_vec(),
    ]
}

fn rust_fromcstr_line(s: &[u8]) -> String {
    let st = from_cstr(s);
    let mut line = format!("{} {}", st.len(), st.width(None));
    for ud in st.iter() {
        line.push(' ');
        line.push_str(&ud_text(ud));
    }
    let _ = write!(line, " |{}", hex(&st.to_bytes()));
    line
}

fn flags_of(f: VisFlags) -> String {
    format!("{:x}", f.0)
}

#[test]
fn string_functions_match_c() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    let mut rng = common::Rng::new(0x1234_5678);
    let mut cases = fixed_inputs();
    for _ in 0..10_000 {
        let len = rng.below(12) as usize;
        let mut s = random_bytes(&mut rng, len);
        // C strings stop at NUL; keep a few embedded NULs to exercise that.
        if rng.below(50) == 0 && !s.is_empty() {
            let at = rng.below(s.len() as u64) as usize;
            s[at] = 0;
        }
        cases.push(s);
    }
    // VIS_DQ: `$` before every byte value exercises the libc isalpha rule.
    for b in 1u8..=0xff {
        cases.push(vec![b'$', b]);
        cases.push(vec![b'$', b, b'x']);
    }
    let needles: Vec<Vec<u8>> = vec![
        b"a".to_vec(),
        b"\xc3\xa9".to_vec(),
        b"\xe4\xb8\xad".to_vec(),
        b"\xff".to_vec(),
        "\u{1F1E9}".as_bytes().to_vec(),
    ];
    let v1 = VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL;
    let v2 = v1 | VisFlags::DQ;

    let mut input = String::new();
    let mut actual = Vec::new();
    let mut inputs = Vec::new();
    for (i, s) in cases.iter().enumerate() {
        let h = arg(s);
        let width = (i % 7) as u32 * 3;
        let needle = &needles[i % needles.len()];
        let _ = writeln!(input, "v {h}");
        actual.push(format!("{}", u8::from(is_valid(s))));
        inputs.push(format!("v {h}"));
        // A leading zero-width sequence aborts pinned xreallocarray at size
        // zero. Test that deliberate crash-path deviation separately below.
        let crashes_c = from_cstr(s)
            .first()
            .is_some_and(|ud| ud.size > 1 && ud.width == 0);
        if !crashes_c {
            let _ = writeln!(input, "s {h}");
            actual.push(hex(&sanitize(s)));
            inputs.push(format!("s {h}"));
        }
        let _ = writeln!(input, "c {h}");
        actual.push(cstr_width(s).to_string());
        let _ = writeln!(input, "p {h} {width}");
        actual.push(hex(&pad_right(s, width)));
        let _ = writeln!(input, "r {h} {width}");
        actual.push(hex(&pad_left(s, width)));
        let _ = writeln!(input, "f {h}");
        actual.push(rust_fromcstr_line(s));
        let _ = writeln!(input, "h {h} {}", arg(needle));
        actual.push(format!(
            "{}",
            u8::from(from_cstr(s).contains(&data(needle)))
        ));
        let _ = writeln!(input, "W {h}");
        actual.push(format!("{}", u8::from(data(s).has_whitespace())));
        for flags in [v1, v2] {
            let _ = writeln!(input, "V {} {h}", flags_of(flags));
            let mut dst = Vec::new();
            strvis(&mut dst, s, flags);
            actual.push(hex(&dst));
        }
        for cmd in ["c", "p", "r", "f", "h", "W", "V1", "V2"] {
            inputs.push(format!("{cmd} {h}"));
        }
    }
    let expected = run_harness(&bin, &input);
    compare("strings", &inputs, &expected, &actual);
}

#[test]
fn sanitize_leading_zero_width_avoids_pinned_allocation_abort() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    assert_eq!(sanitize("\u{200b}suffix".as_bytes()).as_bytes(), b"suffix");
    let mut child = Command::new(bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(
        child.stdin.take().as_mut().unwrap(),
        b"s e2808b737566666978\n",
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(out.stderr.starts_with(b"fatalx"));
}

#[test]
fn whitespace_points_match_c() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    let mut cases: Vec<Vec<u8>> = Vec::new();
    for wc in (0u32..0x3100).chain([0xFEFF, 0x1F600]) {
        if let Some(ud) = Utf8Data::from_wc(wc) {
            cases.push(ud.bytes().to_vec());
            let mut combined = b"a".to_vec();
            combined.extend_from_slice(ud.bytes());
            cases.push(combined);
        }
    }
    cases.push(b"\xc3\x41".to_vec());
    cases.push(b"\xe2\x80".to_vec());
    cases.push(b"\xff\x20".to_vec());
    let input: String = cases.iter().map(|c| format!("W {}\n", hex(c))).collect();
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = cases
        .iter()
        .map(|c| format!("{}", u8::from(data(c).has_whitespace())))
        .collect();
    let inputs: Vec<String> = cases.iter().map(|c| hex(c)).collect();
    compare("has_whitespace", &inputs, &expected, &actual);
}

fn jamo_state_code(state: HangulJamoState) -> u8 {
    match state {
        HangulJamoState::NotHangulJamo => 0,
        HangulJamoState::Choseong => 1,
        HangulJamoState::Composable => 2,
        HangulJamoState::NotComposable => 3,
    }
}

#[test]
fn combined_predicates_match_c() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    let enc = |wc: u32| Utf8Data::from_wc(wc).map_or_else(Vec::new, |ud| ud.bytes().to_vec());
    let mut pool: Vec<Vec<u8>> = Vec::new();
    for wc in [
        0x61, 0x200D, 0xFE0F, 0x3164, 0x1F468, 0x1F469, 0x1F467, 0x1F466, 0x1F3FB, 0x1F3FC,
        0x1F3FF, 0x1F3FA, 0x1F400, 0x1F44B, 0x1F9DF, 0x1F600, 0x2764, 0x1F1E6, 0x1F1E9, 0x1F1EA,
        0x1F1FF, 0x1100, 0x1112, 0x1113, 0x115F, 0x1160, 0x1161, 0x1175, 0x1176, 0x11A7, 0x11A8,
        0x11C2, 0x11C3, 0x11FF, 0xA960, 0xA97C, 0xD7B0, 0xD7C6, 0xD7CB, 0xD7FB, 0xAC00, 0x301,
        0x20E3,
    ] {
        pool.push(enc(wc));
    }
    // Multi-code-point buffers: ZWJ sequences, flag pairs, tone pairs.
    let joined = |parts: &[u32]| parts.iter().flat_map(|&wc| enc(wc)).collect::<Vec<u8>>();
    pool.push(joined(&[0x1F468, 0x200D]));
    pool.push(joined(&[0x1F468, 0x200D, 0x1F469]));
    pool.push(joined(&[0x1F1E9, 0x1F1EA]));
    pool.push(joined(&[0x1F1E9, 0x1F1EA, 0x1F1E6]));
    pool.push(joined(&[0x1F44B, 0x1F3FB]));
    pool.push(joined(&[0x2764, 0xFE0F]));
    pool.push(joined(&[0x1100, 0x1161]));
    pool.push(joined(&[0x1100, 0x1161, 0x11A8]));
    pool.push(joined(&[0x61, 0x1100]));
    pool.push(b"\xc3\x41".to_vec());
    pool.push(b"\xe1\x84".to_vec());

    let mut rng = common::Rng::new(0xc0ffee);
    let mut pairs = Vec::new();
    for a in &pool[..20] {
        for b in &pool[..20] {
            pairs.push((a.clone(), b.clone()));
        }
    }
    while pairs.len() < 900 {
        let a = &pool[rng.below(pool.len() as u64) as usize];
        let b = &pool[rng.below(pool.len() as u64) as usize];
        pairs.push((a.clone(), b.clone()));
    }
    let input: String = pairs
        .iter()
        .map(|(a, b)| format!("C {} {}\n", hex(a), hex(b)))
        .collect();
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = pairs
        .iter()
        .map(|(a, b)| {
            let (a, b) = (data(a), data(b));
            format!(
                "{} {} {} {} {} {} {}",
                u8::from(should_combine(&a, &b)),
                u8::from(should_combine(&b, &a)),
                u8::from(has_zwj(&a)),
                u8::from(is_zwj(&a)),
                u8::from(is_vs(&a)),
                u8::from(is_hangul_filler(&a)),
                jamo_state_code(hanguljamo_check_state(&a, &b))
            )
        })
        .collect();
    let inputs: Vec<String> = pairs
        .iter()
        .map(|(a, b)| format!("{} {}", hex(a), hex(b)))
        .collect();
    compare("combined", &inputs, &expected, &actual);
    assert!(pairs.len() >= 500);
}

#[test]
fn towc_matches_c() {
    let Some(bin) = harness() else {
        return;
    };
    setup();
    let mut cases: Vec<Vec<u8>> = vec![
        b"a".to_vec(),
        b"\0".to_vec(),
        b"\xc3\xa9".to_vec(),
        b"\xc3\xa9\xcc\x81".to_vec(),
        b"\xed\xa0\x80".to_vec(),
        b"\xf4\x90\x80\x80".to_vec(),
        b"\xc0\x80".to_vec(),
        b"\xe0\x80\x80".to_vec(),
        b"\xff".to_vec(),
        b"\xc3".to_vec(),
    ];
    let mut rng = common::Rng::new(77);
    for _ in 0..2000 {
        let len = 1 + rng.below(5) as usize;
        cases.push(random_bytes(&mut rng, len));
    }
    let input: String = cases.iter().map(|c| format!("t {}\n", hex(c))).collect();
    let expected = run_harness(&bin, &input);
    let actual: Vec<String> = cases
        .iter()
        .map(|c| {
            data(c)
                .to_wc()
                .map_or_else(|| "E".to_owned(), |wc| format!("{wc:x}"))
        })
        .collect();
    let inputs: Vec<String> = cases.iter().map(|c| hex(c)).collect();
    compare("towc", &inputs, &expected, &actual);
}

struct OracleServer {
    tmux: PathBuf,
    socket: PathBuf,
}

impl OracleServer {
    fn start() -> Option<OracleServer> {
        let tmux = common::oracle()?;
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = PathBuf::from(format!(
            "/tmp/rmux-utf8-oracle-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).ok()?;
        let server = OracleServer {
            tmux,
            socket: dir.join("sock"),
        };
        let out = server.run(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-x",
            "80",
            "-y",
            "24",
        ]);
        if !out.status.success() {
            eprintln!(
                "oracle comparison skipped: new-session failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            return None;
        }
        Some(server)
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.tmux)
            .arg("-S")
            .arg(&self.socket)
            .args(args)
            .env_remove("TMUX")
            .env("LC_ALL", "en_US.UTF-8")
            .output()
            .expect("run oracle tmux")
    }

    fn width_of(&self, s: &str) -> u32 {
        let out = self.run(&["set", "-g", "@c", s]);
        assert!(
            out.status.success(),
            "set @c: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = self.run(&["display-message", "-p", "#{w:@c}"]);
        assert!(
            out.status.success(),
            "display-message: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .expect("width number")
    }
}

impl Drop for OracleServer {
    fn drop(&mut self) {
        let _ = self.run(&["kill-server"]);
        let _ = std::fs::remove_dir_all(self.socket.parent().unwrap());
    }
}

fn printable_corpus() -> Vec<String> {
    let atoms: &[&str] = &[
        "a",
        "Z",
        "0",
        "~",
        " ",
        "é",
        "ß",
        "ñ",
        "中",
        "文",
        "日本語",
        "한글",
        "ᄀ",
        "ᅡ",
        "ᆨ",
        "ㅤ",
        "😀",
        "👋",
        "👋🏻",
        "🏻",
        "🇩🇪",
        "🇩",
        "👨‍👩‍👧",
        "❤️",
        "❤",
        "☝",
        "⛹",
        "✊",
        "✍",
        "α",
        "β",
        "γ",
        "→",
        "─",
        "│",
        "█",
        "▶",
        "",
        "",
        "€",
        "£",
        "±",
        "×",
        "①",
        "ｆ",
        "ａ",
        "ｱ",
        "e\u{301}",
        "\u{200b}",
        "\u{feff}",
        "\u{3000}",
        "\u{a0}",
        "🧑‍🚀",
        "🦸",
        "🧟",
        "🪼",
        "🫸",
        "👍🏽",
        "🙏🏿",
    ];
    let mut rng = common::Rng::new(0xabcdef);
    let mut corpus: Vec<String> = atoms.iter().map(|a| (*a).to_owned()).collect();
    while corpus.len() < 200 {
        let n = 1 + rng.below(6) as usize;
        let s: String = (0..n)
            .map(|_| atoms[rng.below(atoms.len() as u64) as usize])
            .collect();
        corpus.push(s);
    }
    corpus
}

#[test]
fn oracle_printable_corpus_widths() {
    let Some(server) = OracleServer::start() else {
        return;
    };
    setup();
    let mut differences = Vec::new();
    for s in printable_corpus() {
        let expected = server.width_of(&s);
        let actual = from_cstr(s.as_bytes()).width(None);
        if expected != actual {
            differences.push(format!("{s:?}: oracle {expected} rmux {actual}"));
        }
    }
    for d in &differences {
        eprintln!("oracle width: {d}");
    }
    assert!(
        differences.is_empty(),
        "{} oracle width differences",
        differences.len()
    );
}

#[test]
fn compact_packing_matches_pinned_c() {
    let Some(bin) = harness() else { return };
    setup();
    let mut input = String::new();
    let mut actual = Vec::new();
    let mut inputs = Vec::new();
    for bytes in [b"a".as_slice(), b"abc", b"four", &[b'x'; 31], &[b'x'; 32]] {
        for width in 0..=2 {
            let mut ud = data(bytes);
            ud.width = width;
            let (packed, state) = rmux_util::utf8::from_data(&ud);
            let state = match state {
                Utf8State::More => 0,
                Utf8State::Done => 1,
                Utf8State::Error => 2,
            };
            actual.push(format!(
                "{state} {:08x} {}",
                packed.0,
                ud_text(&rmux_util::utf8::to_data(packed))
            ));
            let line = format!("P {} {width}", hex(bytes));
            let _ = writeln!(input, "{line}");
            inputs.push(line);
        }
    }
    compare("packing", &inputs, &run_harness(&bin, &input), &actual);
}

#[test]
fn width_override_corpus_matches_oracle() {
    let Some(server) = OracleServer::start() else {
        return;
    };
    setup();
    let mut entries: Vec<String> = [
        "U+7FFFFFFF=1",
        "U+FFFFFFFF=1",
        "U+=1",
        "U+0=1",
        "U+41-U+40=1",
        "U+41x=1",
        "α=2",
        "αβ=2",
        "U+03B1-U+03B3=2",
        "U+1F600=0",
        "U+1F600=3",
        "missing",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for i in 0..48 {
        entries.push(format!("U+{:X}={}", 0x3b1 + i % 4, i % 4));
    }
    for entry in entries {
        let out = server.run(&["set", "-gu", "codepoint-widths"]);
        assert!(out.status.success());
        let out = server.run(&["set", "-g", "codepoint-widths", &entry]);
        assert!(out.status.success(), "{entry}: {:?}", out.stderr);
        with_width_cache(|cache| cache.rebuild([entry.as_bytes()].into_iter()));
        for text in ["αβγδ", "😀", "a"] {
            assert_eq!(
                from_cstr(text.as_bytes()).width(None),
                server.width_of(text),
                "{entry}: {text}"
            );
        }
    }
}
