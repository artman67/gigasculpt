//! [`Bits`]: one bit per element, packed in 64-bit words. The storage of a `Flag` column (the
//! hidden faces, decision 0015) and of the hidden-point bits derived from it.

/// A fixed number of bits, packed in 64-bit words. Bits past `len` are always zero, so equal
/// sets compare equal word for word and counts never see stale bits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bits {
    words: Vec<u64>,
    len: usize,
}

impl Bits {
    /// `len` bits, all clear.
    pub fn zeros(len: usize) -> Bits {
        Bits {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Bit `i`.
    ///
    /// # Panics
    /// If `i` is not below `len`.
    #[inline]
    pub fn get(&self, i: usize) -> bool {
        assert!(i < self.len, "bit {i} of {}", self.len);
        self.words[i / 64] >> (i % 64) & 1 == 1
    }

    /// Set or clear bit `i`.
    ///
    /// # Panics
    /// If `i` is not below `len`.
    #[inline]
    pub fn set(&mut self, i: usize, on: bool) {
        assert!(i < self.len, "bit {i} of {}", self.len);
        let (w, b) = (i / 64, 1u64 << (i % 64));
        if on {
            self.words[w] |= b;
        } else {
            self.words[w] &= !b;
        }
    }

    /// Grow or shrink to `len` bits; new bits are clear.
    pub fn resize(&mut self, len: usize) {
        self.words.resize(len.div_ceil(64), 0);
        self.len = len;
        self.clear_tail();
    }

    /// How many bits are set.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether any bit is set.
    pub fn any(&self) -> bool {
        self.words.iter().any(|&w| w != 0)
    }

    /// Set every bit set in `o`.
    pub fn or(&mut self, o: &Bits) {
        self.combine(o, |w, x| w | x);
    }

    /// Clear every bit set in `o`.
    pub fn and_not(&mut self, o: &Bits) {
        self.combine(o, |w, x| w & !x);
    }

    /// Each word replaced by `f` of it and `o`'s word.
    fn combine(&mut self, o: &Bits, f: impl Fn(u64, u64) -> u64) {
        assert_eq!(self.len, o.len, "bit sets of different lengths");
        for (w, x) in self.words.iter_mut().zip(&o.words) {
            *w = f(*w, *x);
        }
    }

    /// Bytes the words take.
    pub fn bytes(&self) -> usize {
        self.words.len() * 8
    }

    /// Keep the bits past `len` clear.
    fn clear_tail(&mut self) {
        let rest = self.len % 64;
        if let (Some(last), true) = (self.words.last_mut(), rest != 0) {
            *last &= (1u64 << rest) - 1;
        }
    }
}

impl FromIterator<bool> for Bits {
    fn from_iter<I: IntoIterator<Item = bool>>(it: I) -> Bits {
        let mut b = Bits::default();
        for on in it {
            if b.len % 64 == 0 {
                b.words.push(0);
            }
            b.len += 1;
            if on {
                b.set(b.len - 1, true);
            }
        }
        b
    }
}
