//! The fixture generator is the standard `SplitMix64`, and its helpers are uniform.

use std::collections::BTreeMap;

use gs_testkit::SplitMix64;
use gs_testkit::rng::signed_unit;

use crate::data::tally as counts;

/// Reference outputs of the published `SplitMix64` (Vigna's `splitmix64.c`), recomputed
/// independently in Python for seeds 1234567 and 0.
#[test]
fn the_generator_matches_the_published_sequence() {
    let got: Vec<u64> = SplitMix64::new(1_234_567).take(5).collect();
    let want = [
        6_457_827_717_110_365_317,
        3_203_168_211_198_807_973,
        9_817_491_932_198_370_423,
        4_593_380_528_125_082_431,
        16_408_922_859_458_223_821,
    ];
    assert_eq!(got, want, "seed 1234567");
    let mut zero = SplitMix64::new(0);
    let got = [zero.next_u64(), zero.next_u64(), zero.next_u64()];
    let want = [
        0xE220_A839_7B1D_CDAF,
        0x6E78_9E6A_A1B9_65F4,
        0x06C4_5D18_8009_454F,
    ];
    assert_eq!(got, want, "seed 0");
}

#[test]
fn below_is_uniform_over_its_range() {
    let mut rng = SplitMix64::new(9);
    let seen = counts((0..10_000).map(|_| rng.below(10)));
    assert_eq!(
        seen.keys().copied().collect::<Vec<_>>(),
        (0..10).collect::<Vec<_>>(),
        "0 to 9"
    );
    for (value, n) in &seen {
        assert!(
            (900..=1100).contains(n),
            "{value} came up {n} times in 10000"
        );
    }
    assert!((0..100).all(|_| rng.below(1) == 0), "below(1) is always 0");
    let items = ['a', 'b', 'c'];
    let picked = counts((0..300).map(|_| *rng.pick(&items) as usize));
    assert_eq!(picked.len(), 3, "pick reaches every item: {picked:?}");
}

#[test]
#[should_panic(expected = "below(0) has no possible value")]
fn below_zero_is_refused() {
    SplitMix64::new(1).below(0);
}

/// Every order of three items comes up about equally often (a Sattolo shuffle, which never
/// leaves an item in place, would reach only two of the six).
#[test]
fn shuffles_reach_every_order_evenly() {
    let mut rng = SplitMix64::new(4);
    let mut seen: BTreeMap<[u8; 3], u32> = BTreeMap::new();
    for _ in 0..6000 {
        let mut items = [0u8, 1, 2];
        rng.shuffle(&mut items);
        *seen.entry(items).or_insert(0) += 1;
    }
    assert_eq!(seen.len(), 6, "all six orders: {seen:?}");
    assert!(
        seen.values().all(|&n| (850..=1150).contains(&n)),
        "evenly: {seen:?}"
    );
}

#[test]
fn signed_units_use_the_top_24_bits() {
    let bits = |v: f32| v.to_bits();
    assert_eq!(bits(signed_unit(0)), bits(-1.0), "lowest");
    assert_eq!(bits(signed_unit(1 << 63)), bits(0.0), "middle");
    assert_eq!(
        bits(signed_unit(u64::MAX)),
        bits(1.0 - 2.0 / 16_777_216.0),
        "highest"
    );
    assert_eq!(
        bits(signed_unit((1 << 40) - 1)),
        bits(-1.0),
        "below bit 40 is ignored"
    );
}
