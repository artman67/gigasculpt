//! Damaged and incomplete files: a damaged chunk refuses only its Part, naming the chunk and
//! where it is; a Part that needs a chunk kind this version does not know is refused; a file cut
//! short anywhere is refused as incomplete; and none of it panics.

use gs_io::scene::{Kept, SceneError, read_bytes};
use serde_json::{Value, json};

use crate::sample::{file, manifest, store, with_manifest};

/// The first chunk of `kind` belonging to part `part`.
fn chunk(m: &Value, kind: &str, part: u64) -> Value {
    let chunks = m["chunks"].as_array().expect("chunks");
    let found = chunks
        .iter()
        .find(|c| c["kind"] == kind && c["part"] == part);
    found.expect("such a chunk").clone()
}

#[test]
fn a_damaged_chunk_refuses_only_its_part() {
    let bytes = file(&[store(10, 1), store(10, 2), store(10, 3)], None);
    let m = manifest(&bytes);
    let c = chunk(&m, "positions", 1);
    let at = c["offset"].as_u64().expect("offset") as usize + 3;
    let mut damaged = bytes.clone();
    damaged[at] ^= 0x40;
    let scene = read_bytes(&damaged, Kept::None).expect("the file still reads");
    assert!(
        scene.parts[0].is_ok() && scene.parts[2].is_ok(),
        "the others open"
    );
    let err = scene.parts[1].as_ref().expect_err("part 1 is refused");
    assert_eq!(err.part, "part-1");
    assert_eq!(err.offset, c["offset"].as_u64(), "names where the chunk is");
    let text = err.to_string();
    assert!(
        text.contains("positions of nodes 0 to") && text.contains("damaged"),
        "{text}"
    );
}

#[test]
fn a_part_needing_an_unknown_required_chunk_is_refused() {
    let bytes = file(&[store(6, 4), store(6, 5)], None);
    let mut m = manifest(&bytes);
    let layer = json!({ "kind": "layer", "part": 1, "range": [0, 1], "filter": "words", "codec": "zstd", "offset": 64, "stored": 0, "raw": 0, "hash": "", "required": true });
    m["chunks"].as_array_mut().expect("chunks").push(layer);
    let scene = read_bytes(&with_manifest(&bytes, &m), Kept::None).expect("the file reads");
    assert!(scene.parts[0].is_ok(), "the other part opens");
    let err = scene.parts[1].as_ref().expect_err("refused");
    assert!(err.reason.contains("'layer'"), "{err}");
}

#[test]
fn a_file_cut_short_anywhere_is_refused_as_incomplete() {
    let bytes = file(&[store(5, 6)], Some(b"tiny picture"));
    for len in 0..bytes.len() {
        match read_bytes(&bytes[..len], Kept::None) {
            Err(SceneError::Incomplete { .. }) => {}
            Err(SceneError::NotScene) if len < 8 => {}
            other => panic!("cut at {len}: {other:?}"),
        }
    }
}

#[test]
fn a_damaged_manifest_refuses_the_file() {
    let bytes = file(&[store(5, 7)], None);
    let t = bytes.len() - 64;
    let at = u64::from_le_bytes(bytes[t..t + 8].try_into().expect("8 bytes")) as usize;
    let mut damaged = bytes.clone();
    damaged[at + 1] ^= 1;
    match read_bytes(&damaged, Kept::None) {
        Err(SceneError::Damaged { offset, reason }) => {
            assert_eq!(offset, at as u64);
            assert!(reason.contains("hash"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    let mut not_ours = bytes;
    not_ours[1] = b'X';
    assert_eq!(
        read_bytes(&not_ours, Kept::None).expect_err("refused"),
        SceneError::NotScene
    );
}
