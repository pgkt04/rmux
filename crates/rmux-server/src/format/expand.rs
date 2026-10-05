// Ported from tmux format.c @ 8f25579c
use super::{
    FormatFlags, FormatRuntime, FormatTree, FormatValue, OptionScope, parse, true_value, variables,
};
use rmux_sys::regex::{ExecFlags, PosixRegex, RegexFlags, RegexMatch};
use rmux_util::bytes::{ByteString, cstr};

#[derive(Clone)]
struct State {
    depth: u32,
    start: u64,
    time: bool,
    nojobs: bool,
    nocycle: bool,
    now: Option<i64>,
}
pub(super) fn expand(
    tree: &mut FormatTree,
    rt: &mut dyn FormatRuntime,
    input: &[u8],
    time: bool,
) -> ByteString {
    let mut state = State {
        depth: 0,
        start: rt.monotonic_ms(),
        time,
        nojobs: false,
        nocycle: false,
        now: None,
    };
    scan(tree, rt, input, &mut state)
}
fn bool_bytes(value: bool) -> ByteString {
    if value {
        b"1".as_slice().into()
    } else {
        b"0".as_slice().into()
    }
}
fn number(input: &[u8], min: i64, max: i64) -> Option<i64> {
    rmux_util::strtonum::strtonum(input, min, max).ok()
}
fn scan(
    tree: &mut FormatTree,
    rt: &mut dyn FormatRuntime,
    input: &[u8],
    state: &mut State,
) -> ByteString {
    let input = cstr(input);
    if input.is_empty() || state.depth == 100 || rt.monotonic_ms().wrapping_sub(state.start) >= 100
    {
        return ByteString::default();
    }
    let mut child = state.clone();
    child.depth += 1;
    let converted;
    let input = if child.time && input.contains(&b'%') {
        let now = *child.now.get_or_insert_with(|| rt.now().sec);
        state.now = child.now;
        let Some(time) = rmux_sys::time::localtime(now) else {
            return ByteString::default();
        };
        let mut buf = [0; 8192];
        let n = rmux_sys::time::strftime(&mut buf, input, &time);
        if n == 0 {
            return ByteString::default();
        };
        converted = buf[..n].to_vec();
        converted.as_slice()
    } else {
        input
    };
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    let mut style_end = None;
    while i < input.len() {
        if i % 10000 == 0 && rt.monotonic_ms().wrapping_sub(child.start) >= 100 {
            break;
        }
        if input[i] != b'#' {
            out.push(input[i]);
            i += 1;
            continue;
        }
        let Some(&ch) = input.get(i + 1) else {
            break;
        };
        match ch {
            b'(' => {
                let mut level = 1;
                let mut end = i + 2;
                while end < input.len() {
                    if input[end] == b'(' {
                        level += 1;
                    }
                    if input[end] == b')' {
                        level -= 1;
                        if level == 0 {
                            break;
                        }
                    }
                    end += 1;
                }
                if level != 0 {
                    break;
                }
                if !tree.flags.contains(FormatFlags::NOJOBS) && !child.nojobs {
                    let raw = &input[i + 2..end];
                    let mut restricted = child.clone();
                    restricted.time = false;
                    restricted.nojobs = true;
                    restricted.nocycle = true;
                    let command = scan(tree, rt, raw, &mut restricted);
                    let value = rt.job(
                        tree.owner,
                        tree.tag,
                        tree.flags,
                        raw,
                        &command,
                        rt.now().sec,
                    );
                    out.extend_from_slice(&scan(tree, rt, &value, &mut restricted));
                }
                i = end + 1;
            }
            b'{' => {
                let Some(n) = parse::skip_checked(&input[i..], b"}", || {
                    rt.monotonic_ms().wrapping_sub(child.start) < 100
                }) else {
                    break;
                };
                let end = i + n;
                let Some(value) = replace(tree, rt, &input[i + 2..end], &mut child) else {
                    break;
                };
                out.extend_from_slice(&value);
                i = end + 1;
            }
            b'[' | b'#' => {
                let mut end = i;
                while input.get(end) == Some(&b'#') {
                    end += 1;
                }
                let hashes = end - i;
                if input.get(end) == Some(&b'[') {
                    style_end = parse::skip_checked(&input[i..], b"]", || {
                        rt.monotonic_ms().wrapping_sub(child.start) < 100
                    })
                    .map(|n| i + n);
                    out.extend_from_slice(&input[i..=end]);
                    i = end + 1;
                } else if ch == b'#' {
                    out.resize(out.len() + hashes / 2, b'#');
                    i = if hashes % 2 == 1 { end - 1 } else { end };
                } else {
                    out.push(ch);
                    i += 2;
                }
            }
            b'}' | b',' => {
                out.push(ch);
                i += 2;
            }
            _ => {
                let alias = if style_end.is_none_or(|end| i + 2 > end) {
                    match ch {
                        b'D' => Some(b"pane_id".as_slice()),
                        b'F' => Some(b"window_flags".as_slice()),
                        b'H' => Some(b"host".as_slice()),
                        b'I' => Some(b"window_index".as_slice()),
                        b'P' => Some(b"pane_index".as_slice()),
                        b'S' => Some(b"session_name".as_slice()),
                        b'T' => Some(b"pane_title".as_slice()),
                        b'W' => Some(b"window_name".as_slice()),
                        b'h' => Some(b"host_short".as_slice()),
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(key) = alias {
                    let Some(v) = replace(tree, rt, key, &mut child) else {
                        break;
                    };
                    out.extend_from_slice(&v);
                } else {
                    out.extend_from_slice(&input[i..i + 2]);
                }
                i += 2;
            }
        }
    }
    state.now = child.now;
    out.into()
}
fn has(mods: &[parse::Modifier], name: &[u8]) -> bool {
    mods.iter().any(|m| m.name == name)
}
fn last<'a>(mods: &'a [parse::Modifier], name: &[u8]) -> Option<&'a parse::Modifier> {
    mods.iter().rev().find(|m| m.name == name)
}
fn arg(modifier: Option<&parse::Modifier>) -> &[u8] {
    modifier
        .and_then(|m| m.args.first())
        .map_or(b"", |v| v.as_slice())
}
fn find(
    tree: &mut FormatTree,
    rt: &mut dyn FormatRuntime,
    key: &[u8],
    mods: &[parse::Modifier],
) -> Option<ByteString> {
    let mut value = None;
    for scope in [
        OptionScope::Server,
        OptionScope::Pane,
        OptionScope::Window,
        OptionScope::GlobalWindow,
        OptionScope::Session,
        OptionScope::GlobalSession,
    ] {
        if let Some(v) = rt.option(&tree.context, scope, key) {
            value = Some(FormatValue::Bytes(v));
            break;
        }
    }
    if value.is_none() {
        if variables::REGISTRY.binary_search(&key).is_ok() {
            value = variables::find_owned(rt, &tree.context, tree.owner, key);
            value.as_ref()?;
        } else {
            value = tree.custom(rt, key);
        }
    }
    if value.is_none() && !has(mods, b"t") {
        value = rt
            .environment(&tree.context, false, key)
            .or_else(|| rt.environment(&tree.context, true, key))
            .map(FormatValue::Bytes);
    }
    let value = value?;
    let mut bytes = if has(mods, b"t") {
        let sec = match value {
            FormatValue::Time(v) => v.sec,
            v => number(&v.bytes(), 0, i64::MAX)?,
        };
        if sec == 0 {
            return None;
        }
        let flag_sets: Vec<_> = mods
            .iter()
            .filter(|m| m.name == b"t")
            .map(|m| arg(Some(m)))
            .collect();
        let now = rt.now().sec;
        if flag_sets
            .iter()
            .any(|f| !f.contains(&b'p') && f.contains(&b'r'))
        {
            variables::relative_time(sec, now)?
        } else if flag_sets
            .iter()
            .any(|f| !f.contains(&b'p') && !f.contains(&b'r') && f.contains(&b'd'))
        {
            variables::difference_time(sec, now)
        } else if flag_sets.iter().any(|f| f.contains(&b'p')) {
            variables::pretty_time_at(sec, now, false)
        } else if let Some(m) = mods.iter().rev().find(|m| {
            m.name == b"t"
                && m.args.len() >= 2
                && m.args[0].contains(&b'f')
                && !m.args[0].iter().any(|b| b"prd".contains(b))
        }) {
            let fmt = parse::strip(&m.args[1]);
            let tm = rmux_sys::time::localtime(sec)?;
            let mut buf = [0; 512];
            let n = rmux_sys::time::strftime(&mut buf, &fmt, &tm);
            buf[..n].into()
        } else {
            let mut buf = [0; 26];
            let n = rmux_sys::time::ctime(sec, &mut buf)?;
            buf[..n.saturating_sub(1)].into()
        }
    } else {
        match value {
            FormatValue::Time(value) if value.sec == 0 => return None,
            value => value.bytes(),
        }
    };
    if has(mods, b"t") {
        return Some(bytes);
    }
    if has(mods, b"b") {
        bytes = rmux_sys::path::basename(&bytes).into();
    }
    if has(mods, b"d") {
        bytes = rmux_sys::path::dirname(&bytes).into();
    }
    let quotes: Vec<_> = mods
        .iter()
        .filter(|m| m.name == b"q")
        .map(|m| arg(Some(m)))
        .collect();
    if quotes.iter().any(|f| f.is_empty()) {
        bytes = variables::quote_shell(&bytes);
    }
    if quotes.iter().any(|f| f.contains(&b's')) {
        bytes = variables::quote_single(&bytes);
    }
    if quotes
        .iter()
        .any(|f| !f.contains(&b's') && (f.contains(&b'e') || f.contains(&b'h')))
    {
        bytes = variables::quote_style(&bytes);
    }
    if quotes.iter().any(|f| {
        !f.contains(&b's') && !f.contains(&b'e') && !f.contains(&b'h') && f.contains(&b'a')
    }) {
        bytes = crate::cmd::arguments::escape(&bytes);
    }
    Some(bytes)
}
fn choose(input: &[u8]) -> Option<(&[u8], &[u8])> {
    let n = parse::skip(input, b",")?;
    Some((&input[..n], &input[n + 1..]))
}
fn replace(
    tree: &mut FormatTree,
    rt: &mut dyn FormatRuntime,
    input: &[u8],
    state: &mut State,
) -> Option<ByteString> {
    let (mods, key) = parse::modifiers(input, |v| {
        if let Some(v) = v {
            Some(scan(tree, rt, v, state))
        } else {
            (rt.monotonic_ms().wrapping_sub(state.start) < 100).then(ByteString::default)
        }
    });
    let mut value;
    if mods.iter().any(|m| {
        m.name == b"I"
            && m.args
                .first()
                .is_some_and(|f| f.iter().any(|b| b"cfe".contains(b)))
    }) {
        value = ByteString::default();
        for kind in b"cfe" {
            if mods
                .iter()
                .any(|m| m.name == b"I" && arg(Some(m)).contains(kind))
            {
                value = rt
                    .client_query(&tree.context, *kind, key)
                    .unwrap_or_default();
            }
        }
    } else if has(&mods, b"A") {
        value = ByteString::default();
        if tree.flags.contains(FormatFlags::STATUS) && !state.nocycle && !key.is_empty() {
            let count = number(arg(last(&mods, b"A")), 1, 100).unwrap_or(1) as u64;
            let frames: Vec<_> = key.split(|b| *b == b',').collect();
            value = frames[(state.start / (count * 100)) as usize % frames.len()].into();
            if frames.len() > 1 {
                if let Some(owner) = tree.owner {
                    rt.cycle(owner);
                }
            }
        }
    } else if has(&mods, b"l") {
        value = parse::unescape(key);
    } else if has(&mods, b"a") {
        let v = scan(tree, rt, key, state);
        value = number(&v, 32, 126)
            .map(|n| vec![n as u8].into())
            .unwrap_or_default();
    } else if has(&mods, b"c") {
        let v = scan(tree, rt, key, state);
        let foreground = mods
            .iter()
            .any(|m| m.name == b"c" && arg(Some(m)).contains(&b'f'));
        let background = mods
            .iter()
            .any(|m| m.name == b"c" && arg(Some(m)).contains(&b'b'));
        value = ByteString::default();
        if foreground || background {
            if v.eq_ignore_ascii_case(b"none") {
                value = b"\x1b[0m".as_slice().into();
            } else if let Ok(colour) = rmux_emu::colour::parse_colour(&v) {
                let mut out = Vec::new();
                rmux_emu::colour::write_colour_escape(colour, background, None, &mut out);
                value = out.into();
            }
        } else if let Some(rgb) = rmux_emu::colour::parse_colour(&v)
            .ok()
            .and_then(|c| c.force_rgb())
        {
            value = format!("{:06x}", rgb.raw() & 0xffffff).into();
        }
    } else if let Some(kind) = b"SWPLOV".iter().find(|k| has(&mods, &[**k])) {
        let flags = arg(last(&mods, &[*kind]));
        let entries = rt.loop_entries(&tree.context, tree.owner, *kind, flags)?;
        let count = entries.len();
        let mut out = Vec::new();
        let alternatives = if b"SWP".contains(kind) {
            choose(key)
        } else {
            None
        };
        for (index, entry) in entries.into_iter().enumerate() {
            let body = if let Some((normal, active)) = alternatives {
                if entry.active { active } else { normal }
            } else {
                key
            };
            let owner = if *kind == b'L' {
                entry.context.evaluated_client
            } else {
                tree.owner
            };
            let mut child = FormatTree::create(owner, tree.item, entry.tag, tree.flags, rt);
            child.defaults(rt, entry.context);
            child.add(b"loop_index", index.to_string().into());
            child.add(b"loop_last_flag", bool_bytes(index + 1 == count));
            for (k, v) in entry.fields {
                child.add(&k, v);
            }
            out.extend_from_slice(&scan(&mut child, rt, body, state));
            child.release(rt);
        }
        value = out.into();
    } else if let Some(m) = last(&mods, b"N") {
        let flags = arg(Some(m));
        let v = scan(tree, rt, key, state);
        value = bool_bytes(rt.name_exists(
            &tree.context,
            !flags.contains(&b'w') && flags.contains(&b's'),
            &v,
        ));
    } else if let Some(m) = last(&mods, b"C") {
        let v = scan(tree, rt, key, state);
        value = rt
            .search(&tree.context, &v, arg(Some(m)))
            .to_string()
            .into();
    } else if has(&mods, b"R") {
        let (a, b) = choose(key)?;
        let text = scan(tree, rt, a, state);
        let count = scan(tree, rt, b, state);
        value = number(&count, 1, 10000)
            .filter(|n| {
                text.len()
                    .checked_mul(*n as usize)
                    .is_some_and(|n| n <= 65536)
            })
            .map(|n| text.repeat(n as usize).into())
            .unwrap_or_default();
    } else if has(&mods, b"!") || has(&mods, b"!!") {
        let v = scan(tree, rt, key, state);
        value = bool_bytes(true_value(Some(&v)) ^ has(&mods, b"!"));
    } else if let Some(m) = mods
        .iter()
        .rev()
        .find(|m| m.name == b"||" || m.name == b"&&")
    {
        let and = m.name == b"&&";
        let mut rest = key;
        let answer = loop {
            let part = choose(rest);
            let operand = part.map_or(rest, |(a, _)| a);
            let v = scan(tree, rt, operand, state);
            let truth = true_value(Some(&v));
            if truth != and || part.is_none() {
                break truth;
            }
            rest = part.unwrap().1;
        };
        value = bool_bytes(answer);
    } else if let Some(m) = mods.iter().rev().find(|m| {
        [b"m".as_slice(), b"<", b">", b"==", b"!=", b"<=", b">="].contains(&m.name.as_slice())
    }) {
        let (a, b) = choose(key)?;
        let a = scan(tree, rt, a, state);
        let b = scan(tree, rt, b, state);
        value = if m.name == b"m" {
            matching(&a, &b, arg(Some(m)))
        } else {
            bool_bytes(match m.name.as_slice() {
                b"==" => a == b,
                b"!=" => a != b,
                b"<" => a < b,
                b">" => a > b,
                b"<=" => a <= b,
                b">=" => a >= b,
                _ => false,
            })
        };
    } else if key.first() == Some(&b'?') {
        let mut rest = &key[1..];
        value = loop {
            let Some((condition, values)) = choose(rest) else {
                break scan(tree, rt, rest, state);
            };
            let found = find(tree, rt, condition, &mods).unwrap_or_else(|| {
                let v = scan(tree, rt, condition, state);
                if v.as_slice() == condition {
                    ByteString::default()
                } else {
                    v
                }
            });
            let pair = choose(values);
            if true_value(Some(&found)) {
                break scan(tree, rt, pair.map_or(values, |(v, _)| v), state);
            }
            let Some((_, next)) = pair else {
                break ByteString::default();
            };
            rest = next;
        };
    } else if let Some(m) = last(&mods, b"e").filter(|m| (1..=3).contains(&m.args.len())) {
        value = expression(tree, rt, key, state, m).unwrap_or_default();
    } else if key.windows(2).any(|v| v == b"#{") {
        value = scan(tree, rt, key, state);
    } else {
        value = find(tree, rt, key, &mods).unwrap_or_default();
    }
    if has(&mods, b"E") {
        value = scan(tree, rt, &value, state);
    } else if has(&mods, b"T") {
        let mut child = state.clone();
        child.time = true;
        value = scan(tree, rt, &value, &mut child);
        state.now = child.now;
    }
    for m in mods.iter().filter(|m| m.name == b"s" && m.args.len() >= 2) {
        let pattern = scan(tree, rt, &m.args[0], state);
        let with = scan(tree, rt, &m.args[1], state);
        let flags = if m.args.get(2).is_some_and(|v| v.contains(&b'i')) {
            RegexFlags::EXTENDED | RegexFlags::ICASE
        } else {
            RegexFlags::EXTENDED
        };
        if let Ok(new) = super::regsub::substitute(&pattern, &with, &value, flags) {
            value = new;
        }
    }
    if let Some(m) = last(&mods, b"=") {
        let width = number(arg(Some(m)), -10000, 10000).unwrap_or(0);
        if width != 0 {
            let mut new = if width > 0 {
                super::draw::trim_left(&value, width as u32)
            } else {
                super::draw::trim_right(&value, (-width) as u32)
            };
            if new != value {
                if let Some(marker) = m.args.get(1) {
                    if width > 0 {
                        new.0.extend_from_slice(marker);
                    } else {
                        let mut out = marker.0.clone();
                        out.extend_from_slice(&new);
                        new = out.into();
                    }
                }
            }
            value = new;
        }
    }
    if let Some(m) = last(&mods, b"p") {
        let width = number(arg(Some(m)), -10000, 10000).unwrap_or(0);
        if width > 0 {
            value = rmux_util::utf8::pad_right(&value, width as u32);
        } else if width < 0 {
            value = rmux_util::utf8::pad_left(&value, (-width) as u32);
        }
    }
    if has(&mods, b"n") {
        value = value.len().to_string().into();
    }
    if has(&mods, b"w") {
        value = super::draw::width(&value).to_string().into();
    }
    Some(value)
}
fn matching(pattern: &[u8], text: &[u8], flags: &[u8]) -> ByteString {
    if flags.contains(&b'p') || flags.contains(&b'z') {
        let result = super::fuzzy::fuzzy_match(pattern, text, super::draw::width(text).max(1));
        if !flags.contains(&b'p') {
            return bool_bytes(result.is_some());
        }
        return result.map(|m| m.columns_text()).unwrap_or_default();
    }
    let yes = if flags.contains(&b'r') {
        let mut regex_flags = RegexFlags::EXTENDED | RegexFlags::NOSUB;
        if flags.contains(&b'i') {
            regex_flags |= RegexFlags::ICASE;
        }
        PosixRegex::new(pattern, regex_flags).ok().is_some_and(|r| {
            r.exec(text, &mut RegexMatch::new(0), ExecFlags::NONE)
                .unwrap_or(false)
        })
    } else {
        rmux_sys::fnmatch::fnmatch(
            pattern,
            text,
            if flags.contains(&b'i') {
                rmux_sys::fnmatch::FnmatchFlags::CASEFOLD
            } else {
                rmux_sys::fnmatch::FnmatchFlags::NONE
            },
        )
    };
    bool_bytes(yes)
}
fn expression(
    tree: &mut FormatTree,
    rt: &mut dyn FormatRuntime,
    key: &[u8],
    state: &mut State,
    m: &parse::Modifier,
) -> Option<ByteString> {
    let (a, b) = choose(key)?;
    let a = scan(tree, rt, a, state);
    let b = scan(tree, rt, b, state);
    let (mut left, n) = rmux_sys::number::strtod(&a);
    if n != a.len() {
        return None;
    }
    let (mut right, n) = rmux_sys::number::strtod(&b);
    if n != b.len() {
        return None;
    }
    let float = m.args.get(1).is_some_and(|v| v.contains(&b'f'));
    let precision = if let Some(v) = m.args.get(2) {
        number(v, -100, 100)? as i32
    } else if float {
        2
    } else {
        0
    };
    if !float {
        left = rmux_sys::number::format_integer_operand(left);
        right = rmux_sys::number::format_integer_operand(right);
    }
    let mut result = match m.args[0].as_slice() {
        b"+" => left + right,
        b"-" => left - right,
        b"*" => left * right,
        b"/" => left / right,
        b"%" | b"m" => left % right,
        b"==" => ((left - right).abs() < 1e-9) as u8 as f64,
        b"!=" => ((left - right).abs() > 1e-9) as u8 as f64,
        b">" => (left > right) as u8 as f64,
        b">=" => (left >= right) as u8 as f64,
        b"<" => (left < right) as u8 as f64,
        b"<=" => (left <= right) as u8 as f64,
        _ => return None,
    };
    if !float {
        result = rmux_sys::number::format_integer_operand(result);
    }
    let mut out = Vec::new();
    rmux_sys::number::printf_fixed(&mut out, precision, result).ok()?;
    Some(out.into())
}
