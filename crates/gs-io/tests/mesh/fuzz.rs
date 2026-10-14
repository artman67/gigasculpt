//! Fuzz properties for the OBJ, PLY and STL readers: mutated, spliced and random files are
//! rejected or read, never a panic, and whatever reads is a consistent mesh. The seeds are our
//! own writers' files, so every part of each reader is reached.

use gs_io::ReadError;
use gs_io::mesh::{MeshFile, ReadOptions, WriteOptions};
use proptest::prelude::*;

use crate::common::{consistent, full_mesh, write_as};

type Reader = fn(&[u8]) -> Result<MeshFile, ReadError>;

fn readers() -> [Reader; 3] {
    [
        |b| {
            gs_io::obj::read(
                b,
                &ReadOptions {
                    group_polygons: true,
                    ..ReadOptions::default()
                },
            )
        },
        |b| gs_io::ply::read(b, &ReadOptions::default()),
        |b| gs_io::stl::read(b),
    ]
}

/// Our own files of each format: the reader's index is `seed % 3`.
fn seeds() -> Vec<Vec<u8>> {
    let mesh = full_mesh();
    let meshes = [("a", &mesh), ("b", &mesh)];
    let mut out = Vec::new();
    for binary in [false, true] {
        let options = WriteOptions {
            binary,
            ..WriteOptions::default()
        };
        for kind in ["obj", "ply", "stl"] {
            out.push(write_as(kind, &meshes, options));
        }
    }
    out
}

fn check(seed: usize, bytes: &[u8]) -> Result<(), TestCaseError> {
    if let Ok(file) = readers()[seed % 3](bytes) {
        prop_assert!(consistent(&file));
    }
    Ok(())
}

proptest! {
    #[test]
    fn flipped_bytes_never_panic(seed in 0usize..6, flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = seeds()[seed].clone();
        for (at, value) in flips {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        check(seed, &bytes)?;
    }

    #[test]
    fn spliced_files_never_panic(seed in 0usize..6, cut in any::<prop::sample::Index>(), drop in 0usize..64, insert in prop::collection::vec(any::<u8>(), 0..64)) {
        let mut bytes = seeds()[seed].clone();
        let at = cut.index(bytes.len());
        let end = (at + drop).min(bytes.len());
        bytes.splice(at..end, insert);
        check(seed, &bytes)?;
    }

    #[test]
    fn noise_never_panics(seed in 0usize..3, noise in prop::collection::vec(any::<u8>(), 0..512)) {
        check(seed, &noise)?;
        // Noise after a valid start reaches the deeper parts of each reader.
        let start = &seeds()[seed][..64];
        check(seed, &[start, &noise].concat())?;
    }
}
