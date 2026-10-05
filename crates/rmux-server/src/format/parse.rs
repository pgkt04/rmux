// Ported from tmux format.c @ 8f25579c
use rmux_util::bytes::ByteString;

pub fn skip(input: &[u8], terminators: &[u8]) -> Option<usize> {
    skip_checked(input, terminators, || true)
}
pub(crate) fn skip_checked(
    input: &[u8],
    terminators: &[u8],
    mut check: impl FnMut() -> bool,
) -> Option<usize> {
    let mut brackets = 0i32;
    let mut i = 0;
    while i < input.len() && input[i] != 0 {
        if i % 10000 == 9999 && !check() {
            return None;
        }
        if input[i] == b'#' && input.get(i + 1) == Some(&b'{') {
            brackets += 1;
        }
        if input[i] == b'#' && input.get(i + 1).is_some_and(|b| b",#{}:".contains(b)) {
            i += 2;
            continue;
        }
        if input[i] == b'}' {
            brackets -= 1;
        }
        if brackets == 0 && terminators.contains(&input[i]) {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn unescape(input: &[u8]) -> ByteString {
    let mut out = Vec::with_capacity(input.len());
    let mut brackets = 0i32;
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'#' && input.get(i + 1) == Some(&b'{') {
            brackets += 1;
        }
        if brackets == 0
            && input[i] == b'#'
            && input.get(i + 1).is_some_and(|b| b",#{}:".contains(b))
        {
            i += 1;
            out.push(input[i]);
            i += 1;
            continue;
        }
        if input[i] == b'}' {
            brackets -= 1;
        }
        out.push(input[i]);
        i += 1;
    }
    out.into()
}

pub fn strip(input: &[u8]) -> ByteString {
    let mut out = Vec::with_capacity(input.len());
    let mut brackets = 0i32;
    for (i, &b) in input.iter().enumerate() {
        if b == b'#' && input.get(i + 1) == Some(&b'{') {
            brackets += 1;
        }
        if b == b'#' && input.get(i + 1).is_some_and(|b| b",#{}:".contains(b)) {
            if brackets != 0 {
                out.push(b);
            }
            continue;
        }
        if b == b'}' {
            brackets -= 1;
        }
        out.push(b);
    }
    out.into()
}

#[derive(Debug)]
pub(crate) struct Modifier {
    pub name: Vec<u8>,
    pub args: Vec<ByteString>,
}

pub(crate) fn modifiers(
    input: &[u8],
    mut expand: impl FnMut(Option<&[u8]>) -> Option<ByteString>,
) -> (Vec<Modifier>, &[u8]) {
    let mut list = Vec::new();
    let mut i = 0;
    let end = |b: Option<&u8>| b.is_some_and(|b| b";:".contains(b));
    while i < input.len() && input[i] != b':' {
        if input[i] == b';' {
            i += 1;
        }
        let Some(&c) = input.get(i) else {
            break;
        };
        if b"labdnwETSWPOVL!<>A".contains(&c) && end(input.get(i + 1)) {
            list.push(Modifier {
                name: vec![c],
                args: Vec::new(),
            });
            i += 1;
            continue;
        }
        if let Some(pair) = input.get(i..i + 2) {
            if [b"||".as_slice(), b"&&", b"!!", b"!=", b"==", b"<=", b">="].contains(&pair)
                && end(input.get(i + 2))
            {
                list.push(Modifier {
                    name: pair.to_vec(),
                    args: Vec::new(),
                });
                i += 2;
                continue;
            }
        }
        if !b"ImCLNPSOVst=pReqWcA".contains(&c) {
            break;
        }
        if end(input.get(i + 1)) {
            list.push(Modifier {
                name: vec![c],
                args: Vec::new(),
            });
            i += 1;
            continue;
        }
        let Some(&wrapper) = input.get(i + 1) else {
            break;
        };
        let mut args = Vec::new();
        if !wrapper.is_ascii_punctuation() || wrapper == b'-' {
            let Some(n) = skip_checked(&input[i + 1..], b":;", || expand(None).is_some()) else {
                break;
            };
            let Some(value) = expand(Some(&unescape(&input[i + 1..i + 1 + n]))) else {
                break;
            };
            args.push(value);
            i += 1 + n;
        } else {
            i += 1;
            loop {
                if input.get(i) == Some(&wrapper) && end(input.get(i + 1)) {
                    i += 1;
                    break;
                }
                let Some(rest) = input.get(i + 1..) else {
                    break;
                };
                let Some(n) = skip_checked(rest, &[wrapper, b';', b':'], || expand(None).is_some())
                else {
                    break;
                };
                let Some(value) = expand(Some(&unescape(&rest[..n]))) else {
                    break;
                };
                args.push(value);
                i += 1 + n;
                if end(input.get(i)) {
                    break;
                }
            }
        }
        list.push(Modifier {
            name: vec![c],
            args,
        });
    }
    if input.get(i) == Some(&b':') {
        (list, &input[i + 1..])
    } else {
        (Vec::new(), input)
    }
}
