//! `gs-par`: the only way GigaSculpt code runs in parallel (`deny.toml` keeps every other crate
//! away from rayon). Every function gives bit-identical results at any thread count, by
//! construction (`docs/spikes/determinism.md`):
//!
//! * [`reduce`] splits `0..len` into chunks whose boundaries depend only on `len` (never on the
//!   thread count or on work stealing), folds each chunk serially, and combines the chunk
//!   results serially in index order.
//! * [`map`] computes one independent value per index. No value reads another, so the order of
//!   evaluation cannot matter.
//! * [`apply`], [`apply_each`] and [`apply_ranges`] write each element through exactly one
//!   owner. Kernels read a
//!   snapshot (or the deltas from [`map`]) and never see a neighbor's half-written value.
//! * [`sort`] is a stable sort, whose result is unique for a given input and order.
//!
//! A kernel is written in two phases: phase 1 reads the mesh (read-only) and produces deltas
//! with [`map`] or a value with [`reduce`]; phase 2 writes with [`apply`].

use std::ops::Range;
use std::sync::OnceLock;

use rayon::prelude::*;

/// Elements per reduction chunk. A constant on purpose: deriving it from the thread count would
/// make results depend on the machine. Changing it changes the bits of every reduction.
pub const CHUNK: usize = 4096;

/// Folds `fold(acc, i)` over `0..len` in fixed chunks of [`CHUNK`], then combines the chunk
/// results serially in index order. Same bits at 1 or 24 threads.
pub fn reduce<A, Id, F, C>(len: usize, identity: Id, fold: F, combine: C) -> A
where
    A: Send,
    Id: Fn() -> A + Sync,
    F: Fn(A, usize) -> A + Sync,
    C: Fn(A, A) -> A,
{
    let partials: Vec<A> = (0..len.div_ceil(CHUNK))
        .into_par_iter()
        .map(|c| (c * CHUNK..((c + 1) * CHUNK).min(len)).fold(identity(), &fold))
        .collect();
    partials.into_iter().fold(identity(), combine)
}

/// Returns `[f(0), f(1), .., f(len - 1)]`, computed in parallel.
pub fn map<T: Send>(len: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    (0..len).into_par_iter().map(&f).collect()
}

/// Calls `f(i, &mut data[i])` for every element, in parallel. Each element has one owner.
pub fn apply<T: Send>(data: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
    data.par_chunks_mut(CHUNK)
        .enumerate()
        .for_each(|(c, chunk)| {
            for (k, x) in chunk.iter_mut().enumerate() {
                f(c * CHUNK + k, x);
            }
        });
}

/// Calls `f(i, &mut items[i])` for every item, in parallel, one task per item. For a few big
/// items, such as the mesh nodes a brush dab touches: [`apply`] packs [`CHUNK`] elements into one
/// task, which would run a whole dab's nodes on one thread.
pub fn apply_each<T: Send>(items: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
    items
        .par_iter_mut()
        .with_max_len(1)
        .enumerate()
        .for_each(|(i, x)| f(i, x));
}

/// Calls `f(range_index, first_element_index, &mut data[range])` for each range, in parallel.
/// For storage where each spatial-tree leaf owns a contiguous run of elements.
///
/// # Panics
/// If the ranges are not sorted, overlap, or run past the end of `data`.
pub fn apply_ranges<T: Send>(
    data: &mut [T],
    ranges: &[Range<usize>],
    f: impl Fn(usize, usize, &mut [T]) + Sync,
) {
    let mut parts: Vec<(usize, usize, &mut [T])> = Vec::with_capacity(ranges.len());
    let mut rest = data;
    let mut at = 0;
    for (k, r) in ranges.iter().enumerate() {
        assert!(
            r.start >= at && r.start <= r.end,
            "ranges must be sorted and disjoint"
        );
        let (_, tail) = rest.split_at_mut(r.start - at);
        let (mine, tail) = tail.split_at_mut(r.end - r.start);
        parts.push((k, r.start, mine));
        rest = tail;
        at = r.end;
    }
    parts
        .into_par_iter()
        .for_each(|(k, start, slice)| f(k, start, slice));
}

/// Sorts `data` in parallel, stably. A stable sort has exactly one possible result for a given
/// input, so the thread count cannot change it.
pub fn sort<T: Ord + Send>(data: &mut [T]) {
    data.par_sort();
}

/// Sorts plain integer keys in parallel, about twice as fast as [`sort`]. An unstable sort may
/// reorder equal elements, but equal integers cannot be told apart, so the result is still the
/// same at any thread count. That is why this takes `u64` and nothing else.
pub fn sort_keys(keys: &mut [u64]) {
    keys.par_sort_unstable();
}

/// Runs `f` on a private pool of `threads` threads, so every `gs-par` call inside it uses that
/// many. For determinism tests and benchmarks; the engine uses the global pool.
///
/// # Panics
/// If the operating system refuses to start the threads.
pub fn with_threads<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap_or_else(|e| panic!("could not start {threads} threads: {e}"));
    pool.install(f)
}

/// Threads of the background pool.
pub const BACKGROUND_THREADS: usize = 4;

/// Runs `f` on the background pool: [`BACKGROUND_THREADS`] threads apart from the global pool,
/// for work that must leave the stroke threads alone, such as a save's chunk encoding (decision
/// 0016). Every `gs-par` call inside it uses that pool, and gives the same bits as anywhere. If
/// the pool's threads cannot start, `f` runs where it is.
pub fn background<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(BACKGROUND_THREADS)
            .thread_name(|i| format!("gs-background-{i}"))
            .build()
            .ok()
    });
    match pool {
        Some(pool) => pool.install(f),
        None => f(),
    }
}

/// How many threads `gs-par` calls use here: the private pool's size inside [`with_threads`],
/// otherwise the global pool's.
pub fn threads() -> usize {
    rayon::current_num_threads()
}
