//! Files no writer makes, built by hand (reviews 0005 and 0007): a chunk table or a manifest that
//! would make the reader allocate far more than the file holds is refused from its claimed size
//! alone, a Part kept beside the scene is read only when a saved reference names it, checked
//! like any chunk, and only as a Part file of one Part that keeps none of its own, and a frozen
//! Part has one set of frozen levels, which hold none of their own.

use gs_io::scene::{FileKind, Kept, SceneError, SceneIn, inspect_bytes, read_bytes};
use serde_json::{Value, json};

use crate::sample::{
    file, file_of, kept, manifest, store, with_chunk, with_manifest, with_manifest_json,
};

/// Reads `bytes` with the kept Part `ref:1/head` asked for.
fn read_head(bytes: &[u8]) -> SceneIn {
    let named = |_: &Value| vec!["ref:1/head".to_string()];
    read_bytes(bytes, Kept::Named(&named)).expect("the file reads")
}

/// Why the kept Part `ref:1/head` was refused.
fn head_refused(bytes: &[u8]) -> String {
    let scene = read_head(bytes);
    let [(name, read)] = <[_; 1]>::try_from(scene.references).expect("one kept Part");
    assert_eq!(name, "ref:1/head", "the kept Part asked for");
    read.expect_err("refused").to_string()
}

/// A scene keeping the Part `inner` holds as `ref:1/head`, its chunk added by hand.
fn keeping(inner: &[u8]) -> Vec<u8> {
    with_chunk(&file(&[store(5, 1)], None), kept("ref:1/head"), inner)
}

fn chunks(m: &mut Value) -> &mut Vec<Value> {
    m["chunks"].as_array_mut().expect("chunks")
}

/// Review 0005, M1. Each refusal comes from the chunk table, before a chunk is read: a second
/// Part made of the first one's chunks (the same bytes counted twice), a Part whose nodes lack
/// their topology (refused for that, not for the positions chunk whose hash is wrong, which
/// only reading would find), and a claim larger than a file this size may hold.
#[test]
fn a_crafted_chunk_table_is_refused_before_any_chunk_is_decoded() {
    let bytes = file(&[store(10, 1)], None);
    let m = manifest(&bytes);
    let damaged = |m: &Value| match read_bytes(&with_manifest(&bytes, m), Kept::None) {
        Err(SceneError::Damaged { reason, .. }) => reason,
        other => panic!("{other:?}"),
    };

    let mut shared = m.clone();
    let mut copy = shared["parts"][0].clone();
    copy["name"] = json!("copy");
    shared["parts"].as_array_mut().expect("parts").push(copy);
    let again: Vec<Value> = (m["chunks"].as_array().expect("chunks").iter())
        .filter(|c| c["part"] == 0)
        .cloned()
        .map(|mut c| {
            c["part"] = json!(1);
            c
        })
        .collect();
    chunks(&mut shared).extend(again);
    let reason = damaged(&shared);
    assert!(reason.contains("share bytes"), "{reason}");

    let mut gap = m.clone();
    chunks(&mut gap).retain(|c| c["kind"] != "topology");
    for c in chunks(&mut gap)
        .iter_mut()
        .filter(|c| c["kind"] == "positions")
    {
        c["hash"] = json!("0".repeat(32));
    }
    let scene = read_bytes(&with_manifest(&bytes, &gap), Kept::None).expect("the file reads");
    let err = scene.parts[0].as_ref().expect_err("refused");
    assert!(err.reason.contains("node 0 is missing a chunk"), "{err}");

    let mut huge = m.clone();
    let positions = chunks(&mut huge)
        .iter_mut()
        .find(|c| c["kind"] == "positions");
    positions.expect("a positions chunk")["raw"] = json!(1u64 << 40);
    let reason = damaged(&huge);
    assert!(reason.contains("its size may hold"), "{reason}");
}

/// Review 0007, M1: the manifest is charged against the same budget as the chunks before it is
/// unpacked, at what it may grow to once read (a two-byte `0,` becomes a 32-byte value), so a
/// few kilobytes of zstd cannot make a read hold gigabytes. Here the session section is 2.5
/// million zeros, 5 MB of JSON in a file of a few kilobytes: an open and an inspect both refuse
/// it, where before both read it as if nothing were wrong.
#[test]
fn a_manifest_larger_than_its_file_may_hold_is_refused_before_it_is_unpacked() {
    let bytes = file(&[store(3, 1)], None);
    let mut m = manifest(&bytes);
    m["session"] = json!("@@");
    let zeros = format!("[{}]", vec!["0"; 2_500_000].join(","));
    let text = serde_json::to_string(&m).expect("json");
    let bomb = with_manifest_json(&bytes, text.replace("\"@@\"", &zeros).as_bytes());
    assert!(bomb.len() < 64 << 10, "a small file: {} bytes", bomb.len());
    let refused = |read: Result<(), SceneError>| match read {
        Err(SceneError::Damaged { reason, .. }) => reason,
        other => panic!("not refused as damaged: {other:?}"),
    };
    let why = refused(read_bytes(&bomb, Kept::None).map(drop));
    assert!(
        why.contains("its manifest") && why.contains("its size may hold"),
        "{why}"
    );
    let why = refused(inspect_bytes(&bomb, false).map(drop));
    assert!(why.contains("its manifest"), "{why}");
}

/// Review 0005, M1: of the Parts a file keeps, only those a saved reference names are read;
/// the others are listed, not read and not dropped silently.
#[test]
fn only_the_kept_parts_a_saved_reference_names_are_read() {
    let (head, other) = (store(6, 2), store(6, 3));
    let refs = [("ref:1/head", &head), ("ref:9/x0", &other)];
    let scene = read_head(&file_of(FileKind::Scene, &[store(5, 1)], &refs));
    let [(name, read)] = <[_; 1]>::try_from(scene.references).expect("one kept Part read");
    assert_eq!(name, "ref:1/head");
    assert!(read.is_ok(), "{read:?}");
    assert_eq!(
        scene.skipped,
        ["kept Part 'ref:9/x0', which no saved reference names"]
    );
}

/// Review 0005, m2 and m3: a kept Part's chunk is checked against its hash and its codec as
/// every chunk is, so an open refuses what `file.inspect` with `verify` lists as damaged, and
/// two kept Parts under one name are refused rather than one silently winning.
#[test]
fn a_kept_part_is_checked_like_any_chunk_and_inspect_agrees_with_open() {
    let bytes = file_of(
        FileKind::Scene,
        &[store(5, 1)],
        &[("ref:1/head", &store(6, 2))],
    );
    let m = manifest(&bytes);
    let entry = (m["chunks"].as_array().expect("chunks").iter())
        .find(|c| c["kind"] == "reference")
        .expect("the kept Part's chunk")
        .clone();
    let at = entry["offset"].as_u64().expect("offset") as usize;
    let damaged = |bytes: &[u8]| inspect_bytes(bytes, true).expect("inspect").damaged;

    let mut flipped = bytes.clone();
    flipped[at + 50] ^= 0x5A;
    let why = head_refused(&flipped);
    assert!(why.contains("hash does not match"), "{why}");
    assert_eq!(damaged(&flipped).len(), 1, "inspect lists it too");

    let mut zstd = m.clone();
    for c in chunks(&mut zstd)
        .iter_mut()
        .filter(|c| c["kind"] == "reference")
    {
        c["codec"] = json!("zstd");
    }
    let zstd = with_manifest(&bytes, &zstd);
    let why = head_refused(&zstd);
    assert!(why.contains("unexpected codec zstd"), "{why}");
    assert_eq!(damaged(&zstd).len(), 1, "inspect lists it too");

    // 32 bytes of the header as the chunk, and the chunk cut to 32 bytes of its declared size.
    let edits = [
        (
            json!({ "offset": 0, "stored": 32, "raw": 32 }),
            "outside the file's chunks",
        ),
        (json!({ "stored": 32 }), "its two sizes differ"),
    ];
    for (edit, reason) in edits {
        let mut m = m.clone();
        for c in chunks(&mut m)
            .iter_mut()
            .filter(|c| c["kind"] == "reference")
        {
            for (key, value) in edit.as_object().expect("an edit") {
                c[key] = value.clone();
            }
        }
        let edited = with_manifest(&bytes, &m);
        let why = head_refused(&edited);
        assert!(why.contains(reason), "{edit}: {why}");
        assert_eq!(damaged(&edited).len(), 1, "{edit}: inspect lists it too");
    }

    let start = at;
    let end = start + entry["stored"].as_u64().expect("stored") as usize;
    let twice = with_chunk(&bytes, kept("ref:1/head"), &bytes[start..end]);
    let why = head_refused(&twice);
    assert!(why.contains("keeps 2 copies under this name"), "{why}");
}

/// Review 0005, T1: what a kept Part's chunk holds must be a Part file of exactly one Part that
/// keeps no Parts of its own, so a crafted file cannot nest kept Parts or slip in a scene.
#[test]
fn a_kept_part_is_a_part_file_of_one_part_keeping_none() {
    let (a, b) = (|| store(6, 2), store(6, 3));
    let nested = file_of(FileKind::Parts, &[a()], &[("ref:2/deeper", &b)]);
    let why = head_refused(&keeping(&nested));
    assert!(why.contains("keeps Parts of its own"), "{why}");

    let scene = file_of(FileKind::Scene, &[a()], &[]);
    let why = head_refused(&keeping(&scene));
    assert!(
        why.contains("does not hold a Part file of one Part"),
        "{why}"
    );

    let two = file_of(FileKind::Parts, &[a(), b], &[]);
    let why = head_refused(&keeping(&two));
    assert!(
        why.contains("does not hold a Part file of one Part"),
        "{why}"
    );

    let one = file_of(FileKind::Parts, &[a()], &[]);
    assert!(
        read_head(&keeping(&one)).references[0].1.is_ok(),
        "one Part reads"
    );
}

/// The entry of part 0's frozen levels, for [`with_chunk`].
fn frozen_levels() -> Value {
    json!({ "kind": "frozen_levels", "part": 0, "filter": "bytes", "codec": "stored", "required": true })
}

/// Review 0007, T1 (decision 0030, section 7): a Part's frozen levels are a Part file that
/// holds no frozen levels of its own, and a Part has one set of them. No writer makes either
/// (`write` refuses the first), so the chunks are added by hand; each refuses its Part by name
/// and leaves the other Part, and one set of plain frozen levels added the same way reads.
#[test]
fn a_frozen_part_has_one_set_of_frozen_levels_holding_none_of_their_own() {
    let one = file_of(FileKind::Parts, &[store(6, 2)], &[]);
    let frozen = with_chunk(
        &file(&[store(5, 1), store(4, 3)], None),
        frozen_levels(),
        &one,
    );
    let read = |bytes: &[u8]| read_bytes(bytes, Kept::None).expect("the file reads").parts;
    let parts = read(&frozen);
    assert!(
        parts[0].as_ref().is_ok_and(|p| p.frozen.is_some()),
        "one set reads: {:?}",
        parts[0].as_ref().err()
    );
    let deeper = with_chunk(&one, frozen_levels(), &one);
    let nested = with_chunk(
        &file(&[store(5, 1), store(4, 3)], None),
        frozen_levels(),
        &deeper,
    );
    let twice = with_chunk(&frozen, frozen_levels(), &one);
    for (bytes, reason) in [
        (nested, "keeps frozen levels of its own"),
        (twice, "has 2 sets of frozen levels"),
    ] {
        let mut parts = read(&bytes);
        assert!(parts[1].is_ok(), "{reason}: the other part reads");
        let why = parts.remove(0).expect_err("refused").to_string();
        assert!(why.contains("'part-0'") && why.contains(reason), "{why}");
    }
}
