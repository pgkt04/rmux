// Ported from tmux utf8.c @ 8f25579c (utf8_width_item, utf8_find_in_width_cache, utf8_insert_width_cache, utf8_add_to_width_cache, utf8_update_width_cache, utf8_width)
//! Code point width cache: the 162 defaults plus `codepoint-widths` entries.

use std::cell::RefCell;
use std::collections::BTreeMap;

use super::tables::DEFAULT_WIDTHS;
use super::{Utf8Data, from_cstr_no_width};

/// `WCHAR_MAX` of the target libc: `wchar_t` is a signed 32-bit integer on
/// macOS and Linux (`utf8.c:346,362`).
pub const WCHAR_MAX: u64 = i32::MAX as u64;

#[derive(Default)]
pub struct WidthCache {
    map: BTreeMap<u32, u8>,
}

impl WidthCache {
    pub fn new() -> WidthCache {
        WidthCache::default()
    }

    /// `utf8_find_in_width_cache`.
    pub fn get(&self, wc: u32) -> Option<u8> {
        self.map.get(&wc).copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `utf8_insert_width_cache`: a later entry replaces an earlier one.
    fn insert(&mut self, wc: u32, width: u8) {
        crate::log_debug!("Unicode width cache: {wc:08X}={width}");
        self.map.insert(wc, width);
    }

    /// `utf8_update_width_cache` without the option lookup: clear, insert the
    /// defaults, then parse each `codepoint=width` entry in order.
    pub fn rebuild<'a>(&mut self, entries: impl Iterator<Item = &'a [u8]>) {
        self.map.clear();
        for &(wc, width) in &DEFAULT_WIDTHS {
            self.map.insert(wc, width);
        }
        for entry in entries {
            self.add(crate::bytes::cstr(entry));
        }
    }

    /// `utf8_add_to_width_cache`: parse one option value; bad values are
    /// ignored.
    fn add(&mut self, s: &[u8]) {
        let Some(eq) = s.iter().position(|&b| b == b'=') else {
            return;
        };
        let (key, value) = (&s[..eq], &s[eq + 1..]);
        let Ok(width) = crate::strtonum::strtonum(value, 0, 2) else {
            return;
        };
        let width = width as u8;

        if let Some(hex) = key.strip_prefix(b"U+") {
            let (n, rest) = strtoull16(hex);
            let Some(n) = n.filter(|&n| n != 0 && n <= WCHAR_MAX) else {
                return;
            };
            let wc_start = n as u32;
            let wc_end = match rest.split_first() {
                Some((b'-', rest)) => {
                    let Some(hex) = rest.strip_prefix(b"U+") else {
                        return;
                    };
                    let (n, rest) = strtoull16(hex);
                    let Some(n) = n.filter(|&n| n != 0 && n <= WCHAR_MAX) else {
                        return;
                    };
                    if !rest.is_empty() || (n as u32) < wc_start {
                        return;
                    }
                    n as u32
                }
                Some(_) => return,
                None => wc_start,
            };
            let mut wc = wc_start;
            loop {
                self.insert(wc, width);
                if wc == wc_end {
                    break;
                }
                wc += 1;
            }
        } else {
            let ud = from_cstr_no_width(key);
            if ud.len() != 1 {
                return;
            }
            let Some(wc) = ud[0].to_wc() else {
                return;
            };
            self.insert(wc, width);
        }
    }
}

/// `strtoull(s, &end, 16)` on a NUL-free byte string: leading C-locale
/// whitespace, optional sign, optional `0x` prefix, hex digits. Returns the
/// value (`None` on overflow, where C gives `ULLONG_MAX` with `ERANGE`) and
/// the unparsed tail. No digits give `Some(0)` with the whole input as tail.
fn strtoull16(s: &[u8]) -> (Option<u64>, &[u8]) {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
        i += 1;
    }
    let mut negative = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        negative = s[i] == b'-';
        i += 1;
    }
    if i + 2 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x' && s[i + 2].is_ascii_hexdigit()
    {
        i += 2;
    }
    let start = i;
    let mut value: u64 = 0;
    let mut overflow = false;
    while i < s.len() && s[i].is_ascii_hexdigit() {
        let digit = u64::from((s[i] as char).to_digit(16).unwrap_or(0));
        match value.checked_mul(16).and_then(|v| v.checked_add(digit)) {
            Some(v) => value = v,
            None => overflow = true,
        }
        i += 1;
    }
    if i == start {
        return (Some(0), s);
    }
    if overflow {
        return (None, &s[i..]);
    }
    if negative {
        value = value.wrapping_neg();
    }
    (Some(value), &s[i..])
}

thread_local! {
    static CACHE: RefCell<WidthCache> = RefCell::new(WidthCache::new());
}

/// Borrow the thread's width cache. The cache starts empty like tmux before
/// `server.c:212`; callers that need the defaults call `rebuild` first.
pub fn with_width_cache<R>(f: impl FnOnce(&mut WidthCache) -> R) -> R {
    CACHE.with(|cache| f(&mut cache.borrow_mut()))
}

/// `utf8_width`: cache, then `wcwidth`, then the C1 rule. `None` is
/// `UTF8_ERROR`.
pub fn width_of(ud: &Utf8Data) -> Option<u8> {
    let wc = ud.to_wc()?;
    if let Some(width) = with_width_cache(|cache| cache.get(wc)) {
        crate::log_debug!("cached width for {wc:08X} is {width}");
        return Some(width);
    }
    let mut width = rmux_sys::locale::wcwidth(wc);
    crate::log_debug!("wcwidth({wc:05X}) returned {width}");
    if width < 0 {
        // C1 control characters are nonprintable, so they are always zero width.
        width = if (0x80..=0x9f).contains(&wc) { 0 } else { 1 };
    }
    u8::try_from(width).ok()
}
