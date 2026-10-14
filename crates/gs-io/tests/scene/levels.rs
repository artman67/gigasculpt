//! A Part with detail levels in a scene file (decision 0016, section 15): its levels read back
//! exactly as gs-core stored them, beside a top that rebuilds as the same store; the file needs
//! a version 2 reader while files without levels still need only version 1; and a level chunk
//! that is missing or does not belong refuses only its Part. A frozen Part's levels (decision
//! 0030, section 7) read back with it, need a version 3 reader, and a damaged copy of them
//! refuses only that Part; the writer refuses frozen levels it would have to nest deeper. A
//! level's own UVs (decision 0014, section 14) read back with it and need a version 4 reader.

use gs_core::{Levels, MeshStore, Placement};
use gs_io::scene::{
    FORMAT, FileKind, Kept, NumberingIn, NumberingOut, PartIn, PartOut, SceneOut, inspect_bytes,
    read_bytes, write,
};
use serde_json::Value;

use crate::sample::{file, file_with, levels, manifest, store, with_manifest};

/// Part `i` of `bytes`, read.
fn part(bytes: &[u8], i: usize) -> Result<PartIn, String> {
    let mut scene = read_bytes(bytes, Kept::None).expect("the file reads");
    scene.parts.remove(i).map_err(|e| e.to_string())
}

#[test]
fn a_part_with_levels_reads_back_its_levels_and_reopens_as_the_same_stack() {
    let stack = levels(6, 2);
    let bytes = file_with(&[store(6, 1)], &[stack.share()], None);
    assert!(
        file_with(&[store(6, 1)], &[stack.share()], None) == bytes,
        "the same stack writes the same bytes"
    );
    let scene = read_bytes(&bytes, Kept::None).expect("the file reads");
    assert_eq!(
        (scene.header.format, scene.header.oldest_reader),
        (FORMAT, 2)
    );
    let mut parts = scene.parts.into_iter();
    let plain = parts.next().expect("part 0").expect("part 0 reads");
    assert!(plain.levels.is_none(), "a plain part has no levels");
    let p = parts.next().expect("part 1").expect("part 1 reads");
    assert!(
        p.levels.as_ref() == Some(&stack.stored()),
        "the levels as stored"
    );
    let top = MeshStore::from_exact(p.config, p.attrs, p.nodes).expect("a store");
    assert!(top.same_content(stack.top()), "the top, column for column");
    let NumberingIn::Given {
        vertices,
        faces,
        first_corner,
    } = p.numbering
    else {
        panic!("the top's numbering is given");
    };
    let placed = Placement {
        vertices,
        faces,
        first_corner,
    };
    assert_eq!(placed, stack.level_numbering(2).placement());
    let stored = p.levels.expect("levels");
    let reopened = Levels::from_file(top, placed, stored).expect("the stack");
    assert_eq!((reopened.count(), reopened.active()), (3, 1));
    assert!(reopened.pending(), "the edits are still pending");
    assert!(reopened.stored() == stack.stored(), "the same stack");
}

/// Smooth UVs (decision 0014, section 14): the UVs a lower level keeps read back bit for bit,
/// and only a file holding them needs a version 4 reader.
#[test]
fn a_levels_own_uvs_read_back_and_need_version_4() {
    let stack = levels(6, 2);
    let mut stored = stack.stored();
    let faces = stored.lower[1].numbering.faces.len();
    stored.lower[1].uvs = (0..faces)
        .map(|f| std::array::from_fn(|i| [f as f32 * 0.25, i as f32 - 0.5]))
        .collect();
    let placed = stack.level_numbering(stack.top_level()).placement();
    let scene = SceneOut {
        kind: FileKind::Scene,
        writer: "gigasculpt test".into(),
        preview: None,
        scene: Value::Null,
        session: Value::Null,
        parts: vec![PartOut {
            name: "smoothed",
            props: Value::Null,
            store: stack.top(),
            numbering: NumberingOut::Given {
                vertices: &placed.vertices,
                faces: &placed.faces,
                first_corner: &placed.first_corner,
            },
            levels: Some(&stored),
            frozen: None,
        }],
        references: Vec::new(),
    };
    let mut bytes = Vec::new();
    write(&mut bytes, &scene).expect("write");
    let read = read_bytes(&bytes, Kept::None).expect("the file reads");
    assert_eq!(read.header.oldest_reader, 4);
    let p = part(&bytes, 0).expect("the part reads");
    assert!(p.levels == Some(stored), "the levels with their own UVs");
}

#[test]
fn a_file_without_levels_still_needs_only_version_1() {
    let scene = read_bytes(&file(&[store(5, 3)], None), Kept::None).expect("reads");
    assert_eq!(scene.header.oldest_reader, 1);
}

#[test]
fn inspect_shows_the_levels() {
    let bytes = file_with(&[], &[levels(6, 4)], None);
    let seen = inspect_bytes(&bytes, true).expect("inspect");
    assert!(seen.damaged.is_empty(), "{:?}", seen.damaged);
    assert_eq!((seen.parts[0].levels, seen.parts[0].active_level), (3, 1));
    assert!(seen.chunks.iter().any(|c| c.kind == "level"));
}

/// The level chunks of `m`, by their place in the chunk table.
fn level_chunks(m: &Value) -> Vec<usize> {
    let chunks = m["chunks"].as_array().expect("chunks");
    (0..chunks.len())
        .filter(|&i| chunks[i]["kind"] == "level")
        .collect()
}

#[test]
fn a_missing_level_chunk_refuses_only_its_part() {
    let bytes = file_with(&[store(5, 5)], &[levels(6, 6)], None);
    let m = manifest(&bytes);
    let found = level_chunks(&m);
    // The first chunk of an array whose length the entry gives. Pending columns are listed
    // only by their chunks, as attribute columns are.
    for name in ["base_faces", "creases", "vertices", "faces", "first_corner"] {
        let mut gone = m.clone();
        let chunks = gone["chunks"].as_array_mut().expect("chunks");
        let i = *found
            .iter()
            .find(|&&i| chunks[i]["name"] == name)
            .expect("a chunk of that array");
        chunks.remove(i);
        let bytes = with_manifest(&bytes, &gone);
        assert!(part(&bytes, 0).is_ok(), "{name}: the other part opens");
        let err = part(&bytes, 1).expect_err("refused");
        assert!(
            err.contains("incomplete") || err.contains("gap"),
            "{name}: {err}"
        );
    }
}

#[test]
fn a_level_chunk_on_a_part_without_levels_refuses_that_part() {
    let bytes = file_with(&[store(5, 7)], &[levels(6, 8)], None);
    let mut m = manifest(&bytes);
    let i = level_chunks(&m)[0];
    m["chunks"][i]["part"] = Value::from(0);
    let bytes = with_manifest(&bytes, &m);
    let err = part(&bytes, 0).expect_err("refused");
    assert!(err.contains("has no detail levels"), "{err}");
}

#[test]
fn levels_bigger_than_the_top_are_refused_before_reading() {
    let bytes = file_with(&[], &[levels(6, 9)], None);
    let mut m = manifest(&bytes);
    m["parts"][0]["levels"]["lower"][1][0][0] = Value::from(40);
    m["parts"][0]["levels"]["lower"][1]
        .as_array_mut()
        .expect("table")
        .extend((0..100_000).map(|_| serde_json::json!([40, 40])));
    let err = part(&with_manifest(&bytes, &m), 0).expect_err("refused");
    assert!(err.contains("bigger than the top"), "{err}");
}

/// A file of two plain Parts, the first frozen with `stack` set aside.
fn frozen_file(base: &MeshStore, other: &MeshStore, stack: &Levels) -> Vec<u8> {
    let (placed, stored) = (
        stack.level_numbering(stack.top_level()).placement(),
        stack.stored(),
    );
    let plain = |name, store| PartOut {
        name,
        props: Value::Null,
        store,
        numbering: NumberingOut::Spatial,
        levels: None,
        frozen: None,
    };
    let record = PartOut {
        numbering: NumberingOut::Given {
            vertices: &placed.vertices,
            faces: &placed.faces,
            first_corner: &placed.first_corner,
        },
        levels: Some(&stored),
        ..plain("frozen", stack.top())
    };
    let scene = SceneOut {
        kind: FileKind::Scene,
        writer: "gigasculpt test".into(),
        preview: None,
        scene: Value::Null,
        session: Value::Null,
        parts: vec![
            PartOut {
                frozen: Some(Box::new(record)),
                ..plain("frozen", base)
            },
            plain("other", other),
        ],
        references: Vec::new(),
    };
    let mut out = Vec::new();
    write(&mut out, &scene).expect("write");
    out
}

#[test]
fn a_frozen_parts_levels_read_back_with_it_and_damage_refuses_only_that_part() {
    let (base, other, stack) = (store(4, 1), store(5, 2), levels(6, 3));
    let bytes = frozen_file(&base, &other, &stack);
    let scene = read_bytes(&bytes, Kept::None).expect("the file reads");
    assert_eq!(scene.header.oldest_reader, 3);
    let p = part(&bytes, 0).expect("the frozen part reads");
    let top = MeshStore::from_exact(p.config, p.attrs, p.nodes).expect("a store");
    assert!(
        top.same_content(&base) && p.levels.is_none(),
        "its own mesh, one level"
    );
    let record = p.frozen.expect("its frozen levels");
    assert!(record.levels.as_ref() == Some(&stack.stored()), "as stored");
    assert!(record.frozen.is_none());
    let m = manifest(&bytes);
    let chunk = (m["chunks"].as_array().expect("chunks").iter())
        .find(|c| c["kind"] == "frozen_levels")
        .expect("a frozen_levels chunk");
    assert_eq!(
        (&chunk["part"], &chunk["required"]),
        (&Value::from(0), &Value::Bool(true))
    );
    let mut damaged = bytes.clone();
    let at = chunk["offset"].as_u64().expect("offset") as usize + 100;
    damaged[at] ^= 0xff;
    let err = part(&damaged, 0).expect_err("refused");
    assert!(
        err.contains("'frozen'") && err.contains("frozen levels"),
        "{err}"
    );
    assert!(part(&damaged, 1).is_ok(), "the other part opens");
}

/// Review 0007, S3: a frozen Part's levels and a kept Part are written as Part files that hold
/// no frozen levels of their own (decision 0030, section 7), so handing the writer one that
/// does is an error naming it, not levels dropped without a word.
#[test]
fn the_writer_refuses_frozen_levels_inside_frozen_levels_or_a_kept_part() {
    let (base, record, deeper) = (store(4, 1), store(5, 2), store(6, 3));
    let plain = |name, store| PartOut {
        name,
        props: Value::Null,
        store,
        numbering: NumberingOut::Spatial,
        levels: None,
        frozen: None,
    };
    let holding = || PartOut {
        frozen: Some(Box::new(plain("deeper", &deeper))),
        ..plain("record", &record)
    };
    let scene = |parts, references| SceneOut {
        kind: FileKind::Scene,
        writer: "gigasculpt test".into(),
        preview: None,
        scene: Value::Null,
        session: Value::Null,
        parts,
        references,
    };
    let nested = PartOut {
        frozen: Some(Box::new(holding())),
        ..plain("frozen", &base)
    };
    let kept = vec![("ref:1/head".to_string(), holding())];
    for (what, file) in [
        ("'frozen'", scene(vec![nested], Vec::new())),
        ("'ref:1/head'", scene(vec![plain("main", &base)], kept)),
    ] {
        let err = write(&mut Vec::new(), &file).expect_err("refused");
        let err = err.to_string();
        assert!(err.contains(what) && err.contains("frozen levels"), "{err}");
    }
}
