// Ported from tmux compat/bitstring.h @ 8f25579c
//! `BitSet`: the `bitstr_t` replacement (`bit_alloc`, `bit_set`, `bit_clear`,
//! `bit_test`, `bit_nset`, `bit_nclear`, `bit_ffs`, `bit_ffc`).

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BitSet {
    words: Vec<u64>,
    nbits: usize,
}

impl BitSet {
    /// `bit_alloc(nbits)`: all bits clear.
    pub fn new(nbits: usize) -> BitSet {
        BitSet {
            words: vec![0; nbits.div_ceil(64)],
            nbits,
        }
    }

    /// Number of bits.
    pub fn len(&self) -> usize {
        self.nbits
    }

    pub fn is_empty(&self) -> bool {
        self.nbits == 0
    }

    fn index(&self, bit: usize) -> (usize, u64) {
        assert!(
            bit < self.nbits,
            "bit {bit} out of range for BitSet of {} bits",
            self.nbits
        );
        (bit / 64, 1u64 << (bit % 64))
    }

    /// `bit_test`.
    pub fn test(&self, bit: usize) -> bool {
        let (w, m) = self.index(bit);
        self.words[w] & m != 0
    }

    /// `bit_set`.
    pub fn set(&mut self, bit: usize) {
        let (w, m) = self.index(bit);
        self.words[w] |= m;
    }

    /// `bit_clear`.
    pub fn clear(&mut self, bit: usize) {
        let (w, m) = self.index(bit);
        self.words[w] &= !m;
    }

    /// `bit_nset(start, stop)`: inclusive; nothing when `start > stop`.
    pub fn set_range(&mut self, start: usize, stop: usize) {
        for bit in start..=stop {
            self.set(bit);
        }
    }

    /// `bit_nclear(start, stop)`: inclusive; nothing when `start > stop`.
    pub fn clear_range(&mut self, start: usize, stop: usize) {
        for bit in start..=stop {
            self.clear(bit);
        }
    }

    /// `bit_nset(0, nbits - 1)`.
    pub fn set_all(&mut self) {
        if self.nbits == 0 {
            return;
        }
        self.words.fill(u64::MAX);
        let tail = self.nbits % 64;
        if tail != 0 {
            let last = self.words.len() - 1;
            self.words[last] = (1u64 << tail) - 1;
        }
    }

    /// `bit_nclear(0, nbits - 1)`.
    pub fn clear_all(&mut self) {
        self.words.fill(0);
    }

    /// `bit_ffs`: the lowest set bit, `None` when every bit is clear.
    pub fn find_first_set(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .find(|&(_, &w)| w != 0)
            .map(|(i, &w)| i * 64 + usize::try_from(w.trailing_zeros()).unwrap_or(0))
    }

    /// `bit_ffc`: the lowest clear bit below `len()`, `None` when all set.
    pub fn find_first_clear(&self) -> Option<usize> {
        self.words
            .iter()
            .enumerate()
            .find(|&(_, &w)| w != u64::MAX)
            .map(|(i, &w)| i * 64 + usize::try_from((!w).trailing_zeros()).unwrap_or(0))
            .filter(|&bit| bit < self.nbits)
    }

    /// Set bits in ascending order.
    pub fn iter_set(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.nbits).filter(move |&b| self.test(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_clear_test() {
        let mut b = BitSet::new(130);
        assert_eq!(b.len(), 130);
        assert!(!b.is_empty());
        assert!(!b.test(0));
        b.set(0);
        b.set(63);
        b.set(64);
        b.set(129);
        assert!(b.test(0) && b.test(63) && b.test(64) && b.test(129));
        assert!(!b.test(1) && !b.test(128));
        b.clear(64);
        assert!(!b.test(64));
        assert_eq!(b.iter_set().collect::<Vec<_>>(), vec![0, 63, 129]);
    }

    #[test]
    fn find_first() {
        let mut b = BitSet::new(70);
        assert_eq!(b.find_first_set(), None);
        assert_eq!(b.find_first_clear(), Some(0));
        b.set(65);
        assert_eq!(b.find_first_set(), Some(65));
        b.set(3);
        assert_eq!(b.find_first_set(), Some(3));
        b.set_all();
        assert_eq!(b.find_first_clear(), None);
        assert_eq!(b.find_first_set(), Some(0));
        b.clear(69);
        assert_eq!(b.find_first_clear(), Some(69));
        b.clear_all();
        assert_eq!(b.find_first_set(), None);
        assert_eq!(b.find_first_clear(), Some(0));
    }

    #[test]
    fn ranges_and_tabs() {
        let mut tabs = BitSet::new(80);
        for i in (8..80).step_by(8) {
            tabs.set(i);
        }
        assert_eq!(tabs.iter_set().count(), 9);
        tabs.clear_range(0, 79);
        assert_eq!(tabs.find_first_set(), None);
        tabs.set_range(10, 12);
        assert_eq!(tabs.iter_set().collect::<Vec<_>>(), vec![10, 11, 12]);
        tabs.set_range(5, 4);
        assert_eq!(tabs.iter_set().count(), 3);
    }

    #[test]
    fn empty_and_exact_word() {
        let mut e = BitSet::new(0);
        assert!(e.is_empty());
        e.set_all();
        assert_eq!(e.find_first_set(), None);
        assert_eq!(e.find_first_clear(), None);
        let mut w = BitSet::new(64);
        w.set_all();
        assert_eq!(w.find_first_clear(), None);
        assert!(w.test(63));
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn out_of_range_panics() {
        BitSet::new(8).set(8);
    }
}
