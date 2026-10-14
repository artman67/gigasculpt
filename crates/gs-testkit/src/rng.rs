//! A deterministic random generator for fixtures and test data.
//!
//! Research 06 (section 2.4) found that a "pseudo-random" formula put every test point on one
//! line and hid a broken grid. Test data that needs randomness comes from here: the standard
//! sequential `SplitMix64` (Steele, Lea and Flood, 2014), seeded explicitly, so the same seed
//! gives the same data on every machine. Written independently of the engine's counter-based
//! `CommandRng`, so tests can check the engine against it.

/// The standard sequential `SplitMix64` generator.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator seeded with `seed`.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`, by multiplying 64 random bits by `n` and keeping the high half. The
    /// bias is below `n / 2^64`. Panics if `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0) has no possible value");
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// One of `items`, chosen uniformly. Panics if `items` is empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Shuffles `items` uniformly (Fisher-Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

impl Iterator for SplitMix64 {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        Some(self.next_u64())
    }
}

/// A 64-bit random value as a float in `[-1, 1)`: its top 24 bits, scaled exactly. This is the
/// mapping `mesh.jitter` documents.
pub fn signed_unit(value: u64) -> f32 {
    ((value >> 40) as f32 / 16_777_216.0) * 2.0 - 1.0
}
