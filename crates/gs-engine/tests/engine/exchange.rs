//! The Blender exchange's engine half (decision 0032), without Blender: `exchange.read` of the
//! folder `exchange.write` filled is what the add-on sends back when nothing changed in Blender,
//! and editing those files stands in for edits made there. The round trips through headless
//! Blender are in `tier2`.

use std::fs;
use std::path::{Path, PathBuf};

use gs_protocol::{AttrValues, ErrorCode};
use gs_testkit::{Session, fixtures, text};
use serde_json::{Value, json};

use crate::support::agent;

mod paint;
pub mod tier2;

/// A fresh, empty folder for one test.
pub fn folder(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("exchange-{}-{name}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    dir
}

/// `path` as a call argument.
pub fn arg(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Part "c": a cube with three levels and creases on its base, three face groups, a painted
/// mask, the faces of group 3 hidden, and a place that is not the identity.
pub fn sculpted() -> Session {
    let mut s = agent();
    let cube = fixtures::cube(2);
    s.add_part("c", &cube);
    // The place first: with a mask or hidden faces, a move goes into the points instead.
    s.ok(
        "transform.apply",
        json!({ "op": { "move": [0.5, -1.0, 2.0] } }),
    );
    let turn = json!({ "rotate": { "axis": "y", "degrees": 30.0 } });
    s.ok("transform.apply", json!({ "op": turn }));
    let f = cube.faces[0];
    let creases: Vec<Value> = (0..4)
        .map(|i| json!({ "a": f[i], "b": f[(i + 1) % 4], "sharpness": 2 }))
        .collect();
    s.ok("level.subdivide", json!({ "creases": creases }));
    s.ok("level.subdivide", json!({}));
    for (id, faces) in [(1, [0, 1, 2]), (2, [40, 41, 42]), (3, [90, 91, 92])] {
        let select = json!({ "by": "faces", "indexes": faces });
        s.ok("group.assign", json!({ "select": select, "id": id }));
    }
    let pen = json!({
        "brush": "Mask Pen",
        "samples": [{ "position": [0.0, 0.0, 1.0] }],
        "size": "40%",
    });
    s.ok("brush.stroke", pen);
    let hide = json!({ "select": { "by": "groups", "ids": [3] } });
    s.ok("visibility.hide", hide);
    s
}

/// `sculpted`'s part "c" filled half way to a blue: its colors vary with the mask, and the
/// points of its hidden faces stay white.
pub fn painted() -> Session {
    let mut s = sculpted();
    let fill = json!({ "color": "#3366cc", "strength": 0.5, "seed": 7 });
    s.ok("paint.fill", fill);
    s
}

/// The manifest in `dir`.
pub fn manifest(dir: &Path) -> Value {
    let path = dir.join("manifest.json");
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Writes `manifest` into `dir`.
fn put_manifest(dir: &Path, manifest: &Value) {
    let path = dir.join("manifest.json");
    let bytes = serde_json::to_vec_pretty(manifest).unwrap_or_else(|e| panic!("{e}"));
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The file of the first part's column Blender calls `blender`.
fn column_file(dir: &Path, blender: &str) -> PathBuf {
    let m = manifest(dir);
    let columns = m["parts"][0]["columns"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let found = columns.iter().find(|c| c["blender"] == blender);
    let file = found.unwrap_or_else(|| panic!("no column {blender} in {m}"));
    dir.join(text(&file["file"]))
}

/// The groups of part `name`, face by face.
fn groups(s: &mut Session, name: &str) -> Vec<u32> {
    match s.mesh(name).attributes.get("group").map(|a| &a.values) {
        Some(AttrValues::U32(g)) => g.clone(),
        other => panic!("part {name} has no groups: {other:?}"),
    }
}

#[test]
fn a_write_read_back_unchanged_leaves_the_scene_exactly_as_it_was() {
    let mut s = sculpted();
    let before = s.hash();
    let steps = s.history()["undoable"].clone();
    for form in ["mesh", "levels"] {
        let dir = folder(&format!("same-{form}"));
        let written = s.query(
            "exchange.write",
            json!({ "folder": arg(&dir), "form": form }),
        );
        let sent = &written["parts"][0];
        assert_eq!(
            (sent["form"].clone(), sent["levels"].clone()),
            (json!(form), json!(3))
        );
        let m = manifest(&dir);
        let count = |key: &str| m["parts"][0][key].as_array().map_or(0, Vec::len);
        let want = if form == "levels" { (4, 3) } else { (0, 6) };
        assert_eq!(
            (count("creases"), count("columns")),
            want,
            "{form}: creases, columns"
        );
        let read = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
        assert_eq!(
            read.result["parts"][0]["how"], "unchanged",
            "{form}: {}",
            read.result
        );
        assert_eq!(
            s.hash(),
            before,
            "{form}: a return of what was sent changed the scene"
        );
    }
    let after = s.history()["undoable"].clone();
    assert_eq!(
        after, steps,
        "a read that changed nothing made an undo step"
    );
}

#[test]
fn moved_points_and_a_new_place_come_back_in_place_as_one_undo_step() {
    let mut s = sculpted();
    let before = s.hash();
    let (had_groups, had_levels) = (groups(&mut s, "c"), s.query("level.describe", json!({})));
    let dir = folder("moved");
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    let sent = manifest(&dir)["parts"][0]["transform"].clone();
    assert_ne!(
        sent["rotation"],
        json!([1.0, 0.0, 0.0, 0.0]),
        "the part is turned"
    );
    // In Blender: point 0 goes up (Blender's z) and the object moves along x.
    let positions = column_file(&dir, "position");
    let mut bytes = fs::read(&positions).unwrap_or_else(|e| panic!("{e}"));
    let z = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) + 0.125;
    bytes[8..12].copy_from_slice(&z.to_le_bytes());
    fs::write(&positions, &bytes).unwrap_or_else(|e| panic!("{e}"));
    let mut m = manifest(&dir);
    let x = sent["location"][0].as_f64().unwrap_or(f64::NAN) + 1.0;
    m["parts"][0]["transform"]["location"][0] = json!(x);
    put_manifest(&dir, &m);

    let read = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(
        read.result["parts"][0]["how"], "in_place",
        "{}",
        read.result
    );
    assert_eq!(groups(&mut s, "c"), had_groups, "the groups stay");
    let levels = s.query("level.describe", json!({}));
    assert_eq!(
        levels["count"], had_levels["count"],
        "the part keeps its levels"
    );
    // Sent again, the part goes as it came back, bit for bit.
    let again = folder("moved-again");
    s.ok("exchange.write", json!({ "folder": arg(&again) }));
    let resent = fs::read(column_file(&again, "position")).unwrap_or_else(|e| panic!("{e}"));
    assert!(resent == bytes, "the points came back as Blender had them");
    let place = manifest(&again)["parts"][0]["transform"].clone();
    assert_eq!(place, m["parts"][0]["transform"], "the new place came back");

    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo takes the whole read back");
}

/// A folder as the add-on writes it for one object: `points` in Blender's axes and one
/// polygon per entry of `polygons`.
fn object(dir: &Path, id: Option<&str>, points: &[[f32; 3]], polygons: &[&[i32]]) {
    let floats: Vec<u8> = points
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let corners: Vec<i32> = polygons.iter().flat_map(|p| p.iter().copied()).collect();
    let starts: Vec<i32> = (polygons.iter())
        .scan(0, |at, p| {
            let start = *at;
            *at += i32::try_from(p.len()).unwrap_or_else(|e| panic!("{e}"));
            Some(start)
        })
        .collect();
    let ints = |v: &[i32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let files = [
        ("0-position.bin", floats),
        ("0-corner_vert.bin", ints(&corners)),
        ("0-loop_start.bin", ints(&starts)),
    ];
    for (name, bytes) in files {
        fs::write(dir.join(name), bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
    let column = |gs: &str, blender: &str, domain: &str, ty: &str, width: u32, file: &str| {
        json!({ "gs": gs, "blender": blender, "domain": domain, "type": ty, "width": width,
                "file": file })
    };
    let entry = json!({
        "id": id,
        "name": "Ngon",
        "form": "mesh",
        "levels": 1,
        "transform": { "location": [0.0, 0.0, 0.0], "rotation": [1.0, 0.0, 0.0, 0.0], "scale": 1.0 },
        "points": points.len(),
        "faces": polygons.len(),
        "corners": corners.len(),
        "columns": [
            column("positions", "position", "point", "f32", 3, "0-position.bin"),
            column("faces", ".corner_vert", "corner", "i32", 1, "0-corner_vert.bin"),
            column("faces", "loop_start", "face", "i32", 1, "0-loop_start.bin"),
        ],
    });
    let m = json!({ "format": "gigasculpt-exchange", "version": 1, "writer": "test",
                    "parts": [entry] });
    put_manifest(dir, &m);
}

#[test]
fn a_pentagon_and_a_loose_point_come_back_as_a_new_part_split_into_triangles() {
    let mut s = agent();
    s.ok("primitive.cube", json!({}));
    let cube = s.hash();
    let id = s.query("part.list", json!({}))["parts"][0]["id"].to_string();
    let dir = folder("ngon");
    let pentagon = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.5, 0.0, 1.0],
        [0.5, 0.0, 1.5],
        [-0.5, 0.0, 1.0],
        [3.0, 3.0, 3.0],
    ];
    object(&dir, Some(&id), &pentagon, &[&[0, 1, 2, 3, 4]]);
    let read = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    let came = &read.result["parts"][0];
    assert_eq!(
        (came["how"].clone(), came["split"].clone()),
        (json!("new"), json!(1))
    );
    let note = text(&came["note"]);
    assert!(
        note.contains("is a new part and 'cube' is unchanged"),
        "{note}"
    );
    let mesh = s.mesh(&text(&came["part"]));
    assert_eq!(mesh.positions.len(), 6, "the loose point stays");
    assert_eq!(mesh.faces.len(), 3, "a pentagon is three triangles");
    // The fan starts at the corner lowest in the scene's y (Blender's z), then z, then |x|.
    assert!(
        mesh.faces.iter().all(|f| f.contains(&0)),
        "{:?}",
        mesh.faces
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), cube);
}

#[test]
fn a_folder_changed_after_its_hash_was_pinned_is_refused() {
    let mut s = sculpted();
    let dir = folder("pinned");
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    let err = s.err(
        "exchange.read",
        json!({ "folder": arg(&dir), "hash": "00" }),
    );
    assert_eq!(err.code, ErrorCode::Precondition, "{}", err.message);
    assert!(err.message.contains("changed"), "{}", err.message);
    // A column file of the wrong length is refused whole, naming the object.
    let mask = column_file(&dir, ".sculpt_mask");
    let mut bytes = fs::read(&mask).unwrap_or_else(|e| panic!("{e}"));
    bytes.truncate(bytes.len() - 4);
    fs::write(&mask, bytes).unwrap_or_else(|e| panic!("{e}"));
    let err = s.err("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(err.code, ErrorCode::Precondition, "{}", err.message);
    assert!(err.message.contains("the object 'c'"), "{}", err.message);
}

#[test]
fn a_levels_form_top_in_another_numbering_comes_back_in_place() {
    let mut s = sculpted();
    let before = s.hash();
    let dir = folder("renumbered");
    let form = json!({ "folder": arg(&dir), "form": "levels" });
    s.ok("exchange.write", form);
    // Blender keeps the base's points first and in order, and numbers the rest its own way.
    let base = fixtures::cube(2).positions.len();
    let read = |p: &Path| fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let (points, corners) = (
        column_file(&dir, "position"),
        column_file(&dir, ".corner_vert"),
    );
    let xyz = read(&points);
    let xyz = xyz.as_chunks::<12>().0;
    let n = xyz.len();
    let new = |v: usize| if v < base { v } else { base + n - 1 - v };
    let mut moved = vec![[0u8; 12]; n];
    for (v, p) in xyz.iter().enumerate() {
        moved[new(v)] = *p;
    }
    fs::write(&points, moved.concat()).unwrap_or_else(|e| panic!("{e}"));
    // The faces reversed, each starting at its second corner.
    let quads = read(&corners);
    let mut turned = Vec::with_capacity(quads.len());
    for q in quads.as_chunks::<16>().0.iter().rev() {
        let q = q.as_chunks::<4>().0;
        for k in 1..=4 {
            let v = usize::try_from(u32::from_le_bytes(q[k % 4])).unwrap_or(usize::MAX);
            turned.extend(u32::try_from(new(v)).unwrap_or(u32::MAX).to_le_bytes());
        }
    }
    fs::write(&corners, turned).unwrap_or_else(|e| panic!("{e}"));
    let came = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(
        came.result["parts"][0]["how"], "unchanged",
        "{}",
        came.result
    );
    assert_eq!(s.hash(), before, "the same points, numbered another way");
}
