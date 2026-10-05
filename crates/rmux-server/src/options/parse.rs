// Ported from tmux options.c @ 8f25579c
//! Pure name handling: array keys, `name[key]` parsing, table search,
//! alias mapping, prefix matching, and choice lookup.

use std::fmt;

use rmux_util::bytes::ByteString;
use rmux_util::strtonum::strtonum;

use super::table::{OPTIONS_OTHER_NAMES, OPTIONS_TABLE};
use super::{OptionName, OptionsArrayKey, OptionsError, OptionsTableEntry};

impl OptionsArrayKey {
    /// `options_array_correct_key`: digits only become a canonical `Index`
    /// (`007` is `7`); anything else is a `Name`. An empty key or a number
    /// above `u32::MAX` is "bad array key: K" (`options.c:40-76`).
    pub fn parse(key: &[u8]) -> Result<OptionsArrayKey, OptionsError> {
        match Self::to_number(key) {
            Some(Ok(idx)) => Ok(OptionsArrayKey::Index(idx)),
            Some(Err(())) => Err(OptionsError::new(b"bad array key: ", key)),
            None => Ok(OptionsArrayKey::Name(ByteString::from(key))),
        }
    }

    /// `options_array_key_to_number`: `None` for a text key, `Some(Err)` for
    /// an empty or overflowing numeric key.
    fn to_number(key: &[u8]) -> Option<Result<u32, ()>> {
        if key.is_empty() {
            return Some(Err(()));
        }
        if !key.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(
            strtonum(key, 0, i64::from(u32::MAX))
                .map(|n| n as u32)
                .map_err(|_| ()),
        )
    }

    /// The key bytes as `show-options` prints them inside `[...]`.
    pub fn to_bytes(&self) -> ByteString {
        match self {
            OptionsArrayKey::Index(i) => ByteString::from(i.to_string()),
            OptionsArrayKey::Name(n) => n.clone(),
        }
    }

    /// Append the key bytes to `out` without an intermediate allocation.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        use std::io::Write;
        match self {
            OptionsArrayKey::Index(i) => write!(out, "{i}").unwrap(),
            OptionsArrayKey::Name(n) => out.extend_from_slice(n),
        }
    }
}

impl fmt::Display for OptionsArrayKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptionsArrayKey::Index(i) => write!(f, "{i}"),
            OptionsArrayKey::Name(n) => write!(f, "{n}"),
        }
    }
}

/// `options_map_name`: the alias map, applied once (`options.c:153-163`).
pub fn map_name(name: &[u8]) -> &[u8] {
    for map in OPTIONS_OTHER_NAMES {
        if map.from == name {
            return map.to;
        }
    }
    name
}

/// `options_search`: exact table lookup, no alias map (`options.c:808-818`).
pub fn search(name: &[u8]) -> Option<&'static OptionsTableEntry> {
    OPTIONS_TABLE.iter().find(|oe| oe.name == name)
}

/// `options_parse`: split `name[key]`. `None` for an empty name, an empty
/// key, a missing `]`, bytes after `]`, or a bad numeric key
/// (`options.c:758-785`). The base name borrows from `name`; `"[1]"` gives
/// an empty base.
pub fn parse_name(name: &[u8]) -> Option<(&[u8], Option<OptionsArrayKey>)> {
    if name.is_empty() {
        return None;
    }
    let Some(open) = name.iter().position(|&b| b == b'[') else {
        return Some((name, None));
    };
    let rest = &name[open + 1..];
    let close = rest.iter().position(|&b| b == b']')?;
    if close + 1 != rest.len() || close == 0 {
        return None;
    }
    let key = OptionsArrayKey::parse(&rest[..close]).ok()?;
    Some((&name[..open], Some(key)))
}

/// `options_match` returned `ambiguous = 1`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Ambiguous;

/// `options_match`: `@` names pass through; otherwise the alias-mapped name
/// is matched exactly or as a unique prefix over the table in source order
/// (`options.c:820-864`). `Ok(None)` is no match or an unparsable name.
pub fn match_name(s: &[u8]) -> Result<Option<(OptionName, Option<OptionsArrayKey>)>, Ambiguous> {
    let Some((parsed, key)) = parse_name(s) else {
        return Ok(None);
    };
    if parsed.first() == Some(&b'@') {
        return Ok(Some((OptionName::User(ByteString::from(parsed)), key)));
    }
    let name = map_name(parsed);
    let mut found: Option<&'static OptionsTableEntry> = None;
    for oe in OPTIONS_TABLE {
        if oe.name == name {
            found = Some(oe);
            break;
        }
        if oe.name.starts_with(name) {
            if found.is_some() {
                return Err(Ambiguous);
            }
            found = Some(oe);
        }
    }
    Ok(found.map(|oe| (OptionName::Table(oe.name), key)))
}

/// `options_find_choice`: the last exact match wins; "unknown value: V"
/// otherwise (`options.c:1238-1255`).
pub fn find_choice(oe: &OptionsTableEntry, value: &[u8]) -> Result<i64, OptionsError> {
    let mut choice: Option<i64> = None;
    for (n, cp) in oe.choices.unwrap_or(&[]).iter().enumerate() {
        if *cp == value {
            choice = Some(n as i64);
        }
    }
    choice.ok_or_else(|| OptionsError::new(b"unknown value: ", value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> OptionsArrayKey {
        OptionsArrayKey::parse(s.as_bytes()).unwrap()
    }

    #[test]
    fn array_key_order_matches_options_array_cmp() {
        let mut keys = [
            key("a"),
            key("10"),
            key("9"),
            key("007"),
            key("b"),
            key("4294967295"),
            key("0"),
            key("a b"),
        ];
        keys.sort();
        let printed: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        assert_eq!(
            printed,
            ["0", "7", "9", "10", "4294967295", "a", "a b", "b"]
        );
        assert_eq!(key("007"), OptionsArrayKey::Index(7));
        assert_eq!(key("abc"), OptionsArrayKey::Name(ByteString::from("abc")));
        assert_eq!(
            OptionsArrayKey::parse(b"").unwrap_err().as_bytes(),
            b"bad array key: "
        );
        assert_eq!(
            OptionsArrayKey::parse(b"4294967296")
                .unwrap_err()
                .as_bytes(),
            b"bad array key: 4294967296"
        );
    }

    #[test]
    fn parse_name_cases() {
        assert_eq!(parse_name(b"x"), Some((&b"x"[..], None)));
        assert_eq!(
            parse_name(b"x[1]"),
            Some((&b"x"[..], Some(OptionsArrayKey::Index(1))))
        );
        assert_eq!(parse_name(b"x[]"), None);
        assert_eq!(parse_name(b"x[1]y"), None);
        assert_eq!(parse_name(b"x[1"), None);
        assert_eq!(parse_name(b"x[4294967296]"), None);
        assert_eq!(
            parse_name(b"[1]"),
            Some((&b""[..], Some(OptionsArrayKey::Index(1))))
        );
        assert_eq!(parse_name(b""), None);
        assert_eq!(
            parse_name(b"x[a b]"),
            Some((
                &b"x"[..],
                Some(OptionsArrayKey::Name(ByteString::from("a b")))
            ))
        );
    }

    #[test]
    fn match_name_cases() {
        for oe in OPTIONS_TABLE {
            let (name, _) = match_name(oe.name).unwrap().unwrap();
            assert_eq!(name, OptionName::Table(oe.name), "{}", oe.name_str());
        }
        for map in OPTIONS_OTHER_NAMES {
            let (name, _) = match_name(map.from).unwrap().unwrap();
            assert_eq!(name, OptionName::Table(map.to));
        }
        assert_eq!(match_name(b"status-l"), Err(Ambiguous));
        assert_eq!(match_name(b"[1]"), Err(Ambiguous));
        assert_eq!(
            match_name(b"status-inte"),
            Ok(Some((OptionName::Table(b"status-interval"), None)))
        );
        assert_eq!(
            match_name(b"@x"),
            Ok(Some((OptionName::User(ByteString::from("@x")), None)))
        );
        assert_eq!(match_name(b"nonexistent-option"), Ok(None));
        assert_eq!(match_name(b"x[]"), Ok(None));
        // An exact match wins over a single earlier prefix match.
        assert_eq!(
            match_name(b"status"),
            Ok(Some((OptionName::Table(b"status"), None)))
        );
    }

    #[test]
    fn find_choice_cases() {
        let oe = search(b"mode-keys").unwrap();
        assert_eq!(find_choice(oe, b"vi"), Ok(1));
        assert_eq!(
            find_choice(oe, b"vim").unwrap_err().as_bytes(),
            b"unknown value: vim"
        );
        assert_eq!(
            find_choice(oe, b"").unwrap_err().as_bytes(),
            b"unknown value: "
        );
    }
}
