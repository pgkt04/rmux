// Ported from tmux fuzzy.c @ 8f25579c
/*
 * Copyright (c) 2025 Nicholas Marriott <nicholas.marriott@gmail.com>
 *
 * Permission to use, copy, modify, and distribute this software for any
 * purpose with or without fee is hereby granted, provided that the above
 * copyright notice and this permission notice appear in all copies.
 *
 * THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
 * WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
 * MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
 * ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
 * WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
 * IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
 * OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
 */

//! Fuzzy matching in the style of fzf (`fuzzy.c:27-51`). The pattern splits
//! into `|` groups and space-separated terms; styles in the text are
//! invisible but `align=` moves the surrounding text as `format_draw_none`
//! lays it out.

use rmux_emu::cell::DEFAULT_CELL;
use rmux_emu::hyperlinks::HyperlinkRegistry;
use rmux_emu::style::{Style, StyleAlign};
use rmux_util::bitset::BitSet;
use rmux_util::bytes::{ByteString, cstr};
use rmux_util::utf8::{Utf8Data, Utf8State};

use super::skip;

const BONUS_EXACT: i32 = 1000;
const BONUS_PREFIX: i32 = 200;
const BONUS_SUFFIX: i32 = 100;
const BONUS_START: i32 = 12;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CONSECUTIVE: i32 = 6;
const PENALTY_LEADING: i32 = 1;
const PENALTY_LEADING_MAX: i32 = 10;
const PENALTY_GAP: i32 = 1;

/// A successful match: the ranking score and one bit per display column
/// occupied by a matched character.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    pub score: u32,
    pub columns: BitSet,
}

impl FuzzyMatch {
    /// The `m/p` text: set columns in ascending order separated by commas
    /// (`format.c:5071-5077`).
    pub fn columns_text(&self) -> ByteString {
        let mut out = Vec::new();
        for column in self.columns.iter_set() {
            if !out.is_empty() {
                out.push(b',');
            }
            out.extend_from_slice(column.to_string().as_bytes());
        }
        out.into()
    }
}

/// `struct fuzzy_char`: one visible character of the text.
#[derive(Clone, Copy)]
struct Char {
    align: StyleAlign,
    ud: Utf8Data,
    width: u32,
    offset: u32,
}

/// `struct fuzzy_term`.
struct Term<'a> {
    inverse: bool,
    exact: bool,
    prefix: bool,
    suffix: bool,
    text: &'a [u8],
}

const ALIGNS: usize = StyleAlign::AbsoluteCentre as usize + 1;

fn align_index(align: StyleAlign) -> usize {
    align as usize
}

/// `fuzzy_is_boundary`.
fn is_boundary(ud: &Utf8Data) -> bool {
    ud.size == 1 && b" -_/.:".contains(&ud.data[0])
}

/// `fuzzy_char_equal`: ASCII case folding only when `fold`.
fn char_equal(a: &Utf8Data, b: &Utf8Data, fold: bool) -> bool {
    if fold && a.size == 1 && b.size == 1 && a.data[0] < 0x80 && b.data[0] < 0x80 {
        return a.data[0].eq_ignore_ascii_case(&b.data[0]);
    }
    a.size == b.size && a.bytes() == b.bytes()
}

/// `fuzzy_align`: the default alignment is the left column.
fn map_align(align: StyleAlign) -> StyleAlign {
    if align == StyleAlign::Default {
        StyleAlign::Left
    } else {
        align
    }
}

/// `fuzzy_add`.
fn add(cs: &mut Vec<Char>, align: StyleAlign, ud: &Utf8Data, widths: &mut [u32; ALIGNS]) {
    let a = align_index(align);
    cs.push(Char {
        align,
        ud: *ud,
        width: u32::from(ud.width),
        offset: widths[a],
    });
    widths[a] += u32::from(ud.width);
}

/// `fuzzy_decode_one`: one UTF-8 character, or the single byte at `cp` when
/// the sequence does not complete (`fuzzy.c:137-152`).
fn decode_one(text: &[u8], mut cp: usize, ud: &mut Utf8Data) -> usize {
    let start = cp;
    if let Ok(mut open) = Utf8Data::open(text[cp]) {
        let mut more = Utf8State::More;
        loop {
            cp += 1;
            if cp == text.len() || more != Utf8State::More {
                break;
            }
            more = open.append(text[cp]);
        }
        if more == Utf8State::Done {
            *ud = open;
            return cp;
        }
        cp = start;
    }
    *ud = Utf8Data::set(text[cp]);
    cp + 1
}

/// `fuzzy_scan`: the visible characters with their alignment and offset
/// (`fuzzy.c:159-229`).
fn scan(text: &[u8], widths: &mut [u32; ALIGNS]) -> Vec<Char> {
    let mut cs = Vec::new();
    let mut current = StyleAlign::Left;
    let mut sy = Style::from_cell(DEFAULT_CELL);
    let mut links: Option<HyperlinkRegistry> = None;
    let hash = Utf8Data::set(b'#');
    let bracket = Utf8Data::set(b'[');
    let mut ud = Utf8Data::default();
    let mut cp = 0usize;

    while cp < text.len() {
        if text[cp] == b'#' {
            let n = text[cp..].iter().take_while(|&&b| b == b'#').count();
            if text.get(cp + n) != Some(&b'[') {
                let leading = if n % 2 == 0 { n / 2 } else { n / 2 + 1 };
                for _ in 0..leading {
                    add(&mut cs, current, &hash, widths);
                }
                cp += n;
                continue;
            }
            for _ in 0..n / 2 {
                add(&mut cs, current, &hash, widths);
            }
            if n % 2 == 0 {
                add(&mut cs, current, &bracket, widths);
                cp += n + 1;
                continue;
            }
            let body = cp + n + 1;
            let Some(end) = skip(&text[body..], b"]").map(|e| body + e) else {
                break;
            };
            let registry = links.get_or_insert_with(HyperlinkRegistry::new);
            if sy.parse(&DEFAULT_CELL, &text[body..end], registry).is_ok() {
                current = map_align(sy.align);
            }
            cp = end + 1;
            continue;
        }

        cp = decode_one(text, cp, &mut ud);
        if ud.size == 1 && (ud.data[0] <= 0x1f || ud.data[0] >= 0x7f) {
            continue;
        }
        add(&mut cs, current, &ud, widths);
    }
    cs
}

/// `fuzzy_column`: the display column of a visible character after the
/// no-list layout, if it is still visible.
fn column(
    fc: &Char,
    start: &[u32; ALIGNS],
    src: &[u32; ALIGNS],
    vis: &[u32; ALIGNS],
) -> Option<u32> {
    let a = align_index(fc.align);
    if fc.offset < src[a] || fc.offset >= src[a] + vis[a] {
        return None;
    }
    Some(start[a] + (fc.offset - src[a]))
}

/// `fuzzy_decode`: decode a term into `out`, failing once `limit`
/// characters would be exceeded.
fn decode(term: &[u8], out: &mut Vec<Utf8Data>, limit: usize) -> bool {
    out.clear();
    let mut cp = 0usize;
    let mut ud = Utf8Data::default();
    while cp != term.len() {
        if out.len() == limit {
            return false;
        }
        cp = decode_one(term, cp, &mut ud);
        out.push(ud);
    }
    true
}

/// `fuzzy_score_positions` (`fuzzy.c:265-295`).
fn score_positions(pos: &[usize], cs: &[Char]) -> i32 {
    let Some(&first) = pos.first() else {
        return 0;
    };
    let mut score = 0i32;
    if first == 0 {
        score += BONUS_START;
    } else {
        if is_boundary(&cs[first - 1].ud) {
            score += BONUS_BOUNDARY;
        }
        if (first as i32) < PENALTY_LEADING_MAX {
            score -= first as i32 * PENALTY_LEADING;
        } else {
            score -= PENALTY_LEADING_MAX * PENALTY_LEADING;
        }
    }
    for i in 1..pos.len() {
        if pos[i] == pos[i - 1] + 1 {
            score += BONUS_CONSECUTIVE;
        } else if is_boundary(&cs[pos[i] - 1].ud) {
            score += BONUS_BOUNDARY;
        }
    }
    let span = pos[pos.len() - 1] - first + 1;
    let gap = span - pos.len();
    score -= gap as i32 * PENALTY_GAP;
    score
}

/// `fuzzy_match_fuzzy`: a forward subsequence compacted backwards from its
/// final position (`fuzzy.c:301-353`).
fn match_fuzzy(
    tok: &[Utf8Data],
    cs: &[Char],
    fold: bool,
    score: &mut i32,
    matched: &mut [bool],
    pos: &mut Vec<usize>,
) -> bool {
    if tok.is_empty() || cs.is_empty() {
        return false;
    }
    pos.clear();
    pos.resize(tok.len(), 0);

    let mut ci = 0usize;
    for (pi, t) in tok.iter().enumerate() {
        while ci != cs.len() && !char_equal(t, &cs[ci].ud, fold) {
            ci += 1;
        }
        if ci == cs.len() {
            return false;
        }
        pos[pi] = ci;
        ci += 1;
    }

    ci = pos[tok.len() - 1];
    for pi in (1..=tok.len()).rev() {
        let mut found = false;
        loop {
            if char_equal(&tok[pi - 1], &cs[ci].ud, fold) {
                pos[pi - 1] = ci;
                found = true;
                break;
            }
            if ci == 0 {
                break;
            }
            ci -= 1;
        }
        if !found {
            return false;
        }
        if pi != 1 {
            ci -= 1;
        }
    }

    *score += score_positions(pos, cs);
    for &p in pos.iter() {
        matched[p] = true;
    }
    true
}

/// `fuzzy_score_exact` (`fuzzy.c:357-378`).
fn score_exact(
    start: usize,
    toklen: usize,
    ncs: usize,
    cs: &[Char],
    prefix: bool,
    suffix: bool,
) -> i32 {
    let mut score = BONUS_EXACT + toklen as i32 * BONUS_CONSECUTIVE;
    if prefix {
        score += BONUS_PREFIX;
    }
    if suffix {
        score += BONUS_SUFFIX;
    }
    if start == 0 {
        score += BONUS_START;
    } else if is_boundary(&cs[start - 1].ud) {
        score += BONUS_BOUNDARY;
    }
    if (start as i32) < PENALTY_LEADING_MAX {
        score -= start as i32 * PENALTY_LEADING;
    } else {
        score -= PENALTY_LEADING_MAX * PENALTY_LEADING;
    }
    if !prefix && !suffix {
        score -= (ncs - (start + toklen)) as i32;
    }
    score
}

/// `fuzzy_match_exact`: eligible starts, first highest score kept
/// (`fuzzy.c:382-433`).
fn match_exact(
    tok: &[Utf8Data],
    cs: &[Char],
    fold: bool,
    prefix: bool,
    suffix: bool,
    score: &mut i32,
    matched: Option<&mut [bool]>,
) -> bool {
    let toklen = tok.len();
    let ncs = cs.len();
    if toklen == 0 || toklen > ncs {
        return false;
    }
    let (start, end) = if prefix && suffix {
        if toklen != ncs {
            return false;
        }
        (0, 1)
    } else if prefix {
        (0, 1)
    } else if suffix {
        (ncs - toklen, ncs - toklen + 1)
    } else {
        (0, ncs - toklen + 1)
    };

    let mut best = None;
    for i in start..end {
        let ok = tok
            .iter()
            .zip(&cs[i..i + toklen])
            .all(|(t, c)| char_equal(t, &c.ud, fold));
        if !ok {
            continue;
        }
        let value = score_exact(i, toklen, ncs, cs, prefix, suffix);
        match best {
            Some((_, bestscore)) if value <= bestscore => {}
            _ => best = Some((i, value)),
        }
    }
    let Some((best, bestscore)) = best else {
        return false;
    };
    *score += bestscore;
    if let Some(matched) = matched {
        for m in &mut matched[best..best + toklen] {
            *m = true;
        }
    }
    true
}

/// `fuzzy_parse_term` (`fuzzy.c:437-471`).
fn parse_term(mut text: &[u8]) -> Option<Term<'_>> {
    let mut term = Term {
        inverse: false,
        exact: false,
        prefix: false,
        suffix: false,
        text: &[],
    };
    if text.is_empty() {
        return None;
    }
    if text[0] == b'!' {
        term.inverse = true;
        text = &text[1..];
    }
    if text.is_empty() {
        return None;
    }
    if text[0] == b'\'' {
        term.exact = true;
        text = &text[1..];
    } else if text[0] == b'^' {
        term.exact = true;
        term.prefix = true;
        text = &text[1..];
    }
    if text.is_empty() {
        return None;
    }
    if text[text.len() - 1] == b'$' {
        term.exact = true;
        term.suffix = true;
        text = &text[..text.len() - 1];
    }
    if text.is_empty() {
        return None;
    }
    if term.inverse {
        term.exact = true;
    }
    term.text = text;
    Some(term)
}

struct Scratch {
    tok: Vec<Utf8Data>,
    pos: Vec<usize>,
}

/// `fuzzy_match_term` (`fuzzy.c:475-498`).
fn match_term(
    term: &Term<'_>,
    scratch: &mut Scratch,
    cs: &[Char],
    fold: bool,
    score: &mut i32,
    matched: &mut [bool],
) -> bool {
    if !decode(term.text, &mut scratch.tok, cs.len()) {
        return term.inverse;
    }
    let mut value = 0i32;
    let matched_term = if term.exact {
        match_exact(
            &scratch.tok,
            cs,
            fold,
            term.prefix,
            term.suffix,
            &mut value,
            if term.inverse { None } else { Some(matched) },
        )
    } else {
        match_fuzzy(
            &scratch.tok,
            cs,
            fold,
            &mut value,
            matched,
            &mut scratch.pos,
        )
    };
    if term.inverse {
        return !matched_term;
    }
    if !matched_term {
        return false;
    }
    *score += value;
    true
}

/// `fuzzy_match_group`: all terms of one `|` alternative (`fuzzy.c:502-526`).
fn match_group(
    group: &[u8],
    scratch: &mut Scratch,
    cs: &[Char],
    fold: bool,
    score: &mut i32,
    matched: &mut [bool],
) -> bool {
    *score = 0;
    let mut any = false;
    let mut cp = 0usize;
    while cp != group.len() {
        while cp != group.len() && group[cp] == b' ' {
            cp += 1;
        }
        if cp == group.len() {
            break;
        }
        let sp = cp;
        while cp != group.len() && group[cp] != b' ' {
            cp += 1;
        }
        let Some(term) = parse_term(&group[sp..cp]) else {
            return false;
        };
        any = true;
        if !match_term(&term, scratch, cs, fold, score, matched) {
            return false;
        }
    }
    any
}

/// `fuzzy_match`: match `pattern` against `text` drawn into `width` columns
/// (`fuzzy.c:534-666`). Inputs end at their first NUL.
pub fn fuzzy_match(pattern: &[u8], text: &[u8], width: u32) -> Option<FuzzyMatch> {
    let pattern = cstr(pattern);
    let text = cstr(text);
    if width == 0 {
        return None;
    }
    let bits = width as usize;

    // An empty query matches everything, with nothing highlighted.
    if pattern.iter().all(|&b| b == b' ' || b == b'|') {
        return Some(FuzzyMatch {
            score: 0,
            columns: BitSet::new(bits),
        });
    }

    // Smart-case: fold unless the pattern has an uppercase character.
    let fold = !pattern.iter().any(u8::is_ascii_uppercase);

    let mut widths = [0u32; ALIGNS];
    let cs = scan(text, &mut widths);
    let ncs = cs.len();
    let mut matched = vec![false; ncs];
    let mut best = vec![false; ncs];
    let mut scratch = Scratch {
        tok: Vec::with_capacity(pattern.len().min(ncs)),
        pos: Vec::new(),
    };

    // Match each |-separated group and keep the best-scoring one.
    let mut found: Option<i32> = None;
    let mut cp = 0usize;
    while cp != pattern.len() {
        while cp != pattern.len() && (pattern[cp] == b' ' || pattern[cp] == b'|') {
            cp += 1;
        }
        if cp == pattern.len() {
            break;
        }
        let sp = cp;
        while cp != pattern.len() && pattern[cp] != b'|' {
            cp += 1;
        }
        matched.fill(false);
        let mut groupscore = 0i32;
        if match_group(
            &pattern[sp..cp],
            &mut scratch,
            &cs,
            fold,
            &mut groupscore,
            &mut matched,
        ) && found.is_none_or(|bestscore| groupscore > bestscore)
        {
            found = Some(groupscore);
            best.copy_from_slice(&matched);
        }
    }
    let bestscore = found?;

    // Work out the trimmed widths and start columns of each alignment,
    // mirroring format_draw_none.
    let (mut wl, mut wc, mut wr) = (
        widths[align_index(StyleAlign::Left)],
        widths[align_index(StyleAlign::Centre)],
        widths[align_index(StyleAlign::Right)],
    );
    let mut wa = widths[align_index(StyleAlign::AbsoluteCentre)];
    while wl + wc + wr > width {
        if wc > 0 {
            wc -= 1;
        } else if wr > 0 {
            wr -= 1;
        } else {
            wl -= 1;
        }
    }
    if wa > width {
        wa = width;
    }

    let mut start = [0u32; ALIGNS];
    let mut src = [0u32; ALIGNS];
    let mut vis = [0u32; ALIGNS];
    let (l, c, r, a) = (
        align_index(StyleAlign::Left),
        align_index(StyleAlign::Centre),
        align_index(StyleAlign::Right),
        align_index(StyleAlign::AbsoluteCentre),
    );
    start[l] = 0;
    src[l] = 0;
    vis[l] = wl;
    start[r] = width - wr;
    src[r] = widths[r] - wr;
    vis[r] = wr;
    start[c] = wl + ((width - wr) - wl) / 2 - wc / 2;
    src[c] = widths[c] / 2 - wc / 2;
    vis[c] = wc;
    start[a] = (width - wa) / 2;
    src[a] = 0;
    vis[a] = wa;

    // Set a bit for each column of each matched character.
    let mut mask = BitSet::new(bits);
    for (fc, &hit) in cs.iter().zip(&best) {
        if !hit {
            continue;
        }
        let Some(col) = column(fc, &start, &src, &vis) else {
            continue;
        };
        for j in 0..fc.width {
            if col + j >= width {
                break;
            }
            mask.set((col + j) as usize);
        }
    }

    Some(FuzzyMatch {
        score: u32::try_from(bestscore).unwrap_or(0),
        columns: mask,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, text: &str, width: u32) -> Option<(u32, Vec<usize>)> {
        fuzzy_match(pattern.as_bytes(), text.as_bytes(), width)
            .map(|r| (r.score, r.columns.iter_set().collect()))
    }

    #[test]
    fn empty_and_zero_width() {
        assert_eq!(m("", "abc", 3), Some((0, vec![])));
        assert_eq!(m(" | ", "abc", 3), Some((0, vec![])));
        assert_eq!(m("a", "abc", 0), None);
        assert_eq!(m("", "abc", 0), None);
    }

    #[test]
    fn subsequence_scores_and_columns() {
        assert_eq!(
            m("abc", "abc", 3),
            Some(((12 + 6 + 6) as u32, vec![0, 1, 2]))
        );
        assert_eq!(m("ac", "abc", 3), Some(((12 - 1) as u32, vec![0, 2])));
        assert_eq!(m("c", "abc", 3), Some((0, vec![2])));
        assert_eq!(m("b", "a-b", 3), Some(((8 - 2) as u32, vec![2])));
        assert_eq!(m("xyz", "abc", 3), None);
        assert_eq!(m("b", "aaaaaaaaaaab", 12), Some((0, vec![11])));
    }

    #[test]
    fn backward_compaction_prefers_shorter_span() {
        assert_eq!(
            m("ab", "a--a-b", 6),
            Some(((8 - 3 + 8 - 1) as u32, vec![3, 5]))
        );
    }

    #[test]
    fn exact_prefix_suffix_and_inverse() {
        assert_eq!(
            m("'bc", "abcbc", 5),
            Some(((1000 + 12 - 1 - 2) as u32, vec![1, 2]))
        );
        assert_eq!(
            m("^ab", "abab", 4),
            Some(((1000 + 12 + 200 + 12) as u32, vec![0, 1]))
        );
        assert_eq!(
            m("ab$", "abab", 4),
            Some(((1000 + 12 + 100 - 2) as u32, vec![2, 3]))
        );
        assert_eq!(
            m("^abab$", "abab", 4),
            Some(((1000 + 24 + 300 + 12) as u32, vec![0, 1, 2, 3]))
        );
        assert_eq!(m("^ab$", "abab", 4), None);
        assert_eq!(m("!b", "abc", 3), None);
        assert_eq!(m("!x", "abc", 3), Some((0, vec![])));
        assert_eq!(m("a !c", "abc", 3), None);
        assert_eq!(m("a !x", "abc", 3), Some((12, vec![0])));
        assert_eq!(m("!", "abc", 3), None);
        assert_eq!(m("'", "abc", 3), None);
        assert_eq!(m("^$", "abc", 3), None);
        assert_eq!(
            m("!abcd", "abc", 3),
            Some((0, vec![])),
            "too long inverse term passes"
        );
        assert_eq!(m("abcd", "abc", 3), None);
    }

    #[test]
    fn groups_keep_first_highest_score() {
        assert_eq!(m("x|c", "abc", 3), Some((0, vec![2])));
        assert_eq!(m("c|'c", "abc", 3), Some(((1000 + 6 - 2) as u32, vec![2])));
        assert_eq!(m("a|c", "abc", 3), Some((12, vec![0])));
        assert_eq!(m("c|a", "abc", 3), Some((12, vec![0])));
        assert_eq!(m("c|x", "abc", 3), Some((0, vec![2])));
        assert_eq!(m("c|'c|c", "abc", 3), Some((1004, vec![2])));
    }

    #[test]
    fn smart_case_and_utf8() {
        assert_eq!(m("a", "A", 1), Some((12, vec![0])));
        assert_eq!(m("A", "a", 1), None);
        assert_eq!(m("é", "É", 1), None);
        assert_eq!(m("é", "xé", 2), Some((0, vec![1])));
        assert_eq!(m("漢", "a漢", 3), Some((0, vec![1, 2])));
        assert_eq!(m("b", "a\u{0301}b", 2), Some((0, vec![1])));
    }

    #[test]
    fn styles_hashes_and_alignment() {
        assert_eq!(m("b", "#[fg=red]ab", 2), Some((0, vec![1])));
        assert_eq!(m("#", "a##b", 3), Some((0, vec![1])));
        assert_eq!(m("[", "##[x", 3), Some((0, vec![1])));
        assert_eq!(m("#[", "###x", 3), None);
        assert_eq!(m("x", "#[fg=red", 3), None, "missing ] stops the scan");
        assert_eq!(m("x", "#[bogus]x", 3), Some((12, vec![0])));
        assert_eq!(m("r", "l#[align=right]r", 5), Some((0, vec![4])));
        assert_eq!(m("c", "l#[align=centre]c", 5), Some((0, vec![3])));
        assert_eq!(m("c", "#[align=absolute-centre]c", 5), Some((12, vec![2])));
        assert_eq!(
            m("'lll", "lll#[align=right]r", 2),
            Some((1029, vec![0, 1])),
            "clipped char has no column"
        );
        assert_eq!(
            m("r", "lll#[align=right]r", 3),
            Some((0, vec![])),
            "trimmed right has no columns"
        );
    }

    #[test]
    fn columns_text_format() {
        let r = fuzzy_match(b"ac", b"abc", 3).unwrap();
        assert_eq!(r.columns_text(), b"0,2");
        assert_eq!(fuzzy_match(b"", b"abc", 3).unwrap().columns_text(), b"");
    }

    #[test]
    fn pinned_score_ties_and_byte_boundaries() {
        assert_eq!(m("'bc", "abcbc", 5), Some((1009, vec![1, 2])));
        assert_eq!(m("c|b", "xxxxxxxxxxxbc", 13), Some((0, vec![12])));
        assert_eq!(m("b|c", "xxxxxxxxxxxbc", 13), Some((0, vec![11])));
        assert_eq!(m("c|b", "abcd", 4), Some((0, vec![1])));
        assert_eq!(m("b|c", "abcd", 4), Some((0, vec![1])));
        assert_eq!(m("!ssh", "s_s_h", 5), Some((0, vec![])));
        assert_eq!(m("!x", "", 1), Some((0, vec![])));
        assert_eq!(m("\u{0301}", "a\u{0301}", 1), Some((0, vec![])));
        assert_eq!(m("漢", "漢", 1), Some((12, vec![0])));
        assert_eq!(m("b", "#[align=right]abc", 2), Some((0, vec![0])));
        assert_eq!(m("a", "#[align=right]abc", 2), Some((12, vec![])));
        assert_eq!(m("b", "#[align=centre]abc", 2), Some((0, vec![1])));
        assert_eq!(m("c", "#[align=centre]abc", 2), Some((0, vec![])));
        assert_eq!(
            m("a", "#[align=right]a#[align=bogus]b", 4),
            Some((12, vec![2]))
        );
        assert_eq!(
            fuzzy_match(b"a\0B", b"A\0b", 2).map(|r| (r.score, r.columns_text())),
            Some((12, ByteString::from(&b"0"[..])))
        );
        assert_eq!(
            fuzzy_match(b"ab", b"\xffa\xe2\x82b\n", 2).map(|r| (r.score, r.columns_text())),
            Some((18, ByteString::from(&b"0,1"[..])))
        );
    }

    #[test]
    fn pinned_c_scores_masks_and_style_alignment() {
        use std::fmt::Write as _;
        use std::io::Write as _;
        use std::process::{Command, Stdio};

        let driver = std::env::var_os("RMUX_G10_HELPER_DRIVER")
            .unwrap_or_else(|| "/tmp/swarm-rmux-build/P5FmtAux/driver".into());
        if !std::path::Path::new(&driver).is_file() {
            eprintln!("SKIP fuzzy pinned C comparison: set RMUX_G10_HELPER_DRIVER");
            return;
        }
        let patterns: &[&[u8]] = &[
            b"",
            b" | ",
            b"a",
            b"A",
            b"b",
            b"c",
            b"ab",
            b"ac",
            b"abc",
            b"'a",
            b"'bc",
            b"^a",
            b"c$",
            b"^abc$",
            b"!a",
            b"!x",
            b"!abc",
            b"!",
            b"'",
            b"^",
            b"$",
            b"!^$",
            b"a b",
            b"a a",
            b"a !x",
            b"a !b",
            b"a|b",
            b"c|b",
            b"b|c",
            b"a|A",
            b"x|'bc",
            b"|a||b|",
            b"a\tb",
            b"#",
            b"[",
            b"#[",
            b"a\0B",
            b"\xff",
            b"\xe2\x82",
            "é".as_bytes(),
            "É".as_bytes(),
            "漢".as_bytes(),
            "a漢".as_bytes(),
            "\u{0301}".as_bytes(),
            "a\u{0301}b".as_bytes(),
        ];
        let texts: &[&[u8]] = &[
            b"",
            b"abc",
            b"ABC",
            b"abcbc",
            b"a--a-b",
            b"a_b_c",
            b"a/b.c:b",
            b"abcdefghijkabc",
            b"a\tb\nc",
            b"\xffa\xe2\x82b",
            b"a\0bc",
            b"#[fg=red]abc#[default]",
            b"a##b",
            b"##[abc",
            b"###abc",
            b"###[align=right]abc",
            b"a#[align=right]bc",
            b"a#[align=centre]bc",
            b"#[align=centre]abc",
            b"#[align=absolute-centre]abc",
            b"abc#[align=right]abc",
            b"abc#[align=centre]abc#[align=right]abc",
            b"a#[align=absolute-centre]bc#[align=left]c",
            b"#[align=right]a#[align=bogus]bc",
            b"#[align=right]a#[default]bc",
            b"#[ignore]abc#[noignore]a",
            b"#[bogus]abc",
            b"a#[fg=red",
            b"#[align=right]ab#[align=default]c",
            "a漢b".as_bytes(),
            "漢漢".as_bytes(),
            "a\u{0301}b".as_bytes(),
            "café".as_bytes(),
            "Éé".as_bytes(),
            "a#[align=right]漢b".as_bytes(),
        ];
        let mut input = String::new();
        let mut expected = Vec::new();
        for pattern in patterns {
            for text in texts {
                for width in [0, 1, 2, 3, 4, 5, 8, 13] {
                    input.push_str("fuzzy ");
                    for byte in *pattern {
                        write!(input, "{byte:02x}").unwrap();
                    }
                    input.push(' ');
                    for byte in *text {
                        write!(input, "{byte:02x}").unwrap();
                    }
                    writeln!(input, " {width}").unwrap();
                    let actual = fuzzy_match(pattern, text, width).map_or_else(
                        || "NONE".to_owned(),
                        |result| {
                            let columns = result.columns_text();
                            if columns.is_empty() {
                                result.score.to_string()
                            } else {
                                format!("{} {}", result.score, String::from_utf8_lossy(&columns))
                            }
                        },
                    );
                    expected.push((pattern, text, width, actual));
                }
            }
        }
        let mut child = Command::new(driver)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start pinned fuzzy reference");
        let mut stdin = child.stdin.take().unwrap();
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let reference = String::from_utf8(output.stdout).unwrap();
        let lines: Vec<_> = reference.lines().collect();
        assert_eq!(lines.len(), expected.len(), "reference row count");
        for ((pattern, text, width, actual), reference) in expected.iter().zip(lines) {
            assert_eq!(
                actual, reference,
                "pattern={pattern:?} text={text:?} width={width}"
            );
        }
    }
}
