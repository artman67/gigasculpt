//! gs-par tests: the thread-count determinism test from `docs/spikes/determinism.md`, and each
//! function against a plain serial loop (differential tests, ARCHITECTURE 10.1 kind 3).

use std::ops::Range;

use gs_par::{
    CHUNK, apply, apply_each, apply_ranges, map, reduce, sort, sort_keys, threads, with_threads,
};

const THREADS: [usize; 5] = [1, 2, 3, 8, 24];
/// A few hundred thousand elements, as the spike recommends: big enough that a reduction whose
/// grouping follows the thread count really changes the bits of an f32 sum.
const N: usize = 300_000;

/// splitmix64, so the data is the same on every machine.
fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// Values of mixed sign and magnitude, so rounding depends on the order of additions.
fn value(i: usize) -> f32 {
    let r = mix(i as u64);
    let unit = (r >> 40) as f32 / (1u64 << 24) as f32 - 0.5;
    unit * [1.0e-3, 1.0, 1.0e3][(r % 3) as usize]
}

/// A small two-phase kernel over the data: a weighted sum, then each element pulled toward
/// the average of its neighbours read from the unchanged values.
fn kernel() -> (u32, Vec<u32>) {
    let data: Vec<f32> = map(N, value);
    let sum = reduce(N, || 0.0f32, |acc, i| acc + data[i] * 0.5, |a, b| a + b);
    let deltas: Vec<f32> = map(N, |i| {
        let left = data[i.saturating_sub(1)];
        let right = data[(i + 1).min(N - 1)];
        f32::midpoint(left, right) - data[i]
    });
    let mut out = data.clone();
    apply(&mut out, |i, x| *x += 0.25 * deltas[i] + sum * 1.0e-9);
    (sum.to_bits(), out.iter().map(|x| x.to_bits()).collect())
}

#[test]
fn results_are_bit_identical_on_1_2_3_8_and_24_threads() {
    let runs: Vec<(u32, Vec<u32>)> = THREADS
        .into_iter()
        .map(|t| with_threads(t, kernel))
        .collect();
    for (t, run) in THREADS.iter().zip(&runs) {
        assert_eq!(run.0, runs[0].0, "f32 sum on {t} threads");
        assert!(run.1 == runs[0].1, "kernel output on {t} threads");
    }
    let main_thread = kernel();
    assert_eq!(main_thread.0, runs[0].0, "f32 sum from the global pool");
    assert!(
        main_thread.1 == runs[0].1,
        "kernel output from the global pool"
    );
}

#[test]
fn reduce_folds_fixed_chunks_and_combines_them_in_order() {
    // The reference: the same chunks, folded and combined left to right by plain loops.
    let mut expected = 0.0f32;
    for start in (0..N).step_by(CHUNK) {
        let mut chunk = 0.0f32;
        for i in start..(start + CHUNK).min(N) {
            chunk += value(i);
        }
        expected += chunk;
    }
    let got = with_threads(8, || {
        reduce(N, || 0.0f32, |a, i| a + value(i), |a, b| a + b)
    });
    assert_eq!(got.to_bits(), expected.to_bits(), "chunked sum");
    let serial: f32 = (0..N).map(value).sum();
    assert_ne!(
        serial.to_bits(),
        expected.to_bits(),
        "non-trivial: the chunking changes the bits, so the test sees a wrong grouping"
    );
    let order = reduce(
        10_000,
        Vec::new,
        |mut v, i| {
            v.push(i);
            v
        },
        |mut a, b| {
            a.extend(b);
            a
        },
    );
    assert!(
        order.iter().copied().eq(0..10_000),
        "every index once, in order"
    );
    assert!(
        reduce(0, || 7u8, |a, _| a + 1, Ord::max) == 7,
        "empty input gives the identity"
    );
}

#[test]
fn map_and_apply_visit_every_index_once() {
    let squares = with_threads(3, || map(N, |i| i * i));
    assert!(
        squares.iter().enumerate().all(|(i, &s)| s == i * i),
        "map keeps index order"
    );
    let mut seen = vec![0usize; N];
    with_threads(3, || apply(&mut seen, |i, x| *x += i + 1));
    assert!(
        seen.iter().enumerate().all(|(i, &x)| x == i + 1),
        "apply passes each element its own index"
    );
}

#[test]
fn apply_each_visits_every_item_once_with_its_own_index() {
    let mut items: Vec<Vec<usize>> = vec![Vec::new(); 37];
    with_threads(4, || apply_each(&mut items, |i, v| v.push(i * 3)));
    assert!(
        items.iter().enumerate().all(|(i, v)| v == &[i * 3]),
        "each item once, with its index"
    );
}

#[test]
fn apply_ranges_writes_each_range_once_and_rejects_overlap() {
    let mut v = vec![0u32; 10];
    apply_ranges(&mut v, &[0..3, 3..3, 5..10], |k, start, s| {
        for (j, x) in s.iter_mut().enumerate() {
            *x += (k * 100 + start + j) as u32;
        }
    });
    assert_eq!(
        v,
        [0, 1, 2, 0, 0, 205, 206, 207, 208, 209],
        "each element written by its own range"
    );
    let overlap =
        std::panic::catch_unwind(|| apply_ranges(&mut [0u8; 4], &[0..3, 2..4], |_, _, _| {}));
    assert!(overlap.is_err(), "overlapping ranges must panic");
    let backwards = std::panic::catch_unwind(|| {
        apply_ranges(
            &mut [0u8; 4],
            &[0..1, Range { start: 3, end: 2 }],
            |_, _, _| {},
        );
    });
    assert!(
        backwards.is_err(),
        "a range that ends before it starts must panic"
    );
    let touching =
        std::panic::catch_unwind(|| apply_ranges(&mut [0u8; 4], &[0..2, 2..4], |_, _, _| {}));
    assert!(
        touching.is_ok(),
        "ranges that touch but do not overlap are fine"
    );
}

/// Compares by key only, so a sort that is not stable can reorder equal keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Keyed(u32, u32);

impl PartialOrd for Keyed {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Keyed {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[test]
fn sort_is_stable_and_the_same_on_every_thread_count() {
    let input: Vec<Keyed> = (0..N as u32)
        .map(|i| Keyed((mix(u64::from(i)) % 64) as u32, i))
        .collect();
    let mut expected = input.clone();
    expected.sort();
    for t in THREADS {
        let mut got = input.clone();
        with_threads(t, || sort(&mut got));
        assert!(got == expected, "stable sort on {t} threads");
    }
}

#[test]
fn with_threads_sets_the_pool_size() {
    for t in [1, 3, 8] {
        assert_eq!(with_threads(t, threads), t, "pool of {t}");
    }
}

#[test]
fn sort_keys_sorts_and_is_the_same_on_every_thread_count() {
    let input: Vec<u64> = (0..N as u64).map(|i| mix(i) % 1000).collect();
    let mut expected = input.clone();
    expected.sort_unstable();
    for t in THREADS {
        let mut got = input.clone();
        with_threads(t, || sort_keys(&mut got));
        assert!(got == expected, "sorted keys on {t} threads");
    }
}
