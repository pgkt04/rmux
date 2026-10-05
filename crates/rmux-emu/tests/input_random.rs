//! Randomized parser robustness test without the oracle: random byte and
//! token streams must never panic, must leave the screen state within its
//! bounds, must report pending bytes exactly outside the ground state, and
//! must give the same result whether fed whole or split in two.
#[path = "../../rmux-util/tests/common/mod.rs"]
mod common;
mod input_corpus;

use common::Rng;
use input_corpus::{Emu, dictionary_stream};

const SX: u32 = 80;
const SY: u32 = 25;
const HLIMIT: u32 = 100;
const SEEDS: u64 = 2000;
const MAX_LEN: usize = 512;

fn random_stream(rng: &mut Rng) -> Vec<u8> {
    let len = rng.below(MAX_LEN as u64) as usize + 1;
    match rng.below(3) {
        0 => (0..len).map(|_| rng.next_u64() as u8).collect(),
        1 => {
            let mut out = dictionary_stream(rng, len);
            out.truncate(len);
            out
        }
        _ => {
            let mut out = dictionary_stream(rng, len / 2);
            for _ in 0..len / 4 {
                let at = rng.below(out.len() as u64 + 1) as usize;
                out.insert(at, rng.next_u64() as u8);
            }
            out
        }
    }
}

fn check_invariants(emu: &Emu, seed: u64) {
    let s = &emu.screen;
    let (sx, sy) = (s.grid.sx(), s.grid.sy());
    assert!(s.cx <= sx, "seed {seed}: cx {} > sx {sx}", s.cx);
    assert!(s.cy < sy, "seed {seed}: cy {} >= sy {sy}", s.cy);
    assert!(
        s.rupper <= s.rlower && s.rlower < sy,
        "seed {seed}: region {}..{} outside 0..{sy}",
        s.rupper,
        s.rlower
    );
    assert_eq!(
        emu.ictx.pending().is_empty(),
        emu.ictx.state_name() == "ground",
        "seed {seed}: pending {:?} in state {}",
        emu.ictx.pending(),
        emu.ictx.state_name()
    );
}

#[test]
fn random_streams_whole_and_split_agree() {
    for seed in 1..=SEEDS {
        let mut rng = Rng::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let bytes = random_stream(&mut rng);

        let mut whole = Emu::new(SX, SY, HLIMIT);
        whole.feed(&bytes);
        check_invariants(&whole, seed);

        let at = rng.below(bytes.len() as u64 + 1) as usize;
        let mut split = Emu::new(SX, SY, HLIMIT);
        split.feed(&bytes[..at]);
        check_invariants(&split, seed);
        split.feed(&bytes[at..]);
        check_invariants(&split, seed);

        assert_eq!(
            whole.ictx.pending(),
            split.ictx.pending(),
            "seed {seed}: pending differs when split at {at}"
        );
        assert_eq!(
            whole.ictx.state_name(),
            split.ictx.state_name(),
            "seed {seed}: state differs when split at {at}"
        );
        assert_eq!(
            whole.state(),
            split.state(),
            "seed {seed}: state line differs when split at {at}"
        );
        // `-e -N` trailing cells follow `cellsize`, which tmux rounds per
        // write batch (`grid.c:311-316`); compare the used cells only.
        let (a, b) = (whole.capture_used(), split.capture_used());
        assert!(
            a == b,
            "seed {seed}: capture differs when split at {at}\n stream: {}\n whole: {}\n split: {}",
            String::from_utf8_lossy(&bytes).escape_debug(),
            String::from_utf8_lossy(&a).escape_debug(),
            String::from_utf8_lossy(&b).escape_debug()
        );
    }
}
