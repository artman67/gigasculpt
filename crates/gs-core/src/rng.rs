//! Counter-based random numbers, so results never depend on thread count or call order. The one
//! home of randomness: the engine's commands, the alpha generator's noise and (later) a stroke's
//! scatter all draw from it (decision 0018).
//!
//! The generator is `SplitMix64`: value `k` of a stream seeded with `s` is exactly the `k`-th
//! output (counting from 0) of the standard sequential `SplitMix64` generator seeded with `s`.
//! Computing it by index means any thread can draw any value. Each command gets its own stream,
//! whose seed is value `seq` of the session's stream. Commands that use randomness pin that seed
//! into their logged arguments, so a replay needs no sequence numbers.

/// The `SplitMix64` increment (the golden ratio in 64-bit fixed point).
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// One stream of random numbers.
#[derive(Clone, Copy, Debug)]
pub struct CommandRng {
    seed: u64,
}

impl CommandRng {
    /// The stream seeded with `seed`.
    pub fn from_seed(seed: u64) -> Self {
        Self { seed }
    }

    /// The stream of command `seq` in a session seeded with `session_seed`.
    pub fn for_command(session_seed: u64, seq: u64) -> Self {
        Self::from_seed(Self::from_seed(session_seed).u64_at(seq))
    }

    /// This stream's seed.
    pub fn seed(self) -> u64 {
        self.seed
    }

    /// Value `index` of the stream.
    pub fn u64_at(self, index: u64) -> u64 {
        let mut z = self
            .seed
            .wrapping_add(index.wrapping_add(1).wrapping_mul(GAMMA));
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Value `index` as a float in `[0, 1)`: the top 24 bits, scaled exactly.
    pub fn unit_at(self, index: u64) -> f32 {
        (self.u64_at(index) >> 40) as f32 / 16_777_216.0
    }

    /// Value `index` as a float in `[-1, 1)`: [`CommandRng::unit_at`] stretched.
    pub fn signed_unit_at(self, index: u64) -> f32 {
        self.unit_at(index) * 2.0 - 1.0
    }
}
