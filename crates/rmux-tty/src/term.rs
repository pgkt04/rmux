// Ported from tmux tmux.h, tty-term.c @ 8f25579c
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

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TtyCodeCode {
    Acsc = 0,
    Am = 1,
    Ax = 2,
    Bce = 3,
    Bel = 4,
    Bidi = 5,
    Blink = 6,
    Bold = 7,
    Civis = 8,
    Clear = 9,
    Clmg = 10,
    Cmg = 11,
    Cnorm = 12,
    Colors = 13,
    Cr = 14,
    Cs = 15,
    Csr = 16,
    Cub = 17,
    Cub1 = 18,
    Cud = 19,
    Cud1 = 20,
    Cuf = 21,
    Cuf1 = 22,
    Cup = 23,
    Cuu = 24,
    Cuu1 = 25,
    Cvvis = 26,
    Dch = 27,
    Dch1 = 28,
    Dim = 29,
    Dl = 30,
    Dl1 = 31,
    Dsbp = 32,
    Dseks = 33,
    Dsesc = 34,
    Dsfcs = 35,
    Dsmg = 36,
    E3 = 37,
    Ech = 38,
    Ed = 39,
    El = 40,
    El1 = 41,
    Enacs = 42,
    Enbp = 43,
    Eneks = 44,
    Enesc = 45,
    Enfcs = 46,
    Enmg = 47,
    Fsl = 48,
    Hls = 49,
    Home = 50,
    Hpa = 51,
    Ich = 52,
    Ich1 = 53,
    Il = 54,
    Il1 = 55,
    Ind = 56,
    Indn = 57,
    Invis = 58,
    Kcbt = 59,
    Kcub1 = 60,
    Kcud1 = 61,
    Kcuf1 = 62,
    Kcuu1 = 63,
    Kdc2 = 64,
    Kdc3 = 65,
    Kdc4 = 66,
    Kdc5 = 67,
    Kdc6 = 68,
    Kdc7 = 69,
    Kdch1 = 70,
    Kdn2 = 71,
    Kdn3 = 72,
    Kdn4 = 73,
    Kdn5 = 74,
    Kdn6 = 75,
    Kdn7 = 76,
    Kend = 77,
    Kend2 = 78,
    Kend3 = 79,
    Kend4 = 80,
    Kend5 = 81,
    Kend6 = 82,
    Kend7 = 83,
    Kf1 = 84,
    Kf10 = 85,
    Kf11 = 86,
    Kf12 = 87,
    Kf13 = 88,
    Kf14 = 89,
    Kf15 = 90,
    Kf16 = 91,
    Kf17 = 92,
    Kf18 = 93,
    Kf19 = 94,
    Kf2 = 95,
    Kf20 = 96,
    Kf21 = 97,
    Kf22 = 98,
    Kf23 = 99,
    Kf24 = 100,
    Kf25 = 101,
    Kf26 = 102,
    Kf27 = 103,
    Kf28 = 104,
    Kf29 = 105,
    Kf3 = 106,
    Kf30 = 107,
    Kf31 = 108,
    Kf32 = 109,
    Kf33 = 110,
    Kf34 = 111,
    Kf35 = 112,
    Kf36 = 113,
    Kf37 = 114,
    Kf38 = 115,
    Kf39 = 116,
    Kf4 = 117,
    Kf40 = 118,
    Kf41 = 119,
    Kf42 = 120,
    Kf43 = 121,
    Kf44 = 122,
    Kf45 = 123,
    Kf46 = 124,
    Kf47 = 125,
    Kf48 = 126,
    Kf49 = 127,
    Kf5 = 128,
    Kf50 = 129,
    Kf51 = 130,
    Kf52 = 131,
    Kf53 = 132,
    Kf54 = 133,
    Kf55 = 134,
    Kf56 = 135,
    Kf57 = 136,
    Kf58 = 137,
    Kf59 = 138,
    Kf6 = 139,
    Kf60 = 140,
    Kf61 = 141,
    Kf62 = 142,
    Kf63 = 143,
    Kf7 = 144,
    Kf8 = 145,
    Kf9 = 146,
    Khom2 = 147,
    Khom3 = 148,
    Khom4 = 149,
    Khom5 = 150,
    Khom6 = 151,
    Khom7 = 152,
    Khome = 153,
    Kic2 = 154,
    Kic3 = 155,
    Kic4 = 156,
    Kic5 = 157,
    Kic6 = 158,
    Kic7 = 159,
    Kich1 = 160,
    Kind = 161,
    Klft2 = 162,
    Klft3 = 163,
    Klft4 = 164,
    Klft5 = 165,
    Klft6 = 166,
    Klft7 = 167,
    Kmous = 168,
    Knp = 169,
    Knxt2 = 170,
    Knxt3 = 171,
    Knxt4 = 172,
    Knxt5 = 173,
    Knxt6 = 174,
    Knxt7 = 175,
    Kpp = 176,
    Kprv2 = 177,
    Kprv3 = 178,
    Kprv4 = 179,
    Kprv5 = 180,
    Kprv6 = 181,
    Kprv7 = 182,
    Kri = 183,
    Krit2 = 184,
    Krit3 = 185,
    Krit4 = 186,
    Krit5 = 187,
    Krit6 = 188,
    Krit7 = 189,
    Kup2 = 190,
    Kup3 = 191,
    Kup4 = 192,
    Kup5 = 193,
    Kup6 = 194,
    Kup7 = 195,
    Ms = 196,
    Nobr = 197,
    Ol = 198,
    Op = 199,
    Rect = 200,
    Rev = 201,
    Rgb = 202,
    Ri = 203,
    Rin = 204,
    Rmacs = 205,
    Rmcup = 206,
    Rmkx = 207,
    Se = 208,
    Setab = 209,
    Setaf = 210,
    Setal = 211,
    Setrgbb = 212,
    Setrgbf = 213,
    Setulc = 214,
    Setulc1 = 215,
    Sgr0 = 216,
    Sitm = 217,
    Smacs = 218,
    Smcup = 219,
    Smkx = 220,
    Smol = 221,
    Smso = 222,
    Smul = 223,
    Smulx = 224,
    Smxx = 225,
    Spb = 226,
    Sxl = 227,
    Ss = 228,
    Swd = 229,
    Sync = 230,
    Tc = 231,
    Tsl = 232,
    U8 = 233,
    Vpa = 234,
    Xt = 235,
}
impl TryFrom<i32> for TtyCodeCode {
    type Error = i32;
    fn try_from(value: i32) -> Result<Self, i32> {
        match value {
            0 => Ok(Self::Acsc),
            1 => Ok(Self::Am),
            2 => Ok(Self::Ax),
            3 => Ok(Self::Bce),
            4 => Ok(Self::Bel),
            5 => Ok(Self::Bidi),
            6 => Ok(Self::Blink),
            7 => Ok(Self::Bold),
            8 => Ok(Self::Civis),
            9 => Ok(Self::Clear),
            10 => Ok(Self::Clmg),
            11 => Ok(Self::Cmg),
            12 => Ok(Self::Cnorm),
            13 => Ok(Self::Colors),
            14 => Ok(Self::Cr),
            15 => Ok(Self::Cs),
            16 => Ok(Self::Csr),
            17 => Ok(Self::Cub),
            18 => Ok(Self::Cub1),
            19 => Ok(Self::Cud),
            20 => Ok(Self::Cud1),
            21 => Ok(Self::Cuf),
            22 => Ok(Self::Cuf1),
            23 => Ok(Self::Cup),
            24 => Ok(Self::Cuu),
            25 => Ok(Self::Cuu1),
            26 => Ok(Self::Cvvis),
            27 => Ok(Self::Dch),
            28 => Ok(Self::Dch1),
            29 => Ok(Self::Dim),
            30 => Ok(Self::Dl),
            31 => Ok(Self::Dl1),
            32 => Ok(Self::Dsbp),
            33 => Ok(Self::Dseks),
            34 => Ok(Self::Dsesc),
            35 => Ok(Self::Dsfcs),
            36 => Ok(Self::Dsmg),
            37 => Ok(Self::E3),
            38 => Ok(Self::Ech),
            39 => Ok(Self::Ed),
            40 => Ok(Self::El),
            41 => Ok(Self::El1),
            42 => Ok(Self::Enacs),
            43 => Ok(Self::Enbp),
            44 => Ok(Self::Eneks),
            45 => Ok(Self::Enesc),
            46 => Ok(Self::Enfcs),
            47 => Ok(Self::Enmg),
            48 => Ok(Self::Fsl),
            49 => Ok(Self::Hls),
            50 => Ok(Self::Home),
            51 => Ok(Self::Hpa),
            52 => Ok(Self::Ich),
            53 => Ok(Self::Ich1),
            54 => Ok(Self::Il),
            55 => Ok(Self::Il1),
            56 => Ok(Self::Ind),
            57 => Ok(Self::Indn),
            58 => Ok(Self::Invis),
            59 => Ok(Self::Kcbt),
            60 => Ok(Self::Kcub1),
            61 => Ok(Self::Kcud1),
            62 => Ok(Self::Kcuf1),
            63 => Ok(Self::Kcuu1),
            64 => Ok(Self::Kdc2),
            65 => Ok(Self::Kdc3),
            66 => Ok(Self::Kdc4),
            67 => Ok(Self::Kdc5),
            68 => Ok(Self::Kdc6),
            69 => Ok(Self::Kdc7),
            70 => Ok(Self::Kdch1),
            71 => Ok(Self::Kdn2),
            72 => Ok(Self::Kdn3),
            73 => Ok(Self::Kdn4),
            74 => Ok(Self::Kdn5),
            75 => Ok(Self::Kdn6),
            76 => Ok(Self::Kdn7),
            77 => Ok(Self::Kend),
            78 => Ok(Self::Kend2),
            79 => Ok(Self::Kend3),
            80 => Ok(Self::Kend4),
            81 => Ok(Self::Kend5),
            82 => Ok(Self::Kend6),
            83 => Ok(Self::Kend7),
            84 => Ok(Self::Kf1),
            85 => Ok(Self::Kf10),
            86 => Ok(Self::Kf11),
            87 => Ok(Self::Kf12),
            88 => Ok(Self::Kf13),
            89 => Ok(Self::Kf14),
            90 => Ok(Self::Kf15),
            91 => Ok(Self::Kf16),
            92 => Ok(Self::Kf17),
            93 => Ok(Self::Kf18),
            94 => Ok(Self::Kf19),
            95 => Ok(Self::Kf2),
            96 => Ok(Self::Kf20),
            97 => Ok(Self::Kf21),
            98 => Ok(Self::Kf22),
            99 => Ok(Self::Kf23),
            100 => Ok(Self::Kf24),
            101 => Ok(Self::Kf25),
            102 => Ok(Self::Kf26),
            103 => Ok(Self::Kf27),
            104 => Ok(Self::Kf28),
            105 => Ok(Self::Kf29),
            106 => Ok(Self::Kf3),
            107 => Ok(Self::Kf30),
            108 => Ok(Self::Kf31),
            109 => Ok(Self::Kf32),
            110 => Ok(Self::Kf33),
            111 => Ok(Self::Kf34),
            112 => Ok(Self::Kf35),
            113 => Ok(Self::Kf36),
            114 => Ok(Self::Kf37),
            115 => Ok(Self::Kf38),
            116 => Ok(Self::Kf39),
            117 => Ok(Self::Kf4),
            118 => Ok(Self::Kf40),
            119 => Ok(Self::Kf41),
            120 => Ok(Self::Kf42),
            121 => Ok(Self::Kf43),
            122 => Ok(Self::Kf44),
            123 => Ok(Self::Kf45),
            124 => Ok(Self::Kf46),
            125 => Ok(Self::Kf47),
            126 => Ok(Self::Kf48),
            127 => Ok(Self::Kf49),
            128 => Ok(Self::Kf5),
            129 => Ok(Self::Kf50),
            130 => Ok(Self::Kf51),
            131 => Ok(Self::Kf52),
            132 => Ok(Self::Kf53),
            133 => Ok(Self::Kf54),
            134 => Ok(Self::Kf55),
            135 => Ok(Self::Kf56),
            136 => Ok(Self::Kf57),
            137 => Ok(Self::Kf58),
            138 => Ok(Self::Kf59),
            139 => Ok(Self::Kf6),
            140 => Ok(Self::Kf60),
            141 => Ok(Self::Kf61),
            142 => Ok(Self::Kf62),
            143 => Ok(Self::Kf63),
            144 => Ok(Self::Kf7),
            145 => Ok(Self::Kf8),
            146 => Ok(Self::Kf9),
            147 => Ok(Self::Khom2),
            148 => Ok(Self::Khom3),
            149 => Ok(Self::Khom4),
            150 => Ok(Self::Khom5),
            151 => Ok(Self::Khom6),
            152 => Ok(Self::Khom7),
            153 => Ok(Self::Khome),
            154 => Ok(Self::Kic2),
            155 => Ok(Self::Kic3),
            156 => Ok(Self::Kic4),
            157 => Ok(Self::Kic5),
            158 => Ok(Self::Kic6),
            159 => Ok(Self::Kic7),
            160 => Ok(Self::Kich1),
            161 => Ok(Self::Kind),
            162 => Ok(Self::Klft2),
            163 => Ok(Self::Klft3),
            164 => Ok(Self::Klft4),
            165 => Ok(Self::Klft5),
            166 => Ok(Self::Klft6),
            167 => Ok(Self::Klft7),
            168 => Ok(Self::Kmous),
            169 => Ok(Self::Knp),
            170 => Ok(Self::Knxt2),
            171 => Ok(Self::Knxt3),
            172 => Ok(Self::Knxt4),
            173 => Ok(Self::Knxt5),
            174 => Ok(Self::Knxt6),
            175 => Ok(Self::Knxt7),
            176 => Ok(Self::Kpp),
            177 => Ok(Self::Kprv2),
            178 => Ok(Self::Kprv3),
            179 => Ok(Self::Kprv4),
            180 => Ok(Self::Kprv5),
            181 => Ok(Self::Kprv6),
            182 => Ok(Self::Kprv7),
            183 => Ok(Self::Kri),
            184 => Ok(Self::Krit2),
            185 => Ok(Self::Krit3),
            186 => Ok(Self::Krit4),
            187 => Ok(Self::Krit5),
            188 => Ok(Self::Krit6),
            189 => Ok(Self::Krit7),
            190 => Ok(Self::Kup2),
            191 => Ok(Self::Kup3),
            192 => Ok(Self::Kup4),
            193 => Ok(Self::Kup5),
            194 => Ok(Self::Kup6),
            195 => Ok(Self::Kup7),
            196 => Ok(Self::Ms),
            197 => Ok(Self::Nobr),
            198 => Ok(Self::Ol),
            199 => Ok(Self::Op),
            200 => Ok(Self::Rect),
            201 => Ok(Self::Rev),
            202 => Ok(Self::Rgb),
            203 => Ok(Self::Ri),
            204 => Ok(Self::Rin),
            205 => Ok(Self::Rmacs),
            206 => Ok(Self::Rmcup),
            207 => Ok(Self::Rmkx),
            208 => Ok(Self::Se),
            209 => Ok(Self::Setab),
            210 => Ok(Self::Setaf),
            211 => Ok(Self::Setal),
            212 => Ok(Self::Setrgbb),
            213 => Ok(Self::Setrgbf),
            214 => Ok(Self::Setulc),
            215 => Ok(Self::Setulc1),
            216 => Ok(Self::Sgr0),
            217 => Ok(Self::Sitm),
            218 => Ok(Self::Smacs),
            219 => Ok(Self::Smcup),
            220 => Ok(Self::Smkx),
            221 => Ok(Self::Smol),
            222 => Ok(Self::Smso),
            223 => Ok(Self::Smul),
            224 => Ok(Self::Smulx),
            225 => Ok(Self::Smxx),
            226 => Ok(Self::Spb),
            227 => Ok(Self::Sxl),
            228 => Ok(Self::Ss),
            229 => Ok(Self::Swd),
            230 => Ok(Self::Sync),
            231 => Ok(Self::Tc),
            232 => Ok(Self::Tsl),
            233 => Ok(Self::U8),
            234 => Ok(Self::Vpa),
            235 => Ok(Self::Xt),
            _ => Err(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TtyTermFlags(pub u32);
impl TtyTermFlags {
    pub const _256COLOURS: Self = Self(1);
    pub const NOAM: Self = Self(2);
    pub const DECSLRM: Self = Self(4);
    pub const DECFRA: Self = Self(8);
    pub const RGBCOLOURS: Self = Self(16);
    pub const VT100LIKE: Self = Self(32);
    pub const SIXEL: Self = Self(64);
    pub const INVALIDMS: Self = Self(128);
    pub const NOREPLACE: Self = Self(256);
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }
}
impl std::ops::BitOr for TtyTermFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl std::ops::BitAnd for TtyTermFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl std::ops::Not for TtyTermFlags {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

pub mod terminfo;
pub mod tparm;

use crate::features::{self, FEATURES};
use crate::tty::{TtyHostInfo, TtyOptions};
use rmux_util::bytes::ByteString;
use std::sync::atomic::{AtomicU64, Ordering};
use terminfo::CapList;
use tparm::{TparmArg, TparmState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodeKind {
    String,
    Number,
    Flag,
}
#[derive(Clone, Copy, Debug)]
pub struct CodeEntry {
    pub code: TtyCodeCode,
    pub name: &'static str,
    pub kind: CodeKind,
}
pub const CODES: [CodeEntry; 236] = [
    CodeEntry {
        code: TtyCodeCode::Acsc,
        name: "acsc",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Am,
        name: "am",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Ax,
        name: "AX",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Bce,
        name: "bce",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Bel,
        name: "bel",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Bidi,
        name: "Bidi",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Blink,
        name: "blink",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Bold,
        name: "bold",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Civis,
        name: "civis",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Clear,
        name: "clear",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Clmg,
        name: "Clmg",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cmg,
        name: "Cmg",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cnorm,
        name: "cnorm",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Colors,
        name: "colors",
        kind: CodeKind::Number,
    },
    CodeEntry {
        code: TtyCodeCode::Cr,
        name: "Cr",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cs,
        name: "Cs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Csr,
        name: "csr",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cub,
        name: "cub",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cub1,
        name: "cub1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cud,
        name: "cud",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cud1,
        name: "cud1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cuf,
        name: "cuf",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cuf1,
        name: "cuf1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cup,
        name: "cup",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cuu,
        name: "cuu",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cuu1,
        name: "cuu1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Cvvis,
        name: "cvvis",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dch,
        name: "dch",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dch1,
        name: "dch1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dim,
        name: "dim",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dl,
        name: "dl",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dl1,
        name: "dl1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dsbp,
        name: "Dsbp",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dseks,
        name: "Dseks",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dsesc,
        name: "Dsesc",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dsfcs,
        name: "Dsfcs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Dsmg,
        name: "Dsmg",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::E3,
        name: "E3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ech,
        name: "ech",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ed,
        name: "ed",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::El,
        name: "el",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::El1,
        name: "el1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Enacs,
        name: "enacs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Enbp,
        name: "Enbp",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Eneks,
        name: "Eneks",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Enesc,
        name: "Enesc",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Enfcs,
        name: "Enfcs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Enmg,
        name: "Enmg",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Fsl,
        name: "fsl",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Hls,
        name: "Hls",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Home,
        name: "home",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Hpa,
        name: "hpa",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ich,
        name: "ich",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ich1,
        name: "ich1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Il,
        name: "il",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Il1,
        name: "il1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ind,
        name: "ind",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Indn,
        name: "indn",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Invis,
        name: "invis",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kcbt,
        name: "kcbt",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kcub1,
        name: "kcub1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kcud1,
        name: "kcud1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kcuf1,
        name: "kcuf1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kcuu1,
        name: "kcuu1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc2,
        name: "kDC",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc3,
        name: "kDC3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc4,
        name: "kDC4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc5,
        name: "kDC5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc6,
        name: "kDC6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdc7,
        name: "kDC7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdch1,
        name: "kdch1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn2,
        name: "kDN",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn3,
        name: "kDN3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn4,
        name: "kDN4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn5,
        name: "kDN5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn6,
        name: "kDN6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kdn7,
        name: "kDN7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend,
        name: "kend",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend2,
        name: "kEND",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend3,
        name: "kEND3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend4,
        name: "kEND4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend5,
        name: "kEND5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend6,
        name: "kEND6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kend7,
        name: "kEND7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf1,
        name: "kf1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf10,
        name: "kf10",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf11,
        name: "kf11",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf12,
        name: "kf12",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf13,
        name: "kf13",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf14,
        name: "kf14",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf15,
        name: "kf15",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf16,
        name: "kf16",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf17,
        name: "kf17",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf18,
        name: "kf18",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf19,
        name: "kf19",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf2,
        name: "kf2",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf20,
        name: "kf20",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf21,
        name: "kf21",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf22,
        name: "kf22",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf23,
        name: "kf23",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf24,
        name: "kf24",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf25,
        name: "kf25",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf26,
        name: "kf26",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf27,
        name: "kf27",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf28,
        name: "kf28",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf29,
        name: "kf29",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf3,
        name: "kf3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf30,
        name: "kf30",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf31,
        name: "kf31",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf32,
        name: "kf32",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf33,
        name: "kf33",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf34,
        name: "kf34",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf35,
        name: "kf35",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf36,
        name: "kf36",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf37,
        name: "kf37",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf38,
        name: "kf38",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf39,
        name: "kf39",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf4,
        name: "kf4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf40,
        name: "kf40",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf41,
        name: "kf41",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf42,
        name: "kf42",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf43,
        name: "kf43",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf44,
        name: "kf44",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf45,
        name: "kf45",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf46,
        name: "kf46",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf47,
        name: "kf47",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf48,
        name: "kf48",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf49,
        name: "kf49",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf5,
        name: "kf5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf50,
        name: "kf50",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf51,
        name: "kf51",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf52,
        name: "kf52",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf53,
        name: "kf53",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf54,
        name: "kf54",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf55,
        name: "kf55",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf56,
        name: "kf56",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf57,
        name: "kf57",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf58,
        name: "kf58",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf59,
        name: "kf59",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf6,
        name: "kf6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf60,
        name: "kf60",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf61,
        name: "kf61",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf62,
        name: "kf62",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf63,
        name: "kf63",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf7,
        name: "kf7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf8,
        name: "kf8",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kf9,
        name: "kf9",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom2,
        name: "kHOM",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom3,
        name: "kHOM3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom4,
        name: "kHOM4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom5,
        name: "kHOM5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom6,
        name: "kHOM6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khom7,
        name: "kHOM7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Khome,
        name: "khome",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic2,
        name: "kIC",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic3,
        name: "kIC3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic4,
        name: "kIC4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic5,
        name: "kIC5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic6,
        name: "kIC6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kic7,
        name: "kIC7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kich1,
        name: "kich1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kind,
        name: "kind",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft2,
        name: "kLFT",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft3,
        name: "kLFT3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft4,
        name: "kLFT4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft5,
        name: "kLFT5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft6,
        name: "kLFT6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Klft7,
        name: "kLFT7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kmous,
        name: "kmous",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knp,
        name: "knp",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt2,
        name: "kNXT",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt3,
        name: "kNXT3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt4,
        name: "kNXT4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt5,
        name: "kNXT5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt6,
        name: "kNXT6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Knxt7,
        name: "kNXT7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kpp,
        name: "kpp",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv2,
        name: "kPRV",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv3,
        name: "kPRV3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv4,
        name: "kPRV4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv5,
        name: "kPRV5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv6,
        name: "kPRV6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kprv7,
        name: "kPRV7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kri,
        name: "kri",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit2,
        name: "kRIT",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit3,
        name: "kRIT3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit4,
        name: "kRIT4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit5,
        name: "kRIT5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit6,
        name: "kRIT6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Krit7,
        name: "kRIT7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup2,
        name: "kUP",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup3,
        name: "kUP3",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup4,
        name: "kUP4",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup5,
        name: "kUP5",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup6,
        name: "kUP6",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Kup7,
        name: "kUP7",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ms,
        name: "Ms",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Nobr,
        name: "Nobr",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Ol,
        name: "ol",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Op,
        name: "op",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rect,
        name: "Rect",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rev,
        name: "rev",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rgb,
        name: "RGB",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Ri,
        name: "ri",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rin,
        name: "rin",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rmacs,
        name: "rmacs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rmcup,
        name: "rmcup",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Rmkx,
        name: "rmkx",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Se,
        name: "Se",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setab,
        name: "setab",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setaf,
        name: "setaf",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setal,
        name: "setal",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setrgbb,
        name: "setrgbb",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setrgbf,
        name: "setrgbf",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setulc,
        name: "Setulc",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Setulc1,
        name: "Setulc1",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Sgr0,
        name: "sgr0",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Sitm,
        name: "sitm",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smacs,
        name: "smacs",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smcup,
        name: "smcup",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smkx,
        name: "smkx",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smol,
        name: "Smol",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smso,
        name: "smso",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smul,
        name: "smul",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smulx,
        name: "Smulx",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Smxx,
        name: "smxx",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Spb,
        name: "Spb",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Sxl,
        name: "Sxl",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Ss,
        name: "Ss",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Swd,
        name: "Swd",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Sync,
        name: "Sync",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Tc,
        name: "Tc",
        kind: CodeKind::Flag,
    },
    CodeEntry {
        code: TtyCodeCode::Tsl,
        name: "tsl",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::U8,
        name: "U8",
        kind: CodeKind::Number,
    },
    CodeEntry {
        code: TtyCodeCode::Vpa,
        name: "vpa",
        kind: CodeKind::String,
    },
    CodeEntry {
        code: TtyCodeCode::Xt,
        name: "XT",
        kind: CodeKind::Flag,
    },
];
impl TtyCodeCode {
    pub const COUNT: usize = CODES.len();
}
pub const fn ncodes() -> usize {
    CODES.len()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TtyCode {
    #[default]
    None,
    String(Vec<u8>),
    Number(i32),
    Flag(bool),
}
#[derive(Debug)]
pub struct TtyTerm {
    name: ByteString,
    codes: Box<[TtyCode; 236]>,
    pub(crate) acs: [[u8; 2]; 256],
    flags: TtyTermFlags,
    applied_features: u32,
    open_serial: u64,
}
static OPEN_SERIAL: AtomicU64 = AtomicU64::new(0);

fn cstr(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}
fn strip(s: &[u8]) -> Vec<u8> {
    let s = cstr(s);
    if !s.contains(&b'$') {
        return s.to_vec();
    }
    let mut out = Vec::with_capacity(s.len().min(8191));
    let mut at = 0;
    while at < s.len() {
        if s[at..].starts_with(b"$<") {
            while at < s.len() && s[at] != b'>' {
                at += 1;
            }
            if at < s.len() {
                at += 1;
            }
            if at == s.len() {
                break;
            }
        }
        out.push(s[at]);
        if out.len() == 8191 {
            break;
        }
        at += 1;
    }
    out
}
fn override_next(s: &[u8], at: &mut usize) -> Option<Vec<u8>> {
    let s = cstr(s);
    if *at >= s.len() {
        return None;
    }
    let mut value = Vec::new();
    while *at < s.len() {
        if s[*at] == b':' {
            if s.get(*at + 1) == Some(&b':') {
                value.push(b':');
                *at += 2;
            } else {
                break;
            }
        } else {
            value.push(s[*at]);
            *at += 1;
        }
        if value.len() == 8191 {
            return None;
        }
    }
    if *at < s.len() {
        *at += 1;
    }
    Some(value)
}
impl TtyTerm {
    pub fn create(
        state: &mut TparmState,
        name: &[u8],
        caps: &CapList,
        host: &mut TtyHostInfo,
        opts: &TtyOptions,
        colorterm: Option<&[u8]>,
    ) -> Result<Self, ByteString> {
        let mut term = Self {
            name: ByteString(cstr(name).to_vec()),
            codes: Box::new(std::array::from_fn(|_| TtyCode::None)),
            acs: [[0; 2]; 256],
            flags: TtyTermFlags(0),
            applied_features: 0,
            open_serial: OPEN_SERIAL.fetch_add(1, Ordering::Relaxed) + 1,
        };
        for cap in caps {
            let cap = cstr(cap.as_ref());
            let Some(eq) = cap.iter().position(|&b| b == b'=') else {
                continue;
            };
            let Some(index) = CODES
                .iter()
                .position(|ent| ent.name.as_bytes() == &cap[..eq])
            else {
                continue;
            };
            let value = &cap[eq + 1..];
            term.codes[index] = match CODES[index].kind {
                CodeKind::String => TtyCode::String(strip(value)),
                CodeKind::Number => rmux_util::strtonum::strtonum(value, 0, i32::MAX as i64)
                    .map_or(TtyCode::None, |n| TtyCode::Number(n as i32)),
                CodeKind::Flag => TtyCode::Flag(value.first() == Some(&b'1')),
            };
        }
        for entry in &opts.terminal_features {
            let entry = entry.as_ref();
            let mut at = 0;
            if let Some(pattern) = override_next(entry, &mut at) {
                if rmux_sys::fnmatch::fnmatch(
                    &pattern,
                    term.name(),
                    rmux_sys::fnmatch::FnmatchFlags::NONE,
                ) {
                    features::parse_features_bytes(&entry[at..], b":", &mut host.features);
                }
            }
        }
        if let Some(colorterm) = colorterm.map(cstr) {
            if colorterm.eq_ignore_ascii_case(b"truecolor")
                || colorterm.eq_ignore_ascii_case(b"24bit")
            {
                features::parse_features("RGB", ",", &mut host.features);
            } else if colorterm.windows(3).any(|s| s == b"256") {
                features::parse_features("256", ",", &mut host.features);
            }
        }
        term.apply_overrides(state, &opts.terminal_overrides);
        if !term.has(TtyCodeCode::Clear) {
            return Err(ByteString(b"terminal does not support clear".to_vec()));
        }
        if !term.has(TtyCodeCode::Cup) {
            return Err(ByteString(b"terminal does not support cup".to_vec()));
        }
        if term.flag(TtyCodeCode::Xt) || term.string(TtyCodeCode::Clear).starts_with(b"\x1b[") {
            term.flags.insert(TtyTermFlags::VT100LIKE);
            features::parse_features("bpaste,focus,title", ",", &mut host.features);
        }
        if (term.flag(TtyCodeCode::Tc) || term.has(TtyCodeCode::Rgb))
            && (!term.has(TtyCodeCode::Setrgbf) || !term.has(TtyCodeCode::Setrgbb))
        {
            features::parse_features("RGB", ",", &mut host.features);
        }
        if term.apply_features(host) {
            term.apply_overrides(state, &opts.terminal_overrides);
        }
        Ok(term)
    }
    pub fn apply(&mut self, caps: &[u8], _quiet: bool, flags: TtyTermFlags) {
        let mut at = 0;
        while let Some(entry) = override_next(caps, &mut at) {
            if entry.is_empty() {
                continue;
            }
            let eq = entry.iter().position(|&b| b == b'=');
            let remove = eq.is_none() && entry.last() == Some(&b'@');
            let end = eq.unwrap_or(entry.len() - usize::from(remove));
            let Some(index) = CODES
                .iter()
                .position(|ent| ent.name.as_bytes() == &entry[..end])
            else {
                continue;
            };
            if flags.contains(TtyTermFlags::NOREPLACE)
                && !matches!(self.codes[index], TtyCode::None)
            {
                continue;
            }
            if remove {
                self.codes[index] = TtyCode::None;
                continue;
            }
            let raw = eq.map_or(&b""[..], |eq| &entry[eq + 1..]);
            let value = rmux_util::vis::strunvis(raw).unwrap_or_else(|| ByteString(raw.to_vec()));
            match CODES[index].kind {
                CodeKind::String => {
                    self.codes[index] = TtyCode::String(cstr(value.as_ref()).to_vec())
                }
                CodeKind::Number => {
                    if let Ok(n) = rmux_util::strtonum::strtonum(value.as_ref(), 0, i32::MAX as i64)
                    {
                        self.codes[index] = TtyCode::Number(n as i32);
                    }
                }
                CodeKind::Flag => self.codes[index] = TtyCode::Flag(true),
            }
        }
    }
    pub fn apply_overrides(&mut self, state: &mut TparmState, overrides: &[ByteString]) {
        for entry in overrides {
            let mut at = 0;
            if let Some(pattern) = override_next(entry.as_ref(), &mut at) {
                if rmux_sys::fnmatch::fnmatch(
                    &pattern,
                    self.name(),
                    rmux_sys::fnmatch::FnmatchFlags::NONE,
                ) {
                    self.apply(&entry.as_ref()[at..], false, TtyTermFlags(0));
                }
            }
        }
        for (flag, present) in [
            (
                TtyTermFlags::RGBCOLOURS,
                self.has(TtyCodeCode::Setrgbf) && self.has(TtyCodeCode::Setrgbb),
            ),
            (
                TtyTermFlags::DECSLRM,
                self.has(TtyCodeCode::Cmg) && self.has(TtyCodeCode::Clmg),
            ),
            (TtyTermFlags::DECFRA, self.has(TtyCodeCode::Rect)),
            (TtyTermFlags::NOAM, !self.flag(TtyCodeCode::Am)),
        ] {
            if present {
                self.flags.insert(flag);
            } else {
                self.flags.remove(flag);
            }
        }
        let mut acs = [[0; 2]; 256];
        let string = if self.has(TtyCodeCode::Acsc) {
            self.string(TtyCodeCode::Acsc)
        } else {
            b"a#j+k+l+m+n+o-p-q-r-s-t+u+v+w+x|y<z>~."
        };
        for pair in string.chunks_exact(2) {
            acs[pair[0] as usize][0] = pair[1];
        }
        self.acs = acs;
        if self.has(TtyCodeCode::Ms) {
            let mut out = Vec::new();
            self.string_ss(state, TtyCodeCode::Ms, b"c", b"?", &mut out);
            if out.is_empty() {
                self.flags.insert(TtyTermFlags::INVALIDMS);
                self.codes[TtyCodeCode::Ms as usize] = TtyCode::None;
            } else {
                self.flags.remove(TtyTermFlags::INVALIDMS);
            }
        }
    }
    pub fn has(&self, code: TtyCodeCode) -> bool {
        !matches!(self.codes[code as usize], TtyCode::None)
    }
    pub fn has_name(&self, name: &str) -> bool {
        CODES
            .iter()
            .find(|ent| ent.name == name)
            .is_some_and(|ent| self.has(ent.code))
    }
    pub fn string(&self, code: TtyCodeCode) -> &[u8] {
        match &self.codes[code as usize] {
            TtyCode::None => b"",
            TtyCode::String(s) => s,
            _ => panic!("not a string: {}", code as usize),
        }
    }
    pub fn number(&self, code: TtyCodeCode) -> i32 {
        match self.codes[code as usize] {
            TtyCode::None => 0,
            TtyCode::Number(n) => n,
            _ => panic!("not a number: {}", code as usize),
        }
    }
    pub fn flag(&self, code: TtyCodeCode) -> bool {
        match self.codes[code as usize] {
            TtyCode::None => false,
            TtyCode::Flag(b) => b,
            _ => panic!("not a flag: {}", code as usize),
        }
    }
    fn expand(
        &self,
        state: &mut TparmState,
        code: TtyCodeCode,
        args: &[TparmArg<'_>],
        out: &mut Vec<u8>,
    ) {
        if tparm::expand(state, self.string(code), args, out).is_err() {
            out.clear();
            rmux_util::log_debug!("could not expand {}", CODES[code as usize].name);
        }
    }
    pub fn string_i(&self, state: &mut TparmState, code: TtyCodeCode, a: i32, out: &mut Vec<u8>) {
        self.expand(state, code, &[TparmArg::Int(a.into())], out);
    }
    pub fn string_ii(
        &self,
        state: &mut TparmState,
        code: TtyCodeCode,
        a: i32,
        b: i32,
        out: &mut Vec<u8>,
    ) {
        self.expand(
            state,
            code,
            &[TparmArg::Int(a.into()), TparmArg::Int(b.into())],
            out,
        );
    }
    pub fn string_iii(
        &self,
        state: &mut TparmState,
        code: TtyCodeCode,
        a: i32,
        b: i32,
        c: i32,
        out: &mut Vec<u8>,
    ) {
        self.expand(
            state,
            code,
            &[
                TparmArg::Int(a.into()),
                TparmArg::Int(b.into()),
                TparmArg::Int(c.into()),
            ],
            out,
        );
    }
    pub fn string_s(&self, state: &mut TparmState, code: TtyCodeCode, a: &[u8], out: &mut Vec<u8>) {
        self.expand(state, code, &[TparmArg::Str(cstr(a))], out);
    }
    pub fn string_ss(
        &self,
        state: &mut TparmState,
        code: TtyCodeCode,
        a: &[u8],
        b: &[u8],
        out: &mut Vec<u8>,
    ) {
        self.expand(
            state,
            code,
            &[TparmArg::Str(cstr(a)), TparmArg::Str(cstr(b))],
            out,
        );
    }
    pub fn flags(&self) -> TtyTermFlags {
        self.flags
    }
    pub fn name(&self) -> &[u8] {
        self.name.as_ref()
    }
    pub fn open_serial(&self) -> u64 {
        self.open_serial
    }
    pub fn describe(&self, code: TtyCodeCode) -> String {
        let index = code as usize;
        let value = match &self.codes[index] {
            TtyCode::None => "[missing]".to_owned(),
            TtyCode::Number(n) => format!("(number) {n}"),
            TtyCode::Flag(b) => format!("(flag) {}", if *b { "true" } else { "false" }),
            TtyCode::String(s) => {
                use rmux_util::vis::{VisFlags, strnvis};
                let escaped = strnvis(
                    s,
                    128,
                    VisFlags::OCTAL | VisFlags::CSTYLE | VisFlags::TAB | VisFlags::NL,
                );
                format!("(string) {}", String::from_utf8_lossy(escaped.as_ref()))
            }
        };
        format!("{index:4}: {}: {value}", CODES[index].name)
    }
    pub fn apply_features(&mut self, host: &mut TtyHostInfo) -> bool {
        let enabled = host.features.enabled & !host.features.disabled;
        for (index, feature) in FEATURES.iter().enumerate() {
            let bit = 1 << index;
            if enabled & bit == 0 || self.applied_features & bit != 0 {
                continue;
            }
            for cap in feature.capabilities {
                self.apply(cap.as_bytes(), true, feature.flags);
            }
            self.flags.insert(feature.flags & !TtyTermFlags::NOREPLACE);
            if feature.name == "utf8" {
                host.utf8 = true;
            }
        }
        let changed = self.applied_features | enabled != self.applied_features;
        self.applied_features |= enabled;
        changed
    }
    pub fn feature_present(&self, name: &str, utf8: bool) -> bool {
        if name == "utf8" {
            return utf8;
        }
        let Some((index, feature)) = FEATURES.iter().enumerate().find(|(_, f)| f.name == name)
        else {
            return false;
        };
        if self.applied_features & (1 << index) != 0 {
            return true;
        }
        if feature.capabilities.is_empty()
            || name == "ignorefkeys"
            || !self.flags.contains(feature.flags)
        {
            return false;
        }
        feature
            .capabilities
            .iter()
            .all(|cap| self.has_name(cap.split('=').next().unwrap_or(cap)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(extra: &[&[u8]]) -> TtyTerm {
        let mut caps = vec![
            ByteString(b"clear=x".to_vec()),
            ByteString(b"cup=%i%p1%d;%p2%d".to_vec()),
            ByteString(b"am=1".to_vec()),
        ];
        caps.extend(extra.iter().map(|s| ByteString(s.to_vec())));
        TtyTerm::create(
            &mut TparmState::default(),
            b"test",
            &caps,
            &mut TtyHostInfo::default(),
            &TtyOptions::default(),
            None,
        )
        .unwrap()
    }
    #[test]
    fn codes_match_pinned_table() {
        let fixture = include_str!("../tests/fixtures/tty-term-codes.c");
        assert_eq!(ncodes(), TtyCodeCode::COUNT);
        for (index, entry) in CODES.iter().enumerate() {
            assert_eq!(entry.code as usize, index);
            let kind = match entry.kind {
                CodeKind::String => "STRING",
                CodeKind::Number => "NUMBER",
                CodeKind::Flag => "FLAG",
            };
            let symbol = format!("TTYC_{}", format!("{:?}", entry.code).to_uppercase());
            let row = fixture
                .lines()
                .find(|line| line.contains(&format!("[{symbol}]")))
                .unwrap();
            assert!(row.contains(&format!("TTYCODE_{kind}")));
            assert!(row.contains(&format!("\"{}\"", entry.name)));
        }
    }
    #[test]
    fn describe_and_false_presence() {
        let term = fixture(&[b"bce=0", b"colors=256", b"bel=\x07"]);
        assert!(term.has(TtyCodeCode::Bce));
        assert!(!term.flag(TtyCodeCode::Bce));
        assert_eq!(term.describe(TtyCodeCode::Bce), "   3: bce: (flag) false");
        assert_eq!(
            term.describe(TtyCodeCode::Colors),
            "  13: colors: (number) 256"
        );
        assert_eq!(term.describe(TtyCodeCode::Ax), "   2: AX: [missing]");
        assert_eq!(term.describe(TtyCodeCode::Bel), "   4: bel: (string) \\a");
    }
    #[test]
    fn override_vis_remove_limits_and_no_replace() {
        let mut term = fixture(&[b"colors=8"]);
        term.apply(b"bel=\\E[1::2m:colors=bad:AX=0", false, TtyTermFlags(0));
        assert_eq!(term.string(TtyCodeCode::Bel), b"\x1b[1:2m");
        assert_eq!(term.number(TtyCodeCode::Colors), 8);
        assert!(term.flag(TtyCodeCode::Ax));
        term.apply(b"bel=second", false, TtyTermFlags::NOREPLACE);
        assert_eq!(term.string(TtyCodeCode::Bel), b"\x1b[1:2m");
        term.apply(b"bel@", false, TtyTermFlags(0));
        assert!(!term.has(TtyCodeCode::Bel));
        term.apply(b"bel=\\q", false, TtyTermFlags(0));
        assert_eq!(term.string(TtyCodeCode::Bel), b"\\q");
        let mut accepted = b"bel=".to_vec();
        accepted.resize(8190, b'x');
        term.apply(&accepted, false, TtyTermFlags(0));
        assert_eq!(term.string(TtyCodeCode::Bel).len(), 8186);
        let mut rejected = b"bel=".to_vec();
        rejected.resize(8191, b'y');
        rejected.extend_from_slice(b":colors=16");
        term.apply(&rejected, false, TtyTermFlags(0));
        assert_eq!(term.string(TtyCodeCode::Bel).len(), 8186);
        assert_eq!(term.number(TtyCodeCode::Colors), 8);
    }
    #[test]
    fn padding_raw_limits_and_invalid_ms() {
        assert_eq!(strip(b"a$<5>b$<1/>c"), b"abc");
        assert_eq!(strip(b"a$<bad"), b"a");
        assert_eq!(strip(&vec![b'a'; 9000]).len(), 9000);
        let mut padded = vec![b'a'; 9000];
        padded[0] = b'$';
        assert_eq!(strip(&padded).len(), 8191);
        let mut term = fixture(&[b"Ms="]);
        assert!(term.flags().contains(TtyTermFlags::INVALIDMS));
        assert!(!term.has(TtyCodeCode::Ms));
        term.apply_overrides(&mut TparmState::default(), &[]);
        assert!(term.flags().contains(TtyTermFlags::INVALIDMS));
        term.apply(b"Ms=\\E]52;%p1%s;%p2%s\\a", true, TtyTermFlags(0));
        term.apply_overrides(&mut TparmState::default(), &[]);
        assert!(!term.flags().contains(TtyTermFlags::INVALIDMS));
    }
    #[test]
    fn override_flag_derivation_and_matching() {
        let mut term = fixture(&[]);
        term.apply_overrides(
            &mut TparmState::default(),
            &[
                ByteString(b"wrong:am@:Rect".to_vec()),
                ByteString(b"te*:setrgbf=x:setrgbb=x:Cmg=x:Clmg=x:Rect:am@".to_vec()),
            ],
        );
        assert!(term.flags().contains(
            TtyTermFlags::RGBCOLOURS
                | TtyTermFlags::DECSLRM
                | TtyTermFlags::DECFRA
                | TtyTermFlags::NOAM
        ));
        term.apply(b"setrgbf@:Clmg@:Rect@:am", false, TtyTermFlags(0));
        term.apply_overrides(&mut TparmState::default(), &[]);
        assert!(!term.flags().intersects(
            TtyTermFlags::RGBCOLOURS
                | TtyTermFlags::DECSLRM
                | TtyTermFlags::DECFRA
                | TtyTermFlags::NOAM
        ));
    }
    #[test]
    fn features_create_requirements_and_serial() {
        let mut state = TparmState::default();
        let mut host = TtyHostInfo::default();
        assert_eq!(
            TtyTerm::create(
                &mut state,
                b"test",
                &vec![],
                &mut host,
                &TtyOptions::default(),
                None
            )
            .unwrap_err()
            .as_ref(),
            b"terminal does not support clear"
        );
        assert_eq!(
            TtyTerm::create(
                &mut state,
                b"test",
                &vec![ByteString(b"clear=x".to_vec())],
                &mut host,
                &TtyOptions::default(),
                None
            )
            .unwrap_err()
            .as_ref(),
            b"terminal does not support cup"
        );
        let mut term = fixture(&[b"Ss=original"]);
        features::parse_features("RGB,cstyle,utf8,ignorefkeys", ",", &mut host.features);
        assert!(term.apply_features(&mut host));
        assert!(!term.apply_features(&mut host));
        assert!(host.utf8);
        assert_eq!(term.string(TtyCodeCode::Ss), b"original");
        assert!(term.feature_present("RGB", false));
        assert!(term.feature_present("ignorefkeys", false));
        assert!(!term.feature_present("rgb", false));
        assert!(!fixture(&[]).feature_present("ignorefkeys", false));
        assert!(fixture(&[]).open_serial() > term.open_serial());
        let opts = TtyOptions {
            terminal_features: vec![ByteString(b"te*:mouse:RGB@".to_vec())],
            ..TtyOptions::default()
        };
        let caps = vec![
            ByteString(b"clear=\x1b[H".to_vec()),
            ByteString(b"cup=x".to_vec()),
            ByteString(b"RGB=0".to_vec()),
        ];
        let term = TtyTerm::create(
            &mut state,
            b"test",
            &caps,
            &mut TtyHostInfo::default(),
            &opts,
            Some(b"truecolor"),
        )
        .unwrap();
        assert!(term.flags().contains(TtyTermFlags::VT100LIKE));
        assert!(term.feature_present("mouse", false));
        assert!(!term.flags().contains(TtyTermFlags::RGBCOLOURS));
    }
    #[test]
    #[should_panic(expected = "not a string")]
    fn wrong_kind_is_invariant_failure() {
        fixture(&[b"colors=8"]).string(TtyCodeCode::Colors);
    }
}
