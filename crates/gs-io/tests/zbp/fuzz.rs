//! Fuzz properties: mutated, spliced and random files are rejected or read, never a panic.
//! The seeds are our own synthetic brushes, so every part of the reader is reached.

use gs_io::zbp::{Image, ZbpBrush, read};
use proptest::prelude::*;

use crate::sample;

/// A small brush (no alpha) and the full one, so mutations hit every part often.
fn seeds() -> [Vec<u8>; 2] {
    let small = crate::writer::Brush {
        alpha: None,
        ..sample::brush()
    };
    [sample::bytes_of(&small), sample::full()]
}

/// Every image that does decode has exactly `width * height` pixels.
fn whole(image: &Image) -> bool {
    let pixels =
        u64::from(image.width) * u64::from(image.height) * u64::from(image.bytes_per_pixel);
    image.data.len() as u64 == pixels
}

fn images_whole(b: &ZbpBrush) -> bool {
    let sections = b.sections.iter().flat_map(|s| &s.images);
    whole(&b.icon.image)
        && b.alpha.iter().all(|a| whole(&a.image))
        && sections.into_iter().all(whole)
}

proptest! {
    #[test]
    fn flipped_bytes_never_panic(seed in 0usize..2, flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = seeds()[seed].clone();
        for (at, value) in flips {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        if let Ok(b) = read(&bytes) {
            prop_assert!(images_whole(&b));
        }
    }

    #[test]
    fn spliced_files_never_panic(seed in 0usize..2, cut in any::<prop::sample::Index>(), drop in 0usize..64, insert in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut bytes = seeds()[seed].clone();
        let at = cut.index(bytes.len());
        let end = (at + drop).min(bytes.len());
        bytes.splice(at..end, insert);
        if let Ok(b) = read(&bytes) {
            prop_assert!(images_whole(&b));
        }
    }

    #[test]
    fn noise_after_the_preamble_never_panics(tail in prop::collection::vec(any::<u8>(), 0..512)) {
        let bytes = [&sample::full()[..92], &tail].concat();
        prop_assert!(read(&bytes).is_err(), "noise is not a brush");
    }
}
