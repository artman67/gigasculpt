//! Fuzz properties: mutated, spliced and cut files never make the reader panic, and neither do
//! files damaged past the hash checks (a chunk's bytes or a manifest number changed and the
//! hashes fixed to match). Whatever reads goes through `MeshStore::from_exact`, and a Part's
//! detail levels through `Levels::from_file`, which refuse or accept but never panic either.

use std::sync::OnceLock;

use gs_core::{Levels, MeshStore, Placement};
use gs_io::scene::{Kept, NumberingIn, read_bytes};
use proptest::prelude::any;
use proptest::sample::Index;
use serde_json::Value;

use crate::sample::{file_with, levels, manifest, store, with_manifest};

fn seed() -> &'static [u8] {
    static SEED: OnceLock<Vec<u8>> = OnceLock::new();
    SEED.get_or_init(|| {
        file_with(
            &[store(5, 1), store(4, 2)],
            &[levels(6, 3)],
            Some(b"picture"),
        )
    })
}

/// Reads, and builds a store from every Part that reads. Only a panic fails.
fn survive(bytes: &[u8]) {
    if let Ok(scene) = read_bytes(bytes, Kept::None) {
        for part in scene.parts.into_iter().flatten() {
            let top = MeshStore::from_exact(part.config, part.attrs, part.nodes);
            if let (
                Ok(top),
                Some(stored),
                NumberingIn::Given {
                    vertices,
                    faces,
                    first_corner,
                },
            ) = (top, part.levels, part.numbering)
            {
                let placed = Placement {
                    vertices,
                    faces,
                    first_corner,
                };
                drop(Levels::from_file(top, placed, stored));
            }
        }
    }
}

/// Every number in the manifest, by its JSON pointer.
fn numbers(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::Number(_) => out.push(at.to_string()),
        Value::Array(items) => {
            for (i, x) in items.iter().enumerate() {
                numbers(x, &format!("{at}/{i}"), out);
            }
        }
        Value::Object(map) => {
            for (k, x) in map {
                numbers(x, &format!("{at}/{k}"), out);
            }
        }
        _ => {}
    }
}

proptest::proptest! {
    #[test]
    fn flipped_bytes_never_panic(flips in proptest::collection::vec((any::<Index>(), any::<u8>()), 1..8)) {
        let mut bytes = seed().to_vec();
        for (at, value) in flips {
            let i = at.index(bytes.len());
            bytes[i] = value;
        }
        survive(&bytes);
    }

    #[test]
    fn spliced_and_cut_files_never_panic(cut in any::<Index>(), drop in 0usize..256, insert in proptest::collection::vec(any::<u8>(), 0..64)) {
        let mut bytes = seed().to_vec();
        let at = cut.index(bytes.len());
        let end = (at + drop).min(bytes.len());
        bytes.splice(at..end, insert);
        survive(&bytes);
    }

    #[test]
    fn damaged_chunks_behind_good_hashes_never_panic(which in any::<Index>(), flips in proptest::collection::vec((any::<Index>(), any::<u8>()), 1..6)) {
        let mut bytes = seed().to_vec();
        let mut m = manifest(&bytes);
        let chunks = m["chunks"].as_array_mut().expect("chunks");
        let i = which.index(chunks.len());
        let c = &mut chunks[i];
        let (offset, stored) = (c["offset"].as_u64().unwrap_or(0) as usize, c["stored"].as_u64().unwrap_or(0) as usize);
        if stored > 0 {
            for (at, value) in flips {
                bytes[offset + at.index(stored)] = value;
            }
            let hash = blake3::hash(&bytes[offset..offset + stored]).to_hex()[..32].to_string();
            c["hash"] = Value::String(hash);
        }
        survive(&with_manifest(&bytes, &m));
    }

    #[test]
    fn changed_manifest_numbers_never_panic(which in any::<Index>(), value in proptest::prop_oneof![0u64..4, 0u64..20_000, 0u64..=u64::from(u32::MAX), proptest::strategy::Just(u64::MAX)]) {
        let bytes = seed();
        let mut m = manifest(bytes);
        let mut found = Vec::new();
        numbers(&m, "", &mut found);
        let at = &found[which.index(found.len())];
        if let Some(n) = m.pointer_mut(at) {
            *n = Value::from(value);
        }
        survive(&with_manifest(bytes, &m));
    }
}
