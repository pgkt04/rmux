//! Platform libc bridge comparisons for G01 work item 16.
mod common;
use std::path::Path;

#[test]
fn libc_bridge_fixtures_match_c() {
    let Some(main) = common::write_c(
        "bridges-reference.c",
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fnmatch.h>
#include <time.h>
int main(void) {
    const char *numbers[] = {"", "1.5x", "  -2e1", "inf", "nan!", "-Infinity"};
    for (int i=0; i<6; i++) {
        char *end; double value = strtod(numbers[i], &end);
        printf("%a %ld\n", value, (long)(end-numbers[i]));
    }
    printf("%d %d %d\n", fnmatch("A*", "abc", FNM_CASEFOLD)==0,
        fnmatch("[", "a", 0)==0, fnmatch("", "", 0)==0);
    time_t seconds = 0; struct tm tm; char out[64];
    localtime_r(&seconds, &tm);
    size_t n = strftime(out, sizeof out, "%Y-%m-%d %H:%M:%S %Z", &tm);
    printf("%s\n", out);
    printf("%zu\n", strftime(out, 3, "%Y-%m-%d", &tm));
    ctime_r(&seconds, out); printf("%s", out);
    return n == 0;
}
"#,
    ) else {
        return;
    };
    let Some(bin) = common::build_c("bridges", &[Path::new(&main)], &["-D_GNU_SOURCE"], false)
    else {
        return;
    };
    let expected = common::run(&bin, &[], &[]);
    let text = std::str::from_utf8(&expected).unwrap();
    let mut lines = text.lines();
    for input in [
        b"".as_slice(),
        b"1.5x",
        b"  -2e1",
        b"inf",
        b"nan!",
        b"-Infinity",
    ] {
        let (value, offset) = rmux_sys::number::strtod(input);
        let (cvalue, coffset) = lines.next().unwrap().split_once(' ').unwrap();
        assert_eq!(offset, coffset.parse::<usize>().unwrap());
        let (reference, _) = rmux_sys::number::strtod(cvalue.as_bytes());
        assert!(value == reference || value.is_nan() && reference.is_nan());
    }
    use rmux_sys::FnmatchFlags as F;
    let matches = [
        rmux_sys::fnmatch(b"A*", b"abc", F::CASEFOLD),
        rmux_sys::fnmatch(b"[", b"a", F::NONE),
        rmux_sys::fnmatch(b"", b"", F::NONE),
    ];
    assert_eq!(
        lines.next().unwrap(),
        format!(
            "{} {} {}",
            u8::from(matches[0]),
            u8::from(matches[1]),
            u8::from(matches[2])
        )
    );
    let tm = rmux_sys::time::localtime(0).unwrap();
    let mut buf = [0; 64];
    let n = rmux_sys::time::strftime(&mut buf, b"%Y-%m-%d %H:%M:%S %Z", &tm);
    assert_eq!(&buf[..n], lines.next().unwrap().as_bytes());
    assert_eq!(
        rmux_sys::time::strftime(&mut buf[..3], b"%Y-%m-%d", &tm).to_string(),
        lines.next().unwrap()
    );
    let n = rmux_sys::time::ctime(0, &mut buf).unwrap();
    assert_eq!(&buf[..n - 1], lines.next().unwrap().as_bytes());
    assert!(lines.next().is_none());
}
