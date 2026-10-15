//! `file.import`: round trips, undo, replay with the file pinned, the import options, and
//! files the import refuses.

use std::path::{Path, PathBuf};

use gs_engine::EngineConfig;
use gs_protocol::{LogRecord, Role, ScriptLine};
use gs_testkit::{Session, code, failure, success};
use serde_json::{Value, json};

use crate::{NO, at, bits, folder, names, only, painted, parts, put, with_painted};

/// Exports part `p` to `name` in a new folder and imports it back as `back`.
fn round_trip(name: &str, export: &Value) -> (Session, Value) {
    let mut s = with_painted();
    let path = at(&folder(), name);
    let mut args = json!({ "path": path, "part": "p" });
    args.as_object_mut()
        .unwrap()
        .extend(export.as_object().unwrap().clone());
    s.ok("file.export", args);
    let result = s.query("file.import", json!({ "path": path, "name": "back" }));
    (s, result)
}

#[test]
fn every_format_round_trips_through_the_commands() {
    let (mut s, result) = round_trip("p.obj", &json!({}));
    assert_eq!(
        bits(&s.mesh("back")),
        bits(&painted()),
        "OBJ keeps everything"
    );
    assert_eq!(parts(&result)[0]["export_scale"], 1.0);
    let (mut s2, _) = round_trip("p.ply", &json!({ "ascii": true }));
    assert_eq!(
        bits(&s2.mesh("back")),
        bits(&only(&painted(), &["color"])),
        "PLY keeps colors"
    );
    let (mut s3, result) = round_trip("p.stl", &json!({}));
    let back = s3.mesh("back");
    assert_eq!(
        back.positions,
        painted().positions,
        "the merged corners come back in order"
    );
    assert_eq!(
        back.faces,
        [
            [0, 1, 2, NO],
            [0, 2, 3, NO],
            [1, 4, 5, NO],
            [1, 5, 2, NO],
            [4, 6, 5, NO]
        ]
    );
    assert!(
        text(&result["notes"]).contains("merged 8 vertices"),
        "{result}"
    );
    assert_eq!(s.mesh("p").positions, painted().positions);
}

fn text(value: &Value) -> String {
    value.to_string()
}

#[test]
fn an_stl_cube_imports_as_eight_welded_points() {
    let cube = cube();
    let mut s = Session::new();
    s.ok("part.create", json!({ "name": "cube", "mesh": cube }));
    let path = at(&folder(), "cube.stl");
    s.ok("file.export", json!({ "path": path }));
    let bytes = std::fs::metadata(&path).unwrap().len();
    assert_eq!(bytes, 84 + 12 * 50, "12 triangles, 36 points");
    let welded = s.query("file.import", json!({ "path": path }));
    assert_eq!(parts(&welded)[0]["vertices"], 8);
    assert_eq!(parts(&welded)[0]["faces"], 12);
    assert_eq!(parts(&welded)[0]["part"], "cube-2", "the name is taken");
    let soup = s.query("file.import", json!({ "path": path, "merge": false }));
    assert_eq!(parts(&soup)[0]["vertices"], 36);
    assert_eq!(parts(&soup)[0]["part"], "cube-3");
}

/// A closed cube of quads, wound outward.
fn cube() -> gs_protocol::PlainMesh {
    let positions = (0..8u8)
        .map(|i| {
            [
                f32::from(i & 1),
                f32::from((i >> 1) & 1),
                f32::from((i >> 2) & 1),
            ]
        })
        .collect();
    gs_protocol::PlainMesh {
        positions,
        faces: vec![
            [0, 2, 3, 1],
            [4, 5, 7, 6],
            [0, 1, 5, 4],
            [2, 6, 7, 3],
            [0, 4, 6, 2],
            [1, 3, 7, 5],
        ],
        attributes: std::collections::BTreeMap::new(),
    }
}

const TWO_OBJECTS: &str = "o left\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n\
                           o right\nv 5 0 0\nv 6 0 0\nv 5 1 2\nf 4 5 6\n";

#[test]
fn an_import_is_one_undo_step() {
    let mut s = Session::new();
    let path = put(&folder(), "two.obj", TWO_OBJECTS.as_bytes());
    let empty = s.hash();
    let done = s.ok("file.import", json!({ "path": path }));
    assert_eq!(names(&mut s), ["left", "right"]);
    let effect = done.effect.unwrap();
    assert_eq!((effect.verts_touched, effect.topology_changed), (6, true));
    let full = s.hash();
    s.ok("history.undo", json!({ "expect": done.history_id }));
    assert_eq!(s.hash(), empty, "undo removes every imported part");
    s.ok("history.redo", json!({ "expect": done.history_id }));
    assert_eq!(s.hash(), full);
}

fn log_path() -> PathBuf {
    folder().join("session.jsonl")
}

fn logged(log: &Path) -> Session {
    let config = EngineConfig {
        log_path: Some(log.to_path_buf()),
        ..EngineConfig::default()
    };
    Session::with(config, Role::Agent)
}

fn records(log: &Path) -> Vec<LogRecord> {
    let text = std::fs::read_to_string(log).unwrap();
    (text.lines().skip(1))
        .map(|line| match ScriptLine::parse(line).unwrap() {
            ScriptLine::Record(r) => r,
            ScriptLine::Header(_) => panic!("a second header"),
        })
        .collect()
}

#[test]
fn the_log_pins_the_file_and_a_replay_notices_a_change() {
    let dir = folder();
    let path = put(&dir, "two.obj", TWO_OBJECTS.as_bytes());
    let log = log_path();
    let mut s = logged(&log);
    s.ok("file.import", json!({ "path": path, "scale": 2 }));
    let target = s.hash();
    let lines = records(&log);
    let hash = lines[0].args["hash"].as_str().unwrap().to_string();
    assert_eq!(hash.len(), 64, "the log records the file's content hash");
    assert_eq!(
        hash,
        blake3::hash(TWO_OBJECTS.as_bytes()).to_hex().to_string()
    );
    let mut again = Session::new();
    success(again.request(lines[0].request().unwrap()), "replay");
    assert_eq!(
        again.hash(),
        target,
        "a replay of the same file rebuilds the scene"
    );
    // The file changes: the replayed line is refused and names the file.
    put(
        &dir,
        "two.obj",
        format!("{TWO_OBJECTS}# edited\n").as_bytes(),
    );
    let mut changed = Session::new();
    let before = changed.hash();
    let err = failure(changed.request(lines[0].request().unwrap()), "changed file");
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert!(
        err.message.contains(&path) && err.message.contains("changed"),
        "{}",
        err.message
    );
    assert_eq!(changed.hash(), before);
    // Leaving the hash out reads the file as it is now.
    let mut fresh = Session::new();
    fresh.ok("file.import", json!({ "path": path }));
}

#[test]
fn scale_mirrors_and_up_axis_move_positions_and_the_export_undoes_them() {
    let dir = folder();
    let path = put(&dir, "two.obj", TWO_OBJECTS.as_bytes());
    let mut s = Session::new();
    let args = json!({ "path": path, "scale": 0.5, "flip_x": true, "up": "z", "name": "m" });
    let result = s.query("file.import", args);
    assert_eq!(parts(&result)[0]["part"], "m-left");
    // Mirrored in x, then the file's +Z turned to +Y (and its +Y to -Z), then halved.
    assert_eq!(s.mesh("m-right").positions[1], [-3.0, 0.0, -0.0]);
    assert_eq!(s.mesh("m-left").positions[2], [-0.0, 0.0, -0.5]);
    let scale = parts(&result)[1]["export_scale"].clone();
    assert_eq!(scale, 2.0);
    let out = at(&dir, "out.obj");
    let back = json!({ "path": out, "part": "m-right", "scale": scale, "flip_x": true, "up": "z" });
    s.ok("file.export", back);
    let file = gs_io::obj::read(
        std::fs::File::open(&out).unwrap(),
        &gs_io::mesh::ReadOptions::default(),
    );
    let written = file.unwrap().objects.remove(0).mesh.positions;
    assert_eq!(written, [[5.0, 0.0, 0.0], [6.0, 0.0, 0.0], [5.0, 1.0, 2.0]]);
    assert_eq!(s.mesh("m-right").positions[2], [-2.5, 1.0, -0.5]);
    for flip in ["flip_y", "flip_z"] {
        let mut t = Session::new();
        t.ok("file.import", json!({ "path": path, flip: true }));
        let axis = usize::from(flip == "flip_z") + 1;
        assert_eq!(t.mesh("right").positions[2][axis], -(axis as f32), "{flip}");
    }
}

#[test]
fn fit_scales_and_centers_and_the_export_gives_the_file_back() {
    let dir = folder();
    // Away from the origin, so the size is not just the far corner.
    let shifted = "o left\nv 1 1 1\nv 2 1 1\nv 1 2 1\nf 1 2 3\n\
                   o right\nv 6 1 1\nv 7 1 1\nv 6 2 3\nf 4 5 6\n";
    let path = put(&dir, "two.obj", shifted.as_bytes());
    let mut s = Session::new();
    let result = s.query(
        "file.import",
        json!({ "path": path, "fit": true, "fit_size": 3, "scale": 2 }),
    );
    let bounds = s.query("mesh.bounds", json!({}));
    let (min, max) = (&bounds["min"], &bounds["max"]);
    let side = |i: usize| max[i].as_f64().unwrap() - min[i].as_f64().unwrap();
    assert!((side(0) - 3.0).abs() < 1e-6, "{bounds}");
    for i in 0..3 {
        let center = max[i].as_f64().unwrap().midpoint(min[i].as_f64().unwrap());
        assert!(center.abs() < 1e-6, "centered: {bounds}");
    }
    let part = &parts(&result)[1];
    let out = at(&dir, "out.obj");
    let back = json!({ "path": out, "part": "right", "scale": part["export_scale"], "offset": part["export_offset"] });
    s.ok("file.export", back);
    let read = gs_io::obj::read(
        std::fs::File::open(&out).unwrap(),
        &gs_io::mesh::ReadOptions::default(),
    )
    .unwrap();
    let want = [[6.0, 1.0, 1.0], [7.0, 1.0, 1.0], [6.0, 2.0, 3.0]];
    for (p, w) in read.objects[0].mesh.positions.iter().zip(want) {
        assert!(
            (0..3).all(|i| (p[i] - w[i]).abs() < 1e-5),
            "{p:?} against {w:?}"
        );
    }
}

/// The positions of the one object in an OBJ file.
fn written(path: &str) -> Vec<[f32; 3]> {
    let file = std::fs::File::open(path).unwrap();
    let read = gs_io::obj::read(file, &gs_io::mesh::ReadOptions::default()).unwrap();
    read.objects[0].mesh.positions.clone()
}

/// A part remembers how to go back to its file (decision 0010): an export with no scale or
/// offset writes it at the file's size and place, and the part keeps that through edits, a
/// checkpoint restore and a remesh.
#[test]
fn a_part_goes_back_to_its_file_size_by_itself() {
    let dir = folder();
    let shifted = "o right\nv 6 1 1\nv 7 1 1\nv 6 2 3\nf 1 2 3\n";
    let path = put(&dir, "one.obj", shifted.as_bytes());
    let mut s = Session::new();
    s.ok(
        "file.import",
        json!({ "path": path, "fit": true, "scale": 2 }),
    );
    s.ok("history.checkpoint", json!({ "name": "in" }));
    let close = |got: &[[f32; 3]]| {
        let want = [[6.0, 1.0, 1.0], [7.0, 1.0, 1.0], [6.0, 2.0, 3.0]];
        (got.len() == 3)
            && got
                .iter()
                .zip(want)
                .all(|(p, w)| (0..3).all(|i| (p[i] - w[i]).abs() < 1e-5))
    };
    let out = at(&dir, "a.obj");
    s.ok("file.export", json!({ "path": out, "part": "right" }));
    assert!(close(&written(&out)), "{:?}", written(&out));
    let moved = json!({ "part": "right", "direction": [1, 0, 0], "distance": 1 });
    s.ok("mesh.translate", moved);
    s.ok("history.restore", json!({ "name": "in" }));
    let out = at(&dir, "b.obj");
    s.ok("file.export", json!({ "path": out, "part": "right" }));
    assert!(
        close(&written(&out)),
        "after a restore: {:?}",
        written(&out)
    );
    // A closed tetrahedron with corners 0 to 1, imported at twice its size, then remeshed.
    let tetra = "o t\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n";
    let path = put(&dir, "t.obj", tetra.as_bytes());
    s.ok("file.import", json!({ "path": path, "scale": 2 }));
    s.ok(
        "mesh.voxel_remesh",
        json!({ "part": "t", "resolution": 16 }),
    );
    let out = at(&dir, "c.obj");
    s.ok("file.export", json!({ "path": out, "part": "t" }));
    let far = (written(&out).iter().flatten()).fold(0f32, |m, &c| m.max(c));
    assert!(
        (0.8..1.2).contains(&far),
        "remeshed, at the file's size: {far}"
    );
    s.ok("primitive.cube", json!({ "name": "box" }));
    let out = at(&dir, "d.obj");
    s.ok("file.export", json!({ "path": out, "part": "box" }));
    let cube = s.mesh("box").positions;
    assert_eq!(written(&out), cube, "a part from no file goes out as it is");
}

#[test]
fn fit_makes_the_largest_side_two_unless_told() {
    let path = put(&folder(), "two.obj", TWO_OBJECTS.as_bytes());
    let mut s = Session::new();
    s.ok("file.import", json!({ "path": path, "fit": true }));
    let size = s.query("mesh.bounds", json!({}))["size"].clone();
    assert!((size[0].as_f64().unwrap() - 2.0).abs() < 1e-6, "{size}");
}

#[test]
fn switches_leave_columns_out() {
    let mut s = with_painted();
    let dir = folder();
    let path = at(&dir, "p.obj");
    s.ok("file.export", json!({ "path": path }));
    for (switch, gone) in [
        ("groups", vec!["group"]),
        ("colors", vec!["color"]),
        ("mask", vec!["mask"]),
        ("uvs", vec!["uv0.u", "uv3.v"]),
    ] {
        let result = s.query("file.import", json!({ "path": path, switch: false }));
        let columns = text(&parts(&result)[0]["columns"]);
        for column in gone {
            assert!(
                !columns.contains(&format!("\"{column}\"")),
                "{switch}: {columns}"
            );
        }
        assert!(
            columns.contains("\"group\"") || switch == "groups",
            "{columns}"
        );
    }
    let materials = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nv 2 2 0\nusemtl a\nf 1 2 3\nusemtl b\nf 2 4 3\nf 2 5 4 3\n";
    let path = put(&dir, "m.obj", materials.as_bytes());
    let mut t = Session::new();
    t.ok(
        "file.import",
        json!({ "path": path, "materials_as_groups": true, "name": "m" }),
    );
    let groups = &t.mesh("m").attributes["group"].values;
    assert_eq!(format!("{groups:?}"), "U32([0, 1, 1])");
    let pentagon = "v 0 0 0\nv 1 0 0\nv 2 1 0\nv 1 2 0\nv 0 1 0\nv 3 3 0\nf 1 2 3 4 5\nf 2 6 3\n";
    let path = put(&dir, "poly.obj", pentagon.as_bytes());
    let result = t.query(
        "file.import",
        json!({ "path": path, "group_polygons": true, "name": "poly" }),
    );
    assert!(
        text(&result["notes"]).contains("split 1 polygons"),
        "{result}"
    );
    let groups = &t.mesh("poly").attributes["group"].values;
    assert_eq!(format!("{groups:?}"), "U32([1, 1, 1, 0])");
    // Merging is off for OBJ unless asked.
    let twice = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 0 0\nv 1 1 0\nf 1 2 3\nf 4 5 3\n";
    let path = put(&dir, "twice.obj", twice.as_bytes());
    let kept = t.query("file.import", json!({ "path": path }));
    assert_eq!(parts(&kept)[0]["vertices"], 5);
    let merged = t.query("file.import", json!({ "path": path, "merge": true }));
    assert_eq!(parts(&merged)[0]["vertices"], 4);
    // 0 and -0 are one position; a file with nothing to merge, drop or split has no notes.
    let zeros = "v 0 0 0\nv -0 -0 0\nv 1 0 0\nv 0 1 0\nf 1 3 4\nf 2 4 3\n";
    let path = put(&dir, "zeros.obj", zeros.as_bytes());
    let merged = t.query("file.import", json!({ "path": path, "merge": true }));
    assert_eq!(parts(&merged)[0]["vertices"], 3);
    let path = put(&dir, "two.obj", TWO_OBJECTS.as_bytes());
    let clean = t.query("file.import", json!({ "path": path, "merge": true }));
    assert_eq!(clean["notes"], json!([]));
}

#[test]
fn files_that_cannot_be_imported_are_refused_and_change_nothing() {
    let dir = folder();
    let mut s = with_painted();
    let mut refused = |args: Value| s.err("file.import", args);
    let err = refused(json!({ "path": at(&dir, "none.obj") }));
    assert_eq!(code(err.code), "E_NOT_FOUND");
    assert!(err.hint.unwrap().contains("absolute path"));
    let err = refused(json!({ "path": put(&dir, "notes.txt", b"hi") }));
    assert!(err.hint.clone().unwrap().contains("obj, ply, stl"));
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        ("E_ARGS_INVALID".into(), Some("/format"))
    );
    let obj = put(&dir, "ok.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    for (arg, value) in [("scale", 0.0), ("fit_size", -1.0)] {
        let err = refused(json!({ "path": obj, arg: value }));
        assert_eq!(err.path.as_deref(), Some(format!("/{arg}").as_str()));
    }
    let err = refused(json!({ "path": obj, "hash": "0".repeat(64) }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    let empty = put(&dir, "empty.obj", b"# nothing\nv 0 0 0\n");
    let err = refused(json!({ "path": empty }));
    assert!(err.message.contains("holds no faces"), "{}", err.message);
    assert!(err.hint.unwrap().contains("another program"));
    let as_ply = refused(json!({ "path": obj, "format": "ply" }));
    assert!(as_ply.message.contains("line 1"), "{}", as_ply.message);
    // A face using one vertex twice is dropped; zero-area faces are not this import's business.
    let doubled = put(
        &dir,
        "doubled.obj",
        b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 2 0 0\ng a\nf 1 1 2\ng b\nf 1 2 3\n",
    );
    let mut t = Session::new();
    let result = t.query("file.import", json!({ "path": doubled }));
    assert!(
        text(&result["notes"]).contains("dropped 1 faces that used a vertex twice"),
        "{result}"
    );
    let groups = &t.mesh("doubled").attributes["group"].values;
    assert_eq!(
        format!("{groups:?}"),
        "U32([1])",
        "the face column follows its faces"
    );
    // Merged vertices keep the first one's color.
    let painted = "v 0 0 0 1 0 0\nv 1 0 0 0 1 0\nv 0 1 0 0 0 1\nv 0 0 0 1 1 1\nf 1 2 3\nf 4 3 2\n";
    let path = put(&dir, "painted.obj", painted.as_bytes());
    t.ok("file.import", json!({ "path": path, "merge": true }));
    let colors = &t.mesh("painted").attributes["color"].values;
    let want = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]].map(u32::from_le_bytes);
    assert_eq!(format!("{colors:?}"), format!("U32({want:?})"));
    let all_doubled = put(&dir, "all.obj", b"v 0 0 0\nv 1 0 0\nf 1 1 2\n");
    let err = t.err("file.import", json!({ "path": all_doubled }));
    assert!(err.message.contains("holds no faces"), "{}", err.message);
    // What the scene refuses is the file's fault, not an argument's.
    let far = put(&dir, "far.obj", b"v 2e18 0 0\nv 0 1 0\nv 0 0 1\nf 1 2 3\n");
    let err = t.err("file.import", json!({ "path": far }));
    assert_eq!((code(err.code), err.path), ("E_PRECONDITION".into(), None));
    assert!(
        err.message.starts_with("the file cannot become a part"),
        "{}",
        err.message
    );
}

#[test]
#[cfg(unix)]
fn a_file_that_cannot_be_opened_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = folder();
    let path = put(&dir, "locked.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let mut s = Session::new();
    let err = s.err("file.import", json!({ "path": path }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    // In a person's words, naming the file; never the system's text with its error number.
    for words in ["locked.obj", "not allowed to read"] {
        assert!(err.message.contains(words), "{}", err.message);
    }
    assert!(!err.message.contains("os error"), "{}", err.message);
}

/// One damaged file per open format carrying a value that is not a finite number into a column
/// (STL has no columns, so into a position): each is refused, naming where.
#[test]
fn values_that_are_not_finite_are_refused_with_their_place() {
    let dir = folder();
    let mut s = Session::new();
    let cases = [
        (
            "uv.obj",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvt nan 0.5\nf 1/1 2/1 3/1\n".to_string(),
            "line 4",
        ),
        (
            "color.obj",
            "v 0 0 0 1 inf 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_string(),
            "line 1",
        ),
        (
            "color.ply",
            "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
             property float z\nproperty float red\nproperty float green\nproperty float blue\n\
             element face 1\nproperty list uchar int vertex_indices\nend_header\n\
             0 0 0 1 1 1\n1 0 0 nan 1 1\n0 1 0 1 1 1\n3 0 1 2\n"
                .to_string(),
            "line 14",
        ),
    ];
    for (name, body, place) in cases {
        let err = s.err(
            "file.import",
            json!({ "path": put(&dir, name, body.as_bytes()) }),
        );
        assert!(err.message.contains(place), "{name}: {}", err.message);
    }
    let mut stl = vec![b' '; 80];
    stl.extend(1u32.to_le_bytes());
    for x in [
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        0.0,
        f32::INFINITY,
        1.0,
        0.0,
    ] {
        stl.extend(f32::to_le_bytes(x));
    }
    stl.extend([0, 0]);
    let err = s.err("file.import", json!({ "path": put(&dir, "inf.stl", &stl) }));
    assert!(err.message.contains("byte 84"), "{}", err.message);
}

/// Mirrored n-gons split into mirrored triangles; the parts are named after the file.
#[test]
fn a_file_without_object_names_is_named_after_the_file() {
    let dir = folder();
    let path = put(&dir, "Plate.obj", b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n");
    let mut s = Session::new();
    s.ok("file.import", json!({ "path": path }));
    s.ok("file.import", json!({ "path": path, "name": "given" }));
    assert_eq!(names(&mut s), ["Plate", "given"]);
}

/// No part is ever named like a reference (review 0003 M1): an export by id writes the part's
/// own name, a file's object named like one becomes `part #1` with a note, and a given name
/// like one is refused. Otherwise `#1` would name two parts and commands would pick the wrong one.
#[test]
fn no_part_is_named_like_a_reference() {
    let dir = folder();
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({}));
    s.ok("primitive.cube", json!({}));
    let path = at(&dir, "by-id.obj");
    let out = s.query("file.export", json!({ "part": "#2", "path": path }));
    assert_eq!(out["parts"], json!(["cube"]));
    let back = s.query("file.import", json!({ "path": path }));
    assert_eq!(parts(&back)[0]["part"], "cube-2");
    let odd = put(
        &dir,
        "odd.obj",
        b"o #1\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n",
    );
    let named = s.query("file.import", json!({ "path": odd }));
    assert_eq!(parts(&named)[0]["part"], "part #1");
    assert!(
        text(&named["notes"]).contains("'#1' would read as a part id"),
        "{named}"
    );
    let err = s.err("file.import", json!({ "path": odd, "name": "@0" }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert_eq!(names(&mut s), ["cube", "cube-2", "part #1", "sphere"]);
    let first = s.query("mesh.stats", json!({ "part": "#1" }));
    assert_eq!(first["parts"][0]["part"], "sphere", "{first}");
}
