//! Fuzz properties: mutated, spliced and random tool and project files are rejected or read,
//! never a panic, and whatever reads is a consistent mesh. The seeds are our own synthetic
//! files, so every part of the reader is reached.

use gs_io::ztl::{Ztl, read};
use gs_protocol::{AttrDomain, AttrValues, NO_VERTEX};

use crate::sample;

fn seeds() -> [Vec<u8>; 2] {
    [sample::tool_file(), sample::project_file()]
}

/// Every decoded level has faces inside its vertices and attribute columns of the right length.
fn consistent(z: &Ztl) -> bool {
    z.geometry
        .iter()
        .flatten()
        .filter_map(|g| g.mesh.as_ref())
        .all(|m| {
            m.levels.iter().all(|level| {
                let mesh = &level.mesh;
                let n = mesh.positions.len();
                let faces_ok = mesh
                    .faces
                    .iter()
                    .flatten()
                    .all(|&v| v == NO_VERTEX || (v as usize) < n);
                let columns_ok = mesh.attributes.values().all(|a| {
                    let len = match &a.values {
                        AttrValues::F32(v) => v.len(),
                        AttrValues::U32(v) => v.len(),
                    };
                    len == match a.domain {
                        AttrDomain::Vertex => n,
                        AttrDomain::Face => mesh.faces.len(),
                    }
                });
                faces_ok && columns_ok
            })
        })
}

proptest::proptest! {
    #[test]
    fn flipped_bytes_never_panic(seed in 0usize..2, flips in proptest::collection::vec((proptest::prelude::any::<proptest::sample::Index>(), proptest::prelude::any::<u8>()), 1..8)) {
        let mut bytes = seeds()[seed].clone();
        for (at, value) in flips {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        if let Ok(z) = read(&bytes) {
            proptest::prop_assert!(consistent(&z));
        }
    }

    #[test]
    fn spliced_files_never_panic(seed in 0usize..2, cut in proptest::prelude::any::<proptest::sample::Index>(), drop in 0usize..64, insert in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64)) {
        let mut bytes = seeds()[seed].clone();
        let at = cut.index(bytes.len());
        let end = (at + drop).min(bytes.len());
        bytes.splice(at..end, insert);
        if let Ok(z) = read(&bytes) {
            proptest::prop_assert!(consistent(&z));
        }
    }

    #[test]
    fn noise_after_the_preamble_never_panics(tail in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
        let bytes = [&sample::tool_file()[..92], &tail].concat();
        proptest::prop_assert!(read(&bytes).is_err(), "noise is not a tool");
    }
}
