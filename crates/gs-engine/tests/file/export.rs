//! `file.export`: what it writes for each option, and what it refuses.

use gs_io::mesh::ReadOptions;
use gs_testkit::code;
use serde_json::json;

use crate::{NO, at, folder, only, painted, put, with_painted};

fn read_obj(path: &str) -> gs_io::mesh::MeshFile {
    gs_io::obj::read(std::fs::File::open(path).unwrap(), &ReadOptions::default()).unwrap()
}

#[test]
fn every_part_goes_into_one_file_each_as_an_object() {
    let mut s = with_painted();
    s.ok(
        "part.create",
        json!({ "name": "q", "mesh": only(&painted(), &[]) }),
    );
    let dir = folder();
    let path = at(&dir, "both.obj");
    let out = s.query("file.export", json!({ "path": path }));
    assert_eq!(out["parts"], json!(["p", "q"]));
    assert_eq!(
        (out["vertices"].as_u64(), out["faces"].as_u64()),
        (Some(14), Some(6))
    );
    assert_eq!(
        out["bytes"].as_u64(),
        Some(std::fs::metadata(&path).unwrap().len())
    );
    let file = read_obj(&path);
    let names: Vec<&str> = file.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["p", "q"]);
    // Binary STL is one mesh of triangles; ASCII STL keeps one solid per part.
    let stl = at(&dir, "both.stl");
    let out = s.query("file.export", json!({ "path": stl }));
    assert_eq!(out["vertices"].as_u64(), Some(2 * 15));
    let read = |p: &str| gs_io::stl::read(std::fs::File::open(p).unwrap()).unwrap();
    assert_eq!(read(&stl).objects.len(), 1);
    let ascii = at(&dir, "solids.stl");
    s.ok("file.export", json!({ "path": ascii, "ascii": true }));
    assert_eq!(read(&ascii).objects.len(), 2);
    let ply = at(&dir, "text.ply");
    s.ok(
        "file.export",
        json!({ "path": ply, "ascii": true, "part": "p" }),
    );
    assert!(
        std::fs::read_to_string(&ply)
            .unwrap()
            .starts_with("ply\nformat ascii 1.0\n")
    );
    let named = at(&dir, "no-extension");
    s.ok(
        "file.export",
        json!({ "path": named, "format": "ply", "part": "q" }),
    );
    assert!(
        std::fs::read(&named)
            .unwrap()
            .starts_with(b"ply\nformat binary_little_endian")
    );
}

#[test]
fn options_shape_the_file() {
    let mut s = with_painted();
    let dir = folder();
    let path = at(&dir, "shaped.obj");
    let args = json!({
        "path": path, "triangles": true, "flip_faces": true, "scale": 2, "offset": [1, 0, 0],
        "up": "z", "flip_y": true, "uvs": false, "groups": false, "colors": false, "mask": false
    });
    s.ok("file.export", args);
    let m = read_obj(&path).objects.remove(0).mesh;
    assert!(m.attributes.is_empty(), "every column switched off");
    assert_eq!(m.faces[..2], [[0, 2, 1, NO], [0, 3, 2, NO]]);
    // (x, y, z) * 2 + (1, 0, 0), turned Z-up to (x, -z, y), then y mirrored.
    let p = painted().positions[1];
    assert_eq!(m.positions[1], [p[0] * 2.0 + 1.0, p[2] * 2.0, p[1] * 2.0]);
    let flips = at(&dir, "flips.obj");
    s.ok(
        "file.export",
        json!({ "path": flips, "flip_x": true, "flip_z": true }),
    );
    let m = read_obj(&flips).objects.remove(0).mesh;
    let p = painted().positions[6];
    assert_eq!(m.positions[6], [-p[0], p[1], -p[2]]);
}

#[test]
fn files_are_never_replaced_unless_asked() {
    let mut s = with_painted();
    let dir = folder();
    let path = put(&dir, "kept.obj", b"precious");
    let err = s.err("file.export", json!({ "path": path }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert!(err.hint.unwrap().contains("overwrite"));
    assert_eq!(std::fs::read(&path).unwrap(), b"precious");
    s.ok("file.export", json!({ "path": path, "overwrite": true }));
    assert_eq!(read_obj(&path).objects.len(), 1);
    let leftovers = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(leftovers, 1, "no temporary file stays behind");
    let err = s.err(
        "file.export",
        json!({ "path": dir.to_string_lossy() , "format": "obj" }),
    );
    assert!(err.message.contains("is a folder"), "{}", err.message);
    let err = s.err(
        "file.export",
        json!({ "path": at(&dir, "missing/x.obj"), "overwrite": true }),
    );
    assert_eq!(code(err.code), "E_PRECONDITION");
    let err = s.err("file.export", json!({ "path": at(&dir, "missing/x.obj") }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}

#[test]
fn what_cannot_be_written_is_refused() {
    let mut s = with_painted();
    let dir = folder();
    let err = s.err("file.export", json!({ "path": at(&dir, "a.ztl") }));
    assert_eq!(err.path.as_deref(), Some("/format"));
    let err = s.err(
        "file.export",
        json!({ "path": at(&dir, "a.obj"), "scale": 0 }),
    );
    assert_eq!(err.path.as_deref(), Some("/scale"));
    let err = s.err(
        "file.export",
        json!({ "path": at(&dir, "a.obj"), "part": "nope" }),
    );
    assert_eq!(code(err.code), "E_NOT_FOUND");
    let err = s.err("file.export", json!({ "path": at(&dir, "a.txt") }));
    assert_eq!(err.path.as_deref(), Some("/format"));
    let mut empty = gs_testkit::Session::new();
    let err = empty.err("file.export", json!({ "path": at(&dir, "a.obj") }));
    assert_eq!(code(err.code), "E_PRECONDITION");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
