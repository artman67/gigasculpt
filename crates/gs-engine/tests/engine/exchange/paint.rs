//! Surface paint through the exchange without Blender (decision 0032, section 8's amendment;
//! decision 0039, section 12): the folder `exchange.write` fills, edited as Blender would send
//! it back, and folders written by hand for the colors only Blender writes.

use std::fs;
use std::path::Path;

use gs_io::mesh::column;
use gs_protocol::{AttrDomain, AttrValues, Attribute, ErrorCode, PlainMesh};
use gs_testkit::{Session, fixtures, text};
use serde_json::{Value, json};

use super::{arg, column_file, folder, manifest, painted, put_manifest};
use crate::determinism::seeded;
use crate::support::{EngineSession, agent};
use crate::views::log_path;

const WHITE: u32 = u32::MAX;
/// Red, as the color column holds it (red in the low byte).
const RED: u32 = 0xff00_00ff;

/// The color words of `m`, white where it has no column.
fn colors(m: &PlainMesh) -> Vec<u32> {
    let white = vec![WHITE; m.positions.len()];
    column(m, "color", AttrDomain::Vertex).map_or(white, <[u32]>::to_vec)
}

/// Everything of `m` but its colors.
fn uncolored(mut m: PlainMesh) -> Value {
    m.attributes.remove("color");
    serde_json::to_value(m).unwrap_or_else(|e| panic!("{e}"))
}

fn undoable(s: &mut Session) -> u64 {
    s.history()["undoable"].as_u64().unwrap_or(u64::MAX)
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn a_painted_part_writes_the_color_file_its_manifest_names_with_the_stores_bytes() {
    let mut s = painted();
    let top = colors(&s.mesh("c"));
    assert!(top.iter().any(|&c| c != WHITE), "the fill painted");
    let (before, steps) = (s.hash(), undoable(&mut s));
    let dir = folder("paint-out");
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    let part = manifest(&dir)["parts"][0].clone();
    let listed = part["columns"].as_array().cloned().unwrap_or_default();
    let color = listed.iter().find(|c| c["gs"] == "color");
    let want = json!({ "gs": "color", "blender": "Color", "domain": "point", "type": "u8",
                       "width": 4, "file": "0-Color.bin" });
    assert_eq!(color, Some(&want), "{part}");
    let bytes: Vec<u8> = top.iter().flat_map(|c| c.to_le_bytes()).collect();
    assert!(
        read(&dir.join("0-Color.bin")) == bytes,
        "the store's bytes, red first"
    );
    assert!(
        part.get("paint_hidden").is_none(),
        "nothing while the paint shows"
    );

    // Hidden paint goes with its switch, and its colors all the same.
    s.ok("part.set_paint_visible", json!({ "visible": false }));
    let hidden = folder("paint-out-hidden");
    s.ok("exchange.write", json!({ "folder": arg(&hidden) }));
    assert_eq!(manifest(&hidden)["parts"][0]["paint_hidden"], json!(true));
    assert!(
        read(&hidden.join("0-Color.bin")) == bytes,
        "hidden paint goes too"
    );
    s.ok("history.undo", json!({}));

    // The Levels form sends no color.
    let levels = folder("paint-out-levels");
    let form = json!({ "folder": arg(&levels), "form": "levels" });
    s.ok("exchange.write", form);
    let sent = manifest(&levels)["parts"][0]["columns"].clone();
    assert!(!sent.to_string().contains("Color"), "{sent}");

    // Read back as they went, each changes nothing.
    for dir in [&dir, &hidden, &levels] {
        let came = s.ok("exchange.read", json!({ "folder": arg(dir) }));
        assert_eq!(
            came.result["parts"][0]["how"], "unchanged",
            "{}",
            came.result
        );
    }
    assert_eq!(s.hash(), before, "the paint came back exactly");
    assert_eq!(undoable(&mut s), steps, "no undo step for nothing");
}

/// A cube with a mask, three face groups and hidden faces, set by hand, and no color.
fn unpainted() -> Session {
    let mut cube = fixtures::cube(2);
    let (points, faces) = (cube.positions.len() as u32, cube.faces.len() as u32);
    let mask = (0..points).map(|v| v as f32 / points as f32).collect();
    let group = (0..faces).map(|f| f % 3 + 1).collect();
    let hide = (0..faces).map(|f| u32::from(f % 5 == 0)).collect();
    let (point, face) = (AttrDomain::Vertex, AttrDomain::Face);
    for (name, domain, values) in [
        ("mask", point, AttrValues::F32(mask)),
        ("group", face, AttrValues::U32(group)),
        ("hide", face, AttrValues::U32(hide)),
    ] {
        cube.attributes
            .insert(name.into(), Attribute { domain, values });
    }
    let mut s = agent();
    s.add_part("c", &cube);
    s
}

/// One BLAKE3 hash of a folder as `exchange.read` pins it (decision 0032, section 1), with the
/// writer's version left out so the pin holds across releases.
fn folder_hash(dir: &Path) -> String {
    let text_of = String::from_utf8(read(&dir.join("manifest.json"))).unwrap_or_default();
    let writer = concat!("GigaSculpt ", env!("CARGO_PKG_VERSION"));
    let mut hasher = blake3::Hasher::new();
    hasher.update(text_of.replace(writer, "GigaSculpt").as_bytes());
    let m = manifest(dir);
    let parts = m["parts"].as_array().into_iter().flatten();
    for c in parts.flat_map(|p| p["columns"].as_array().into_iter().flatten()) {
        hasher.update(&read(&dir.join(text(&c["file"]))));
    }
    hasher.finalize().to_hex().to_string()
}

#[test]
fn an_unpainted_part_writes_the_folder_it_wrote_before_byte_for_byte() {
    // Pinned on main at 700deaa5, before paint went to Blender: a part with no color column
    // writes exactly this, file names, manifest and every byte.
    const BEFORE: &str = "3060666204e29c57744f086981fe8a228262882d55c9adc93603116ee4d2bcab";
    let mut s = unpainted();
    let dir = folder("unpainted");
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    let shown = || String::from_utf8(read(&dir.join("manifest.json"))).unwrap_or_default();
    assert_eq!(folder_hash(&dir), BEFORE, "{}", shown());
    // Painted and undone, it has no column again and writes the same.
    s.ok("paint.fill", json!({ "color": "#3366cc", "seed": 1 }));
    s.ok("history.undo", json!({}));
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    assert_eq!(folder_hash(&dir), BEFORE, "{}", shown());
}

/// Bytes of four-byte elements, as their file holds them.
fn u8s(colors: &[[u8; 4]]) -> Vec<u8> {
    colors.concat()
}

/// Little-endian floats of four-float elements, as their file holds them.
fn f32s(colors: &[[f32; 4]]) -> Vec<u8> {
    colors
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

/// Object `k` as the add-on writes it, with no part id: a sheet of two triangles, `[0, 1, 2]`
/// and `[0, 2, 3]`, and the loose point 4, with one color column of `domain` and `ty` holding
/// `bytes`. Returns its manifest entry.
fn object(dir: &Path, k: usize, name: &str, (domain, ty): (&str, &str), bytes: &[u8]) -> Value {
    let points: [[f32; 3]; 5] = [
        [0., 0., 0.],
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [3.; 3],
    ];
    let ints = |v: &[i32]| -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() };
    let floats: Vec<u8> = points
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let files = [
        ("position", floats),
        ("corner_vert", ints(&[0, 1, 2, 0, 2, 3])),
        ("loop_start", ints(&[0, 3])),
        ("Color", bytes.to_vec()),
    ];
    for (blender, data) in &files {
        let path = dir.join(format!("{k}-{blender}.bin"));
        fs::write(&path, data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    let column = |gs: &str, blender: &str, domain: &str, ty: &str, width: u32| {
        json!({ "gs": gs, "blender": blender, "domain": domain, "type": ty, "width": width,
                "file": format!("{k}-{}.bin", blender.trim_start_matches('.')) })
    };
    json!({
        "name": name,
        "form": "mesh",
        "levels": 1,
        "transform": { "location": [0.0, 0.0, 0.0], "rotation": [1.0, 0.0, 0.0, 0.0], "scale": 1.0 },
        "points": 5,
        "faces": 2,
        "corners": 6,
        "columns": [
            column("positions", "position", "point", "f32", 3),
            column("faces", ".corner_vert", "corner", "i32", 1),
            column("faces", "loop_start", "face", "i32", 1),
            column("color", "Color", domain, ty, 4),
        ],
    })
}

/// Writes four objects into `dir`, one per color form only Blender writes, and returns what
/// each must come back as, point by point, worked out by hand in `f32` with the sRGB curve.
fn four_colors(dir: &Path) -> [[u32; 5]; 4] {
    // Point bytes: an opaque byte is taken as it is; alpha 0 leaves white; (128, 0, 0, 128) is
    // red 0.2159 linear, and laid over white with 1 - 128/255 = 0.4980 gives 0.7139 and
    // 0.4980, which encode to 219.76 and 187.19 of 255.
    let point_u8 = [
        [10, 20, 30, 255],
        [0, 0, 0, 0],
        [128, 0, 0, 128],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
    ];
    // Point floats, linear and premultiplied: 0.5 encodes to 187.52; (0.25, 0, 0) at alpha 0.5
    // over white is (0.75, 0.5, 0.5), and 0.75 encodes to 224.61; 2 and -1 clamp.
    let point_f32 = [
        [1.0, 0.5, 0.0, 1.0],
        [0.25, 0.0, 0.0, 0.5],
        [2.0, -1.0, 0.5, 1.0],
        [0.0; 4],
        [0.0, 0.0, 0.0, 1.0],
    ];
    // Corners 0 to 5 are points 0, 1, 2, 0, 2, 3, and point 4 has none, so it reads white.
    // Point 0 averages white and black in linear light, 0.5, which is 188, not the bytes' 128.
    let corner_u8 = [
        [255; 4],
        [0, 0, 0, 0],
        [10, 20, 30, 255],
        [0, 0, 0, 255],
        [10, 20, 30, 255],
        [255, 0, 0, 255],
    ];
    // Point 1: (0, 0, 0.5) at alpha 0.5 is (0.5, 0.5, 1) over white. Point 2 averages 0.5 and
    // 0 to 0.25, which encodes to 136.96.
    let corner_f32 = [
        [1.0; 4],
        [0.0, 0.0, 0.5, 0.5],
        [0.5, 0.5, 0.5, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
    ];
    let parts = [
        object(dir, 0, "point-bytes", ("point", "u8"), &u8s(&point_u8)),
        object(dir, 1, "point-floats", ("point", "f32"), &f32s(&point_f32)),
        object(dir, 2, "corner-bytes", ("corner", "u8"), &u8s(&corner_u8)),
        object(
            dir,
            3,
            "corner-floats",
            ("corner", "f32"),
            &f32s(&corner_f32),
        ),
    ];
    let m = json!({ "format": "gigasculpt-exchange", "version": 1, "writer": "test",
                    "parts": parts });
    put_manifest(dir, &m);
    [
        [0xff1e_140a, WHITE, 0xffbb_bbdc, RED, 0xff00_ff00],
        [0xff00_bcff, 0xffbc_bce1, 0xffbc_00ff, WHITE, 0xff00_0000],
        [0xffbc_bcbc, WHITE, 0xff1e_140a, RED, WHITE],
        [0xffbc_bcbc, 0xffff_bcbc, 0xff89_8989, 0xff00_ff00, WHITE],
    ]
}

#[test]
fn byte_float_and_corner_colors_come_back_as_blender_means_them_on_new_parts() {
    let mut s = agent();
    let dir = folder("paint-four");
    let want = four_colors(&dir);
    let read_back = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    let parts = read_back.result["parts"].as_array().cloned();
    let parts = parts.unwrap_or_default();
    assert_eq!(parts.len(), 4, "{}", read_back.result);
    for (came, want) in parts.iter().zip(want) {
        assert_eq!(came["how"], "new", "{came}");
        let name = text(&came["part"]);
        assert_eq!(colors(&s.mesh(&name)), want, "{name}");
    }
    // A float that is not a number is refused, naming its object.
    let file = dir.join("1-Color.bin");
    let mut bytes = read(&file);
    bytes[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    fs::write(&file, bytes).unwrap_or_else(|e| panic!("{e}"));
    let err = s.err("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(err.code, ErrorCode::Precondition, "{}", err.message);
    assert!(err.message.contains("'point-floats'"), "{}", err.message);
}

#[test]
fn changed_colors_come_back_in_place_as_one_undo_step_that_changes_only_them() {
    let mut s = painted();
    let (before, had) = (s.hash(), s.mesh("c"));
    let steps = undoable(&mut s);
    let dir = folder("paint-in-place");
    s.ok("exchange.write", json!({ "folder": arg(&dir) }));
    // Repainted in Blender: points 0 and 5 red.
    let file = column_file(&dir, "Color");
    let mut bytes = read(&file);
    for v in [0, 5] {
        bytes[4 * v..4 * v + 4].copy_from_slice(&RED.to_le_bytes());
    }
    fs::write(&file, &bytes).unwrap_or_else(|e| panic!("{e}"));
    let came = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(
        came.result["parts"][0]["how"], "in_place",
        "{}",
        came.result
    );
    assert_eq!(undoable(&mut s), steps + 1, "one undo step");
    let now = s.mesh("c");
    let (was, is) = (colors(&had), colors(&now));
    let changed: Vec<usize> = (0..was.len()).filter(|&v| was[v] != is[v]).collect();
    assert_eq!(changed, [0, 5], "only the repainted points");
    assert_eq!((is[0], is[5]), (RED, RED));
    assert!(uncolored(had) == uncolored(now), "nothing but the colors");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo takes it back");

    // Its color removed in Blender: white, as a table column left out reads.
    let mut m = manifest(&dir);
    if let Some(columns) = m["parts"][0]["columns"].as_array_mut() {
        columns.retain(|c| c["blender"] != "Color");
    }
    put_manifest(&dir, &m);
    let came = s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    assert_eq!(
        came.result["parts"][0]["how"], "in_place",
        "{}",
        came.result
    );
    let now = colors(&s.mesh("c"));
    assert!(now.iter().all(|&c| c == WHITE), "white everywhere");
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "and back");
}

#[test]
fn a_replay_of_a_read_gives_the_same_bits() {
    let dir = folder("paint-replay");
    four_colors(&dir);
    let log = log_path("exchange-paint");
    let mut s = seeded(1, Some(log.clone()));
    s.ok("exchange.read", json!({ "folder": arg(&dir) }));
    let hash = s.hash();
    let mut replay = seeded(2, None);
    replay.replay_log(&log);
    assert_eq!(replay.hash(), hash, "the replay");
}
