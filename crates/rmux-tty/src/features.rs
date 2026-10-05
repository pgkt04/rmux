// Ported from tmux tty-features.c @ 8f25579c
/*
 * Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use crate::term::TtyTermFlags;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TtyFeatures {
    pub enabled: u32,
    pub disabled: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct TtyFeature {
    pub name: &'static str,
    pub capabilities: &'static [&'static str],
    pub flags: TtyTermFlags,
}
pub const FEATURES: [TtyFeature; 23] = [
    TtyFeature {
        name: "256",
        capabilities: &[
            "AX",
            "setab=\\E[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m",
            "setaf=\\E[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m",
        ],
        flags: TtyTermFlags::_256COLOURS,
    },
    TtyFeature {
        name: "appesc",
        capabilities: &["Enesc=\\E[?7727h", "Dsesc=\\E[?7727l"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "bpaste",
        capabilities: &["Enbp=\\E[?2004h", "Dsbp=\\E[?2004l"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "ccolour",
        capabilities: &["Cs=\\E]12;%p1%s\\a", "Cr=\\E]112\\a"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "clipboard",
        capabilities: &["Ms=\\E]52;%p1%s;%p2%s\\a"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "hyperlinks",
        capabilities: &["Hls=\\E]8;%?%p1%l%tid=%p1%s%;;%p2%s\\E\\\\"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "cstyle",
        capabilities: &["Ss=\\E[%p1%d q", "Se=\\E[2 q"],
        flags: TtyTermFlags::NOREPLACE,
    },
    TtyFeature {
        name: "extkeys",
        capabilities: &["Eneks=\\E[>4;2m", "Dseks=\\E[>4m"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "focus",
        capabilities: &["Enfcs=\\E[?1004h", "Dsfcs=\\E[?1004l"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "ignorefkeys",
        capabilities: &[
            "kf0@", "kf1@", "kf2@", "kf3@", "kf4@", "kf5@", "kf6@", "kf7@", "kf8@", "kf9@",
            "kf10@", "kf11@", "kf12@", "kf13@", "kf14@", "kf15@", "kf16@", "kf17@", "kf18@",
            "kf19@", "kf20@", "kf21@", "kf22@", "kf23@", "kf24@", "kf25@", "kf26@", "kf27@",
            "kf28@", "kf29@", "kf30@", "kf31@", "kf32@", "kf33@", "kf34@", "kf35@", "kf36@",
            "kf37@", "kf38@", "kf39@", "kf40@", "kf41@", "kf42@", "kf43@", "kf44@", "kf45@",
            "kf46@", "kf47@", "kf48@", "kf49@", "kf50@", "kf51@", "kf52@", "kf53@", "kf54@",
            "kf55@", "kf56@", "kf57@", "kf58@", "kf59@", "kf60@", "kf61@", "kf62@", "kf63@",
        ],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "margins",
        capabilities: &[
            "Enmg=\\E[?69h",
            "Dsmg=\\E[?69l",
            "Clmg=\\E[s",
            "Cmg=\\E[%i%p1%d;%p2%ds",
        ],
        flags: TtyTermFlags::DECSLRM,
    },
    TtyFeature {
        name: "mouse",
        capabilities: &["kmous=\\E[M"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "osc7",
        capabilities: &["Swd=\\E]7;", "fsl=\\a"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "overline",
        capabilities: &["Smol=\\E[53m"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "progressbar",
        capabilities: &["Spb=\\E]9;4;%p1%d;%p2%d\\E\\\\"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "rectfill",
        capabilities: &["Rect"],
        flags: TtyTermFlags::DECFRA,
    },
    TtyFeature {
        name: "RGB",
        capabilities: &[
            "AX",
            "setrgbf=\\E[38;2;%p1%d;%p2%d;%p3%dm",
            "setrgbb=\\E[48;2;%p1%d;%p2%d;%p3%dm",
            "setab=\\E[%?%p1%{8}%<%t4%p1%d%e%p1%{16}%<%t10%p1%{8}%-%d%e48;5;%p1%d%;m",
            "setaf=\\E[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m",
        ],
        flags: TtyTermFlags(TtyTermFlags::_256COLOURS.0 | TtyTermFlags::RGBCOLOURS.0),
    },
    TtyFeature {
        name: "sixel",
        capabilities: &["Sxl"],
        flags: TtyTermFlags::SIXEL,
    },
    TtyFeature {
        name: "strikethrough",
        capabilities: &["smxx=\\E[9m"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "sync",
        capabilities: &["Sync=\\E[?2026%?%p1%{1}%-%tl%eh%;"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "title",
        capabilities: &["tsl=\\E]0;", "fsl=\\a"],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "usstyle",
        capabilities: &[
            "Smulx=\\E[4::%p1%dm",
            "Setulc=\\E[58::2::%p1%{65536}%/%d::%p1%{256}%/%{255}%&%d::%p1%{255}%&%d%;m",
            "Setulc1=\\E[58::5::%p1%dm",
            "ol=\\E[59m",
        ],
        flags: TtyTermFlags(0),
    },
    TtyFeature {
        name: "utf8",
        capabilities: &[],
        flags: TtyTermFlags(0),
    },
];

pub fn parse_features(s: &str, sep: &str, f: &mut TtyFeatures) {
    parse_features_bytes(s.as_bytes(), sep.as_bytes(), f);
}
pub fn parse_features_bytes(s: &[u8], sep: &[u8], f: &mut TtyFeatures) {
    parse(s, sep, &mut f.enabled, Some(&mut f.disabled));
}
pub fn parse_features_enabled_only(s: &str, sep: &str, enabled: &mut u32) {
    parse(s.as_bytes(), sep.as_bytes(), enabled, None);
}
fn parse(s: &[u8], sep: &[u8], enabled: &mut u32, mut disabled: Option<&mut u32>) {
    let s = &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())];
    for part in s.split(|b| sep.contains(b)) {
        let remove = part.last() == Some(&b'@');
        let name = &part[..part.len() - usize::from(remove)];
        let Some(index) = FEATURES
            .iter()
            .position(|f| f.name.as_bytes().eq_ignore_ascii_case(name))
        else {
            break;
        };
        let bit = 1 << index;
        if remove {
            *enabled &= !bit;
            if let Some(disabled) = disabled.as_mut() {
                **disabled |= bit;
            }
        } else if !disabled
            .as_ref()
            .is_some_and(|disabled| **disabled & bit != 0)
        {
            *enabled |= bit;
        }
    }
}
pub fn feature_names(enabled: u32) -> String {
    let mut out = String::new();
    for (index, feature) in FEATURES.iter().enumerate() {
        if enabled & (1 << index) != 0 {
            if !out.is_empty() {
                out.push(',');
            }
            out.push_str(feature.name);
        }
    }
    out
}
pub fn default_features(name: &str, _version: u32, f: &mut TtyFeatures) {
    const BASE: &str = "256,RGB,bpaste,clipboard,mouse,strikethrough,title";
    let extra = match name {
        "mintty" => "appesc,ccolour,cstyle,extkeys,margins,overline,usstyle",
        "tmux" => "ccolour,cstyle,extkeys,focus,overline,usstyle,hyperlinks,progressbar",
        "rxvt-unicode" => {
            parse_features("256,bpaste,ccolour,cstyle,mouse,title,ignorefkeys", ",", f);
            return;
        }
        "iTerm2" => "cstyle,extkeys,margins,usstyle,sync,osc7,hyperlinks,progressbar",
        "foot" => "ccolour,cstyle,extkeys,usstyle,sync,osc7,hyperlinks",
        "WezTerm" => "ccolour,cstyle,extkeys,focus,hyperlinks,margins,usstyle",
        "ghostty" => {
            "ccolour,cstyle,extkeys,focus,margins,overline,hyperlinks,osc7,sync,usstyle,progressbar"
        }
        "Rio" => "ccolour,cstyle,focus,overline,hyperlinks,osc7,sync,usstyle,progressbar",
        "XTerm" => "ccolour,cstyle,extkeys,focus",
        _ => return,
    };
    parse_features(BASE, ",", f);
    parse_features(extra, ",", f);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parsing_order_disable_and_unknown() {
        assert_eq!(
            feature_names(0x7fffff),
            "256,appesc,bpaste,ccolour,clipboard,hyperlinks,cstyle,extkeys,focus,ignorefkeys,margins,mouse,osc7,overline,progressbar,rectfill,RGB,sixel,strikethrough,sync,title,usstyle,utf8"
        );
        let mut f = TtyFeatures::default();
        parse_features("rGb:RGB@:RGB:utf8:unknown:mouse", ":", &mut f);
        assert_eq!(feature_names(f.enabled), "utf8");
        assert_eq!(feature_names(f.disabled), "RGB");
        let mut enabled = 0;
        parse_features_enabled_only("RGB,RGB@,RGB", ",", &mut enabled);
        assert_eq!(feature_names(enabled), "RGB");
        parse_features("mouse,,title", ",", &mut f);
        assert_eq!(feature_names(f.enabled), "mouse,utf8");
    }
    #[test]
    fn all_default_terminal_names() {
        for name in [
            "mintty",
            "tmux",
            "rxvt-unicode",
            "iTerm2",
            "foot",
            "WezTerm",
            "ghostty",
            "Rio",
            "XTerm",
        ] {
            let mut f = TtyFeatures::default();
            default_features(name, 1, &mut f);
            assert!(f.enabled != 0, "{name}");
            let mut later = TtyFeatures::default();
            default_features(name, u32::MAX, &mut later);
            assert_eq!(f, later);
        }
        let mut f = TtyFeatures::default();
        default_features("xterm", 0, &mut f);
        assert_eq!(f.enabled, 0);
    }
}
