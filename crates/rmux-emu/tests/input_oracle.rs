//! End-to-end differential test: feed the same byte streams to the oracle
//! tmux (`cat` of a file into a fresh pane) and to `InputCtx`, then compare
//! `capture-pane -p -e -N -S -`, `capture-pane -p -F -N -S -` and a
//! `display -p` of the screen state byte for byte.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;
mod input_corpus;

use input_corpus::Emu;
use rmux_emu::input::dump::STATE_FORMAT;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const SX: u32 = 80;
const SY: u32 = 24;
const HLIMIT: u32 = 2000;

type Stream = (&'static str, Vec<u8>);

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn repeat(part: &[u8], n: usize) -> Vec<u8> {
    part.repeat(n)
}

/// Every G05 sequence family, one or more streams each.
fn corpus() -> Vec<Stream> {
    let mut v: Vec<Stream> = Vec::new();
    let mut add = |name: &'static str, bytes: Vec<u8>| v.push((name, bytes));

    // C0.
    add(
        "c0-basic",
        cat(&[b"abc\x07def\x08\x08X\tY\nZ\x0bW\x0cV\rU\x00after-nul\x7fdel"]),
    );
    add(
        "c0-tabs",
        cat(&[
            b"a\tb\tc\tdefghijklmnop\tq\n",
            b"abcdefgh\r\tover-content\n",
            b"        \r\tover-spaces\n",
            b"\x1b[1;31m  \x1b[0m\r\tcoloured-spaces\n",
            b"\x1b[80G\tX\n",
            b"\x1b[78Gab\tc\n",
            b"\x1b[75G\t\tY\n",
            &repeat(b"\t", 12),
            b"end",
        ]),
    );
    add(
        "c0-so-si",
        cat(&[b"\x1b)0plain\x0eqqqxxjk\x0fback\x1b)Bagain\x0eqq\x0f"]),
    );
    add(
        "c0-crlf-vt-ff",
        cat(&[b"line1\nline2\x0bline3\x0cline4\r\nline5\r\rline6"]),
    );

    // ESC.
    add(
        "esc-ris",
        cat(&[
            b"\x1b[1;4;31mbefore\x1b[5;10r\x1b[?6h\x1b[?25l\x1b[=\x1b[?1000h",
            &repeat(b"fill\n", 30),
            b"\x1bcafter",
        ]),
    );
    add(
        "esc-ind-nel-ri",
        cat(&[
            b"top\x1bDind\x1bEnel\x1bMri\x1bM\x1bM\x1bMscrolled-down\x1b[24;1Hbottom\x1bD\x1bDpast",
        ]),
    );
    add(
        "esc-hts-tbc-cbt",
        cat(&[
            b"\x1b[5G\x1bH\x1b[13G\x1bH\x1b[1G\tA\tB\tC\n",
            b"\x1b[13G\x1b[g\x1b[1G\tA\tB\n",
            b"\x1b[3g\x1b[1G\tA\tB\n",
            b"\x1b[70G\x1b[Zz\x1b[2Zy\x1b[9Zx\n",
            b"\x1b[81G\x1bHx\x1b[80G\x1b[gy",
        ]),
    );
    add("esc-keypad-set", cat(&[b"\x1b=kp"]));
    add("esc-keypad-reset", cat(&[b"\x1b=\x1b>kp"]));
    add(
        "esc-decsc-decrc",
        cat(&[
            b"\x1b[5;10H\x1b[1;32m\x1b(0\x1b7\x1b[1;1H\x1b[0mXX\x1b8qq\x1b(Bplain\n",
            b"\x1b[?6h\x1b[3;20r\x1b[2;2H\x1b7\x1b[?6l\x1b[1;1H\x1b8Z",
        ]),
    );
    add("esc-decaln", cat(&[b"x\x1b#8\x1b[12;40Hmid"]));
    add(
        "esc-charsets",
        cat(&[
            b"\x1b(0lqqqqk\n",
            b"\x1b(Babc\x1b)0\x0eqqxx\x0fabc\n",
            b"\x1b(0tuvwxyz{|}~`abcdefghijklmnopqrs\x1b(B",
        ]),
    );
    add(
        "esc-st-unknown",
        cat(&[b"a\x1b\\b\x1bZc\x1b d\x1b#9e\x1b(Zf"]),
    );

    // CSI cursor movement and editing.
    add(
        "csi-cursor",
        cat(&[
            b"0123456789\r\x1b[3@ins\x1b[5;10HX\x1b[AU\x1b[2BD\x1b[5CF\x1b[DB\x1b[2Enl\x1b[Fpl",
            b"\x1b[10Ghpa\x1b[5dvpa\x1b[s\x1b[1;1Hhome\x1b[uback\x1b[0;0Hzero\x1b[100;200Hclamp",
            b"\x1b[1;1H\x1b[0A\x1b[0B\x1b[0C\x1b[0Dzero-moves\x1b[;5Hsemi",
        ]),
    );
    add(
        "csi-erase",
        cat(&[
            &repeat(
                b"abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\n",
                23,
            ),
            b"\x1b[5;20H\x1b[K\x1b[6;20H\x1b[1K\x1b[7;20H\x1b[2K\x1b[8;20H\x1b[4X\x1b[9;20H\x1b[3P",
            b"\x1b[10;20H\x1b[41m\x1b[K\x1b[11;20H\x1b[44m\x1b[1K\x1b[12;20H\x1b[42m\x1b[5X\x1b[0m",
            b"\x1b[20;40H\x1b[J\x1b[3;40H\x1b[1J",
        ]),
    );
    add(
        "csi-ed2-ed3",
        cat(&[
            &repeat(b"history line\n", 30),
            b"\x1b[2Jcleared\x1b[3Jhistory-gone\x1b[2;3J",
        ]),
    );
    add(
        "csi-lines",
        cat(&[
            &repeat(b"row\n", 23),
            b"\x1b[5;15r\x1b[7;1H\x1b[2Lins\x1b[10;1H\x1b[M\x1b[2Sup\x1b[Tdown",
            b"\x1b[4;1H\x1b[Loutside\x1b[16;1H\x1b[Moutside2\x1b[r\x1b[1;1H\x1b[3Lall",
            b"\x1b[43m\x1b[12;1H\x1b[2L\x1b[14;1H\x1b[S\x1b[0m",
        ]),
    );
    add(
        "csi-rep",
        cat(&[
            b"ab\x1b[5bc\x1b[bd\x1b[0b\n",
            b"\x1b(0q\x1b(B\x1b[3b\n",
            b"\xe6\x97\xa5\x1b[3b\n",
            b"x\x1b[?9999z\x1b[3b\n",
            b"\x1b[5by\x1b[2A\x1b[3b\n",
            b"\x1b[78Gz\x1b[10b",
        ]),
    );
    add(
        "csi-decscusr",
        cat(&[b"\x1b[2 qa\x1b[5 qb\x1b[0 qc\x1b[ qd"]),
    );
    add(
        "csi-insert-mode",
        cat(&[b"abcdef\r\x1b[4hXY\x1b[4lZ\x1b[34hvv\x1b[34lww\x1b[4h\x1b[70Glonginsertion"]),
    );
    add(
        "csi-private-modes-set",
        cat(&[
            b"\x1b[?1h\x1b[?7l\x1b[?25l\x1b[?1000h\x1b[?1005h\x1b[?1006h\x1b[?2004h\x1b[?12h",
            b"\x1b[?1004h\x1b[34l\x1b[?1002h\x1b[?3h",
        ]),
    );
    add(
        "csi-private-modes-list",
        cat(&[b"\x1b[?1;1004;2004;1003;7l\x1b[?1;25;1000h\x1b[?1001l"]),
    );
    add(
        "csi-private-modes-reset",
        cat(&[b"\x1b[?1000h\x1b[?1001l\x1b[?1006h\x1b[?1006l\x1b[?1003h\x1b[?1003l"]),
    );
    add(
        "csi-origin",
        cat(&[b"\x1b[5;10r\x1b[?6hA\x1b[HB\x1b[20;1HC\x1b[1;1HD\x1b[?6l\x1b[HE\x1b[?6hF"]),
    );
    add(
        "csi-decstbm",
        cat(&[
            b"\x1b[10;5rbad\x1b[2;24r\x1b[0;0r\x1b[24;24r\x1b[1;1r\x1b[3;30r\x1b[0;10r\x1b[30;40r",
        ]),
    );
    add(
        "csi-alt-47",
        cat(&[b"main\x1b[?47halt\x1b[5;5Hx\x1b[?47lback\x1b[?47h\x1b[?47lagain"]),
    );
    add(
        "csi-alt-1047",
        cat(&[b"main\x1b[?1047halt\x1b[5;5Hx\x1b[?1047lback"]),
    );
    add(
        "csi-alt-1049",
        cat(&[
            &repeat(b"main\n", 30),
            b"\x1b[10;20Hcur\x1b[1;31m\x1b[?1049h\x1b[0malt\x1b[5;5Hx\x1b[?1049lback",
        ]),
    );
    add(
        "csi-alt-1049-stay",
        cat(&[b"main\x1b[7;9H\x1b[?1049halt\x1b[?1049hagain\x1b[3;3Hy"]),
    );
    add(
        "csi-alt-off-without-on",
        cat(&[b"main\x1b[4;6H\x1b[?1049loff\x1b[?47loff2"]),
    );
    add(
        "csi-sync",
        cat(&[b"\x1b[?2026hsynced\x1b[2;2Hmore\x1b[?2026lafter\x1b[?2026l\x1b[?2026h"]),
    );
    add(
        "csi-winops-title",
        cat(&[
            b"\x1b]2;first\x07\x1b[22;0t\x1b]2;second\x07\x1b[22;2t\x1b]2;third\x07\x1b[23;0t",
            b"\x1b[23;2t\x1b[23;0t\x1b[22;1t\x1b[23;1t\x1b[1t\x1b[3;1;2t\x1b[9;1t\x1b[22t",
        ]),
    );
    add("csi-winops-push-many", {
        let mut b = Vec::new();
        for i in 0..14 {
            b.extend_from_slice(format!("\x1b]2;title{i}\x07\x1b[22;0t").as_bytes());
        }
        for _ in 0..16 {
            b.extend_from_slice(b"\x1b[23;0t");
        }
        b
    });
    add(
        "csi-queries",
        cat(&[b"a\x1b[cb\x1b[>cc\x1b[5nd\x1b[6ne\x1b[?4$pf\x1b[$pg\x1b[?25$ph\x1b[>qi\x1b[18tj\x1b[?996nk"]),
    );
    add(
        "csi-discard",
        cat(&[
            b"\x1b[",
            &repeat(b"1", 80),
            b"\x18OK\n",
            b"\x1b[    \x18OK\n",
            b"\x1b[",
            &repeat(b"1;", 24),
            b"5H24params\n",
            b"\x1b[",
            &repeat(b"1;", 22),
            b"5H23params\n",
            b"\x1b[!!!!Hfour-intermediates\n",
            b"\x1b[!!!Hthree\n",
            b"\x1b[",
            &repeat(b"1", 63),
            b"Csixty-three\n",
            b"\x1b[",
            &repeat(b"1", 64),
            b"Csixty-four",
        ]),
    );
    add(
        "csi-c0-inside",
        cat(&[
            b"a\x1b[2\nCb\x1b[\x075Cc\x1b\n[3Cd\x1b[2\x18Xe\x1b[2\x1aXf\x1b[\rg\x1b[1\x08\x08h",
            b"\x1b[?1\x0a5h\x1b]2;ti\x18tle\x07\x1bP\x18q\x1b\\k",
        ]),
    );
    add(
        "csi-unknown",
        cat(&[b"\x1b[?9999zOK\x1b[>4;2m\x1b[>4m\x1b[?1;2$p\x1b[>Z\x1b[?;;z\x1b[=1hz"]),
    );
    add(
        "csi-high-bytes-inside",
        cat(&[b"\x1b[\xff2C\xc3\xa9\x1b\xff[1C\x1bP\xffq\x1b\\x"]),
    );

    // SGR.
    add(
        "sgr-attrs",
        cat(&[
            b"\x1b[1mbold\x1b[2mdim\x1b[3mital\x1b[4mul\x1b[5mblink\x1b[6mblink2\x1b[7mrev",
            b"\x1b[8mhid\x1b[9mstrike\x1b[21mdul\x1b[53mover\x1b[0m\n",
            b"\x1b[1;2m\x1b[22mnobold\x1b[3m\x1b[23m\x1b[4m\x1b[24m\x1b[21m\x1b[24m\x1b[5m\x1b[25m",
            b"\x1b[7m\x1b[27m\x1b[8m\x1b[28m\x1b[9m\x1b[29m\x1b[53m\x1b[55mcleared\n",
            b"\x1b[4m\x1b[21mdouble\x1b[4msingle\x1b[4:3mcurly\x1b[4:4mdotted\x1b[4:5mdashed\x1b[4:0moff",
            b"\x1b[4:1msgl\x1b[4:2mdbl\x1b[24m\n",
            b"\x1b[1;4;31;42mcombo\x1b[mreset\x1b[;1mlead-empty\x1b[1;;4mmid-empty\x1b[0m",
        ]),
    );
    add("sgr-colours", {
        let mut b = Vec::new();
        for n in (30..=37)
            .chain([39])
            .chain(40..=47)
            .chain([49])
            .chain(90..=97)
            .chain(100..=107)
        {
            b.extend_from_slice(format!("\x1b[{n}m{n:x}").as_bytes());
        }
        b.extend_from_slice(b"\x1b[0m\n\x1b[59mus\x1b[58;5;17mus17\x1b[59mdefault\x1b[0m");
        b
    });
    add(
        "sgr-256-rgb",
        cat(&[
            b"\x1b[38;5;123ma\x1b[48;5;200mb\x1b[58;5;17mc\x1b[38;2;1;2;3md\x1b[48;2;255;128;0me",
            b"\x1b[58;2;9;8;7mf\x1b[0m\n",
            b"\x1b[38;2;1;2mtrunc\x1b[0m\x1b[38;5mmissing\x1b[0m\x1b[38;2;300;1;1mbad\x1b[0m",
            b"\x1b[38mnone\x1b[0m\x1b[48;9munknown\x1b[0m\x1b[38;5;300mbig\x1b[0m\n",
            b"\x1b[38;2;1;2;3;4mextra\x1b[0m\x1b[31;38;5;9;42mmix\x1b[0m\x1b[38;2;1;2;-1mneg\x1b[0m",
            b"\x1b[38;5;;1mempty-idx\x1b[0m\x1b[38;2;;;mempty-rgb\x1b[0m",
        ]),
    );
    add(
        "sgr-colon",
        cat(&[
            b"\x1b[38:2::255:0:0ma\x1b[38:2:255:0:0mb\x1b[38:5:123mc\x1b[48:5:200:9md",
            b"\x1b[58:2::1:2:3me\x1b[48:2::1:2:3:4:5mf\x1b[38:2:1:2mg\x1b[38:2::1:2mh\x1b[0m\n",
            b"\x1b[1:2:3:4:5:6:7:8meight\x1b[1:2:3:4:5:6:7mseven\x1b[4:1:2mthree\x1b[4:9mnine",
            b"\x1b[38:5:mempty\x1b[38:2::::mempties\x1b[58:5:1;1;31mmixed\x1b[0m",
        ]),
    );
    add(
        "sgr-reset-link",
        cat(&[
            b"\x1b]8;;http://example.com\x07\x1b[31mlinked\x1b[0mstill\x1b[mgone\x1b]8;;\x07plain",
        ]),
    );

    // DCS.
    add(
        "dcs-embedded-esc",
        cat(&[b"a\x1bPqab\x1b\x1bcd\x1b\\b\x1bP1;2;3q\x07\x18\x1a\x00x\x1b\\c\x1bP:junk\x1b\\d\x1bPq\x1bXe\x1b\\f"]),
    );
    add(
        "dcs-decrqss",
        cat(&[b"a\x1bP$q q\x1b\\b\x1bP$qBAD\x1b\\c\x1bP$q qextra\x1b\\d\x1bP$$q q\x1b\\e"]),
    );
    add(
        "dcs-passthrough",
        cat(&[
            b"a\x1bPtmux;\x1b\x1b[31mred\x1b\\b\x1bPtmux;\x1b\x1b]2;pt\x07\x1b\\c\x1bPtmuxx\x1b\\d",
        ]),
    );

    // OSC.
    add(
        "osc-title",
        cat(&[
            b"\x1b]0;bel-title\x07a\x1b]2;st-title\x1b\\b\x1b]0;caf\xc3\xa9 \xe6\x97\xa5\x07c",
            b"\x1b]2;bad\xff\x07d\x1b]0;#(cmd) #{x} \\e\x07e\x1b]2;\x07f\x1b]0;tab\there\x07g",
        ]),
    );
    add(
        "osc-esc-terminated",
        cat(&[b"\x1b]0;t\x1b[Hx\x1b]2;u\x1b[2;2Hy\x1b]0;v\x1bDz"]),
    );
    add(
        "osc-7-path",
        cat(&[b"\x1b]7;file://host/tmp\x07a\x1b]7;\x07b\x1b]7;bad\xff\x07c"]),
    );
    add(
        "osc-8-hyperlinks",
        cat(&[
            b"\x1b]8;;http://example.com\x07link\x1b]8;;\x07 \x1b]8;id=a;http://example.com/a\x07ida",
            b"\x1b]8;id=a;http://example.com/a\x07same\x1b]8;id=b;http://example.com/a\x07idb\x1b]8;;\x07\n",
            b"\x1b]8;id=a:id=b;http://bad\x07X\x1b]8;id=no-separator\x07Y\x1b]8;foo=bar:id=c;http://c\x07Z",
            b"\x1b]8;id=;http://empty-id\x07E\x1b]8;;\x07\n",
            b"\x1b]8;;http://example.com/\x1b\\st-link\x1b]8;;\x1b\\plain\x1b]8\x07nosemi\n",
            b"\x1b]8;;http://",
            &repeat(b"long", 100),
            b"\x07L\x1b]8;;\x07",
        ]),
    );
    add(
        "osc-9-4-progress",
        cat(&[b"\x1b]9;4;1;50\x07a\x1b]9;4;2\x07b\x1b]9;4;0\x07c\x1b]9;4;5;200\x07d\x1b]9;4;z\x07e\x1b]9;hello\x07f"]),
    );
    add(
        "osc-colours",
        cat(&[
            b"\x1b]10;red\x07a\x1b]11;#102030\x07b\x1b]12;blue\x07c\x1b]10;notacolour\x07d",
            b"\x1b]4;1;red;2;#00ff00\x07e\x1b]4;999;red\x07f\x1b]104\x07g\x1b]104;1;2\x07h\x1b]104;999\x07i",
            b"\x1b]110\x07j\x1b]111\x07k\x1b]112\x07l\x1b]110;x\x07m\x1b]10;?\x07n\x1b]11;?\x07o\x1b]12;?\x07p",
            b"\x1b]4;1;?\x07q",
        ]),
    );
    add(
        "osc-133",
        cat(&[
            b"\x1b]133;A\x07$ \x1b]133;B\x07cmd\n\x1b]133;C\x07out1\nout2\n\x1b]133;D;7\x07",
            b"\x1b]133;A\x07$ \x1b]133;B\x07two\n\x1b]133;C\x07\x1b]133;D;-1\x07\x1b]133;D\x07",
            b"\x1b]133;P;k=s\x07sec\n\x1b]133;P;k=i\x07pri\n\x1b]133;N\x07n\x1b]133;I\x07i\n",
            b"\x1b]133;D;300\x07\x1b]133;Z\x07\x1b]133\x07end",
        ]),
    );
    add(
        "osc-unknown",
        cat(&[b"\x1b]999;bad\x07OK\x1b]abc\x07a\x1b]\x07b\x1b]2\x07c\x1b];x\x07d\x1b]52bad\x07e\x1b]52;c;@@@\x07f\x1b]4294967296;x\x07g"]),
    );
    add(
        "apc-title",
        cat(&[b"a\x1b_apc title\x1b\\b\x1b_bad\xff\x1b\\c\x1b_\x1b\\d\x1b_x\x1b[Hy"]),
    );
    add(
        "rename",
        cat(&[b"a\x1bkname\x1b\\b\x1bk\x1b\\c\x1bkx\x1b[2;2Hd"]),
    );
    add(
        "sos-pm",
        cat(&[b"a\x1bXsos junk\x1b[31m\x1b\\b\x1b^pm junk\x07\x1b\\c\x1bX\x1b[1;1Hd"]),
    );

    // Wrap and scrolling.
    add(
        "wrap-long",
        cat(&[
            &repeat(b"0123456789", 20),
            b"\n",
            &repeat(b"a", 79),
            "日本".as_bytes(),
            b"\n",
            &repeat(b"b", 80),
            "日".as_bytes(),
            b"\n",
            &repeat(b"c", 80),
            b"\x1b[1mX\n",
            b"\x1b[?7l",
            &repeat(b"d", 100),
            "語".as_bytes(),
            b"\x1b[?7h\n",
            &repeat(b"e", 80),
            b"\x08\x08zz\n",
            &repeat(b"f", 80),
            b"\x1b[Kafter-el",
        ]),
    );
    add(
        "scroll-region",
        cat(&[
            &repeat(b"L\n", 23),
            b"\x1b[5;10r\x1b[10;1Hbottom\n\n\x1bDdown\x1b[5;1H\x1bM\x1bMup\x1b[4;1Habove\n",
            b"\x1b[11;1Hbelow\n\x1b[r\x1b[24;1H\n\n\x1bDhist",
        ]),
    );
    add(
        "history",
        cat(&[
            &repeat(b"history line with some text 0123456789\n", 40),
            b"end",
        ]),
    );
    add(
        "history-wrapped",
        cat(&[
            &repeat(b"x", 2000),
            b"\n",
            &repeat(b"wrapped line ", 20),
            b"\x1b[2Jcleared",
        ]),
    );

    // UTF-8.
    add(
        "utf8-wide-combining",
        cat(&[
            "日本語テキスト 한국어 中文\n".as_bytes(),
            "e\u{301}a\u{308}\u{301}x\n".as_bytes(),
            "👨\u{200d}👩\u{200d}👧 ❤\u{fe0f} 👍🏽 🇯🇵\n".as_bytes(),
            b"\xe6\x97\x1b[1m\xa5bold\x1b[0m\n",
            b"\xe6\x97\rcr\n",
            b"\xe6\x97\x1b]2;t\x07\xa5\n",
            b"\xe6\x97\x7f\xa5\n",
            b"\xf0\x9f\x98\x1b[2C\x80\n",
            b"\xe6\x97\xa5\x1b[2Z\xe6\x97\xa5",
        ]),
    );
    add(
        "utf8-invalid",
        cat(&[
            b"\xf0\x80\x80\x80A\xed\xa0\x80B\n",
            b"\x80C\xbfD\xc3E\xf4\x90\x80\x80F\xf8\x88\x80\x80\x80G\xc0\xafH\n",
            b"\xe6\x97\xa5\xa5I\xff\xfeJ",
        ]),
    );
    add(
        "utf8-at-margin",
        cat(&[
            &repeat(b" ", 79),
            "日x\n".as_bytes(),
            &repeat(b" ", 78),
            "日x\n".as_bytes(),
            b"\x1b[?7l",
            &repeat(b" ", 79),
            "日x\x1b[?7h\n".as_bytes(),
            b"\x1b[4h",
            &repeat(b"a", 78),
            "\r日\x1b[4l".as_bytes(),
        ]),
    );

    // Mixed streams.
    add(
        "mixed-1",
        cat(&[
            b"\x1b[?1049h\x1b[H\x1b[2J\x1b[1;1H\x1b]2;vim\x07\x1b[?25l",
            &repeat(b"\x1b[1;34m~\x1b[0m\x1b[K\r\n", 20),
            b"\x1b[1;7m file.txt \x1b[0m\x1b[K\r\n\x1b[?25h\x1b[3;5H",
            "日本語 text é\x1b[?1049l".as_bytes(),
            b"\x1b]0;shell\x07$ \x1b]133;A\x07ls\n\x1b]133;C\x07a b c\n\x1b]133;D;0\x07$ ",
        ]),
    );
    add(
        "mixed-2",
        cat(&[
            &repeat(b"\x1b[31mred\x1b[32mgreen\x1b[0m \x1b[1mbold\x1b[22m \x1b[4:3mcurly\x1b[24m\n", 10),
            b"\x1b[5;20r\x1b[20;1H",
            &repeat(b"\x1b[38;5;208mscroll\x1b[0m\n", 10),
            b"\x1b[r\x1b7\x1b[1;1H\x1bPtmux;\x1b\x1b[1m\x1b\\\x1b8\x1b]8;;http://x\x07link\x1b]8;;\x07",
            b"\x1b[?7l",
            &repeat(b"z", 100),
            b"\x1b[?7h\x1b[6n\x1b[c\x1bP$q q\x1b\\\x1b[22;0t\x1b]2;pushed\x07\x1b[23;0t",
        ]),
    );
    add(
        "mixed-3",
        cat(&[
            b"\x1b#8\x1b[3;3H\x1b[2K\x1b[5;5H\x1b[1J\x1b[10;10H\x1b(0lqqk\x1b(B\x1b[?6h\x1b[8;16r\x1b[H",
            &repeat(b"in-region\n", 12),
            b"\x1b[?6l\x1b[r\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[4h\x1b[1;1Hinsert\x1b[4l",
            b"\x1b[?25l\x1b[?12h\x1b[=\x1b[34h\x1b]9;4;1;33\x07\x1b]10;red\x07\x1b]11;blue\x07",
            "\x1b[38:2::10:20:30m\x1b[58:5:9mμ\x1b[0m\x1b[s\x1b[20;1H\x1b[u".as_bytes(),
        ]),
    );

    add(
        "csi-aliases-theme-modoff",
        cat(&[b"\x1b[2;3falias\x1b[12`hpa\x1b[?2031h\x1b[?996n\x1b[?2031l\x1b[>4;2m\x1b[>4n\x1b[?1;1;0S"]),
    );
    add(
        "osc-52-default",
        cat(&[b"\x1b]52;c;aGk=\x07a\x1b]52;pc;?\x1b\\b\x1b]52;c;\x07c\x1b]52;c;@@@\x07d"]),
    );
    add(
        "dcs-prefix-states",
        cat(&[b"a\x1bP1;2$q q\x1b\\b\x1bP1?ignored\x1b\\c\x1bP$1ignored\x1b\\d\x1bP\x07\xff1;2qbody\x1b\\e"]),
    );

    // Random dictionary stream.
    let mut rng = common::Rng::new(0x9e37_79b9_7f4a_7c15);
    add(
        "random-dict",
        input_corpus::dictionary_stream(&mut rng, 6000),
    );
    v
}

/// Streams also fed to the emulator one byte per writer transaction.
const SPLIT_STREAMS: &[&str] = &["mixed-1", "utf8-wide-combining", "osc-8-hyperlinks"];

/// `capture` is `-e -N`; byte-split runs use `-T` because each writer
/// transaction rounds cell storage independently (`grid.c:311-316`).
#[derive(PartialEq)]
struct Dumps {
    capture: Vec<u8>,
    used: Vec<u8>,
    flags: Vec<u8>,
    state: String,
}

struct Oracle {
    tmux: PathBuf,
    socket: String,
}

impl Oracle {
    fn new(tmux: &Path, tag: &str) -> Oracle {
        let socket = std::env::temp_dir()
            .join(format!("rmux-g05-{}-{tag}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        Oracle {
            tmux: tmux.to_path_buf(),
            socket,
        }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.tmux)
            .args(["-f", "/dev/null", "-S", &self.socket])
            .args(args)
            .output()
            .expect("run oracle")
    }

    fn start(&self, command: &str) {
        let out = self.run(&[
            "new",
            "-d",
            "-x",
            &SX.to_string(),
            "-y",
            &SY.to_string(),
            command,
        ]);
        assert!(
            out.status.success(),
            "oracle start: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Start a pane that cats `file` into itself with echo and output
    /// processing off, signals `done`, and then idles.
    fn feed_file(&self, file: &Path) {
        let command = format!(
            "stty -echo -opost; cat '{}'; '{}' -S '{}' wait-for -S done; exec sleep 300",
            file.display(),
            self.tmux.display(),
            self.socket
        );
        self.start(&command);
        let out = self.run(&["wait-for", "done"]);
        assert!(out.status.success(), "wait-for done");
    }

    fn dumps(&self) -> Dumps {
        let capture = self
            .run(&["capture-pane", "-p", "-e", "-N", "-S", "-"])
            .stdout;
        let used = self
            .run(&["capture-pane", "-p", "-e", "-N", "-T", "-S", "-"])
            .stdout;
        let flags = self
            .run(&["capture-pane", "-p", "-F", "-N", "-T", "-S", "-"])
            .stdout;
        let state = self.run(&["display", "-p", STATE_FORMAT]).stdout;
        let mut state = String::from_utf8(state).expect("state line utf-8");
        if state.ends_with('\n') {
            state.pop();
        }
        Dumps {
            capture,
            used,
            flags,
            state,
        }
    }

    /// Bytes still in the pty buffer after `wait-for` may be unread; wait
    /// until two dumps 20 ms apart agree.
    fn settled_dumps(&self) -> Dumps {
        let mut last = self.dumps();
        for _ in 0..100 {
            std::thread::sleep(Duration::from_millis(20));
            let next = self.dumps();
            if next == last {
                return next;
            }
            last = next;
        }
        last
    }

    fn title(&self) -> Vec<u8> {
        let mut t = self.run(&["display", "-p", "#{pane_title}"]).stdout;
        if t.last() == Some(&b'\n') {
            t.pop();
        }
        t
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = Command::new(&self.tmux)
            .args(["-f", "/dev/null", "-S", &self.socket, "kill-server"])
            .output();
    }
}

fn emu_dumps(emu: &Emu) -> Dumps {
    Dumps {
        capture: emu.capture(),
        used: emu.capture_used(),
        flags: emu.flags(),
        state: emu.state(),
    }
}

fn escape(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|&b| std::ascii::escape_default(b))
        .map(char::from)
        .collect()
}

fn report_lines(what: &str, name: &str, expected: &[u8], got: &[u8]) -> bool {
    if expected == got {
        return true;
    }
    eprintln!("== {name}: {what} mismatch");
    let exp: Vec<&[u8]> = expected.split(|&b| b == b'\n').collect();
    let act: Vec<&[u8]> = got.split(|&b| b == b'\n').collect();
    if exp.len() != act.len() {
        eprintln!(
            "   oracle has {} lines, rmux has {} lines",
            exp.len(),
            act.len()
        );
    }
    for (i, (e, a)) in exp.iter().zip(act.iter()).enumerate() {
        if e != a {
            eprintln!(
                "   line {i}:\n     oracle: {}\n     rmux:   {}",
                escape(e),
                escape(a)
            );
        }
    }
    false
}

/// Compare the exact capture for whole streams and used cells for byte splits.
fn compare(name: &str, oracle: &Dumps, emu: &Dumps, split: bool) -> bool {
    let mut ok = report_lines("capture-pane -e -T", name, &oracle.used, &emu.used);
    ok &= report_lines("capture-pane -F -T", name, &oracle.flags, &emu.flags);
    if oracle.state != emu.state {
        eprintln!(
            "== {name}: state mismatch\n     oracle: {}\n     rmux:   {}",
            oracle.state, emu.state
        );
        ok = false;
    }
    if !split {
        ok &= report_lines(
            "capture-pane -e -N -S -",
            name,
            &oracle.capture,
            &emu.capture,
        );
    }
    ok
}

fn new_emu(title: &[u8]) -> Emu {
    let mut emu = Emu::new(SX, SY, HLIMIT);
    emu.screen.title = title.to_vec();
    emu
}

#[test]
fn streams_match_oracle() {
    let Some(tmux) = common::oracle() else {
        eprintln!("input oracle test skipped: oracle tmux missing");
        return;
    };
    let dir = std::env::temp_dir().join(format!("rmux-g05-streams-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // The default pane title is the host name (`window.c:1494-1495`).
    let title = {
        let o = Oracle::new(&tmux, "title");
        o.start("sleep 300");
        o.title()
    };

    let mut failures = Vec::new();
    let mut count = 0;
    for (name, bytes) in corpus() {
        let file = dir.join(format!("{name}.bin"));
        std::fs::write(&file, &bytes).unwrap();
        let oracle = Oracle::new(&tmux, name);
        oracle.feed_file(&file);
        let expected = oracle.settled_dumps();
        drop(oracle);

        let mut emu = new_emu(&title);
        emu.feed(&bytes);
        count += 1;
        if !compare(name, &expected, &emu_dumps(&emu), false) {
            failures.push(name.to_string());
        }

        if SPLIT_STREAMS.contains(&name) {
            let mut emu = new_emu(&title);
            for b in &bytes {
                emu.feed(std::slice::from_ref(b));
            }
            count += 1;
            let split = format!("{name} (byte-split)");
            if !compare(&split, &expected, &emu_dumps(&emu), true) {
                failures.push(split);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("{count} comparisons, {} failed", failures.len());
    assert!(failures.is_empty(), "mismatching streams: {failures:?}");
}
