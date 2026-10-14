//! What a written scene file reads back as: every Part's columns bit for bit (a store rebuilt
//! from them is the same store), its numbering and properties exactly, the scene and session
//! sections, the preview; and the same scene always writes the same bytes.

use gs_core::MeshStore;
use gs_io::scene::{
    FORMAT, FileKind, Kept, NumberingIn, NumberingOut, PartOut, SceneIn, SceneOut, inspect_bytes,
    read_bytes, write,
};
use serde_json::json;

use crate::sample::{file, manifest, numbering, props, store, with_manifest};

fn read(bytes: &[u8]) -> SceneIn {
    read_bytes(bytes, Kept::None).expect("the file reads")
}

#[test]
fn every_part_reads_back_as_the_same_store() {
    let stores = [store(14, 1), store(9, 2)];
    let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
    let scene = read(&file(&stores, Some(&png)));
    assert_eq!(scene.header.kind, FileKind::Scene);
    assert_eq!(
        (scene.header.format, scene.header.oldest_reader),
        (FORMAT, 1)
    );
    assert_eq!(
        scene.preview.as_deref(),
        Some(&png[..]),
        "the preview, as given"
    );
    assert_eq!(scene.session, json!({ "seed": 7, "counter": 42 }));
    assert_eq!(scene.scene, json!({ "id": "0123456789abcdef" }));
    assert!(
        scene.skipped.is_empty(),
        "nothing unknown: {:?}",
        scene.skipped
    );
    for (i, (part, original)) in scene.parts.into_iter().zip(&stores).enumerate() {
        let part = part.expect("the part reads");
        assert_eq!(part.name, format!("part-{i}"));
        assert_eq!(part.props, props(i as u32), "properties bit for bit");
        let expected = if i == 0 {
            NumberingIn::Spatial
        } else {
            let (vertices, faces, first_corner) = numbering(original);
            NumberingIn::Given {
                vertices,
                faces,
                first_corner,
            }
        };
        assert_eq!(part.numbering, expected, "the numbering");
        let rebuilt = MeshStore::from_exact(part.config, part.attrs, part.nodes).expect("a store");
        assert!(rebuilt.same_content(original), "part {i}: the same store");
    }
}

#[test]
fn the_same_scene_writes_the_same_bytes() {
    let stores = [store(12, 3), store(6, 4)];
    let (a, b) = (file(&stores, None), file(&stores, None));
    assert!(a == b, "two writes of one scene are byte-identical");
    let shared = [stores[0].share(), stores[1].share()];
    assert!(
        file(&shared, None) == a,
        "and so are writes of a shared copy"
    );
}

#[test]
fn a_file_for_a_newer_reader_is_refused_naming_the_version() {
    let mut bytes = file(&[store(4, 5)], None);
    let needs = FORMAT + 1;
    bytes[12..16].copy_from_slice(&needs.to_le_bytes());
    let err = read_bytes(&bytes, Kept::None).expect_err("refused");
    assert_eq!(err, gs_io::scene::SceneError::TooNew { needs });
    assert!(
        err.to_string().contains(&format!("format {needs}")),
        "{err}"
    );
}

#[test]
fn a_newer_writers_optional_data_is_skipped_and_listed() {
    let bytes = file(&[store(6, 6)], None);
    let mut m = manifest(&bytes);
    m["cameras"] = json!([{ "name": "front" }]);
    let extra = json!({ "kind": "morph", "part": 0, "range": [0, 1], "filter": "words", "codec": "zstd", "offset": 64, "stored": 0, "raw": 0, "hash": "", "required": false });
    m["chunks"].as_array_mut().expect("chunks").push(extra);
    let newer = with_manifest(&bytes, &m);
    let scene = read(&newer);
    assert!(scene.parts[0].is_ok(), "the part still reads");
    assert_eq!(scene.skipped, ["manifest field 'cameras'", "morph chunk"]);
    let seen = inspect_bytes(&newer, false).expect("inspect");
    assert_eq!(seen.skipped, scene.skipped, "inspect lists the same");
}

/// A Part kept beside the scene (a Reference View's frozen copy, decision 0024 section 8) reads
/// back under its name and is never one of the scene's Parts; a reader that does not know the
/// kind, as an older one, skips it and reads the scene as it is.
#[test]
fn a_part_kept_beside_the_scene_reads_back_and_an_older_reader_skips_it() {
    let (scene_part, kept) = (store(5, 1), store(7, 2));
    let (v, f, c) = numbering(&kept);
    let part = |name, store| PartOut {
        name,
        props: props(0),
        store,
        numbering: NumberingOut::Spatial,
        levels: None,
        frozen: None,
    };
    let out = SceneOut {
        kind: FileKind::Scene,
        writer: "gigasculpt test".into(),
        preview: None,
        scene: json!({}),
        session: json!({}),
        parts: vec![part("head", &scene_part)],
        references: vec![(
            "ref:1/head".into(),
            PartOut {
                numbering: NumberingOut::Given {
                    vertices: &v,
                    faces: &f,
                    first_corner: &c,
                },
                ..part("head", &kept)
            },
        )],
    };
    let mut bytes = Vec::new();
    write(&mut bytes, &out).expect("write");
    let named = |_: &serde_json::Value| vec!["ref:1/head".to_string()];
    let scene = read_bytes(&bytes, Kept::Named(&named)).expect("the file reads");
    assert_eq!(scene.parts.len(), 1, "the scene's Part alone");
    let [(name, kept_in)] = <[_; 1]>::try_from(scene.references).expect("one kept Part");
    assert_eq!(name, "ref:1/head");
    let kept_in = kept_in.expect("it reads");
    let given = NumberingIn::Given {
        vertices: v.clone(),
        faces: f.clone(),
        first_corner: c.clone(),
    };
    assert_eq!(kept_in.numbering, given);
    let rebuilt = MeshStore::from_exact(kept_in.config, kept_in.attrs, kept_in.nodes);
    assert!(
        rebuilt.expect("a store").same_content(&kept),
        "the same store"
    );
    let seen = inspect_bytes(&bytes, true).expect("inspect");
    assert_eq!(seen.damaged, Vec::<String>::new(), "its hash verifies");

    let mut m = manifest(&bytes);
    for chunk in m["chunks"].as_array_mut().expect("chunks") {
        if chunk["kind"] == "reference" {
            chunk["kind"] = json!("kept-elsewhere");
        }
    }
    let older = read(&with_manifest(&bytes, &m));
    assert_eq!(older.skipped, ["kept-elsewhere chunk"]);
    assert!(older.references.is_empty(), "nothing kept");
    let head = older
        .parts
        .into_iter()
        .next()
        .expect("a part")
        .expect("it reads");
    let rebuilt = MeshStore::from_exact(head.config, head.attrs, head.nodes);
    assert!(rebuilt.expect("a store").same_content(&scene_part));
}

#[test]
fn inspect_describes_the_file_without_reading_parts() {
    let stores = [store(10, 7)];
    let bytes = file(&stores, Some(b"picture"));
    let seen = inspect_bytes(&bytes, true).expect("inspect");
    assert_eq!(seen.bytes, bytes.len() as u64);
    assert_eq!(seen.preview, Some(7));
    assert!(
        seen.damaged.is_empty(),
        "every chunk verifies: {:?}",
        seen.damaged
    );
    let part = &seen.parts[0];
    assert_eq!(part.nodes, stores[0].node_count() as u64);
    assert!(part.free_vertex_slots > 0, "the edits left free slots");
    let kinds: Vec<&str> = seen.chunks.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["attribute", "free", "positions", "preview", "topology"]
    );
    let positions = &seen.chunks[2];
    assert!(positions.stored < positions.raw, "positions compress");
}
