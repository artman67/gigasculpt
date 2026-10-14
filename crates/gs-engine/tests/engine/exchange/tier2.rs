//! The exchange through headless Blender (decision 0032, section 12). Each test runs one of the
//! add-on's test scripts, which import the add-on's own modules, in a Blender of its own: no
//! user setup (`--factory-startup`, `BLENDER_USER_RESOURCES` in the test's folder), one thread
//! (Multires results depend on the thread count), and a script that raises fails the run.
//! Blender is `GS_BLENDER`, else `/usr/bin/blender`; `GS_BLENDER=skip` skips these tests with a
//! SKIP line on stderr. Tier 1 leaves this module to tier 2.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gs_io::mesh::column;
use gs_protocol::{AttrDomain, PlainMesh};
use gs_testkit::measure::{self, norm, sub};
use gs_testkit::{Session, text};
use serde_json::{Value, json};

use super::{arg, column_file, folder, manifest, painted, sculpted};
use crate::socket::Server;
use crate::support::worst;

/// The Blender to run, or `None` after a SKIP line when `GS_BLENDER=skip`.
fn binary(test: &str) -> Option<String> {
    let bin = std::env::var("GS_BLENDER").unwrap_or_else(|_| "/usr/bin/blender".into());
    if bin == "skip" {
        eprintln!("SKIP {test}: GS_BLENDER=skip");
        return None;
    }
    assert!(
        Path::new(&bin).exists(),
        "no Blender at {bin}: the exchange round trips need Blender 5.2 or newer. Install it \
         (on Arch Linux, `pacman -S blender`), set GS_BLENDER to its path, or set \
         GS_BLENDER=skip to skip them"
    );
    Some(bin)
}

/// Runs `script` from `addons/blender/tests` with `args`, and `env` added, in a headless
/// Blender of its own. Returns the JSON of its `RESULT` line, or `None` when Blender is skipped.
fn blender(test: &str, script: &str, args: &[String], env: &[(&str, &Path)]) -> Option<Value> {
    let bin = binary(test)?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../addons/blender/tests")
        .join(script);
    let headless = [
        "-b",
        "--factory-startup",
        "--offline-mode",
        "-t",
        "1",
        "--python-exit-code",
        "1",
        "--python",
    ];
    let out = Command::new(&bin)
        .args(headless)
        .arg(&path)
        .arg("--")
        .args(args)
        .env("BLENDER_USER_RESOURCES", folder(&format!("{test}-blender")))
        .envs(env.iter().copied())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {bin}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let said = || format!("{stdout}{}", String::from_utf8_lossy(&out.stderr));
    assert!(
        out.status.success(),
        "{script} failed in Blender:\n{}",
        said()
    );
    let line = (stdout.lines())
        .find_map(|l| l.strip_prefix("RESULT "))
        .unwrap_or_else(|| panic!("{script} gave no RESULT line:\n{}", said()));
    Some(serde_json::from_str(line).unwrap_or_else(|e| panic!("{script}: {e}: {line}")))
}

/// One trip's folders: what GigaSculpt wrote, what Blender sent back, Blender's extras (the
/// first object's points in world space), and what the script reported.
struct Trip {
    to: PathBuf,
    back: PathBuf,
    extras: PathBuf,
    blender: Value,
}

/// Writes every part of `s` in `form`, takes them through Blender with `edit` (`trip.py` lists
/// the edits), and returns the folders; `None` when Blender is skipped.
fn trip(s: &mut Session, test: &str, form: &str, edit: &str) -> Option<Trip> {
    let [to, back, extras] = ["to", "back", "extras"].map(|k| folder(&format!("{test}-{k}")));
    s.ok(
        "exchange.write",
        json!({ "folder": arg(&to), "form": form }),
    );
    let args = [arg(&to), arg(&back), arg(&extras), edit.into()];
    let blender = blender(test, "trip.py", &args, &[])?;
    Some(Trip {
        to,
        back,
        extras,
        blender,
    })
}

/// Reads what Blender sent back and returns what became of the first object.
fn read(s: &mut Session, dir: &Path) -> Value {
    let read = s.ok("exchange.read", json!({ "folder": arg(dir) }));
    read.result["parts"][0].clone()
}

/// The bytes of the first part's column Blender calls `blender` in `dir`.
fn bytes(dir: &Path, blender: &str) -> Vec<u8> {
    let path = column_file(dir, blender);
    fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The length of the diagonal of `m`'s bounding box.
fn diagonal(m: &PlainMesh) -> f64 {
    let [lo, hi] = measure::bounds(&m.positions).unwrap_or_default();
    norm(sub(hi.map(f64::from), lo.map(f64::from)))
}

/// Part "c" at every level, base first, ending with the top active again.
fn every_level(s: &mut Session) -> Vec<PlainMesh> {
    let count = s.query("level.describe", json!({}))["count"]
        .as_u64()
        .unwrap_or(1);
    (1..=count)
        .map(|level| {
            s.ok("level.set", json!({ "level": level }));
            s.mesh("c")
        })
        .collect()
}

#[test]
fn a_mask_groups_hidden_faces_and_creases_come_back_with_the_same_hash() {
    let mut s = sculpted();
    let (before, steps) = (s.hash(), s.history()["undoable"].clone());
    let Some(t) = trip(&mut s, "t2-a", "mesh", "none") else {
        return;
    };
    let sent = manifest(&t.to)["parts"][0]["columns"].clone();
    let columns = sent.as_array().cloned().unwrap_or_default();
    assert_eq!(columns.len(), 6, "every column of the table went: {sent}");
    for c in &columns {
        let name = text(&c["blender"]);
        let (went, came) = (bytes(&t.to, &name), bytes(&t.back, &name));
        assert!(went == came, "{name} came back different");
        eprintln!("a: {name}, {} bytes, bit for bit", came.len());
    }
    let came = read(&mut s, &t.back);
    assert_eq!(came["how"], "unchanged", "{came}");
    assert_eq!(s.hash(), before, "the scene changed");
    assert_eq!(s.history()["undoable"], steps, "an undo step for nothing");
}

#[test]
fn b_a_moved_and_turned_part_keeps_its_place() {
    let mut s = sculpted();
    let had = s.mesh("c");
    let Some(t) = trip(&mut s, "t2-b", "mesh", "turn") else {
        return;
    };
    assert_eq!(read(&mut s, &t.back)["how"], "in_place");
    let again = folder("t2-b-again");
    s.ok("exchange.write", json!({ "folder": arg(&again) }));
    let place = |dir: &Path| manifest(dir)["parts"][0]["transform"].clone();
    assert_ne!(place(&t.back), place(&t.to), "Blender moved and turned it");
    assert_eq!(
        place(&again),
        place(&t.back),
        "the place came back exactly as Blender had it"
    );
    // Blender's edit in the scene's axes: a quarter turn about +y round the part's old place
    // (0.5, -1, 2), then 1 along x. Every point moved by exactly that, point by point.
    let now = s.mesh("c");
    let turned: Vec<[f32; 3]> = (had.positions.iter())
        .map(|&[x, y, z]| {
            let (dx, dz) = (f64::from(x) - 0.5, f64::from(z) - 2.0);
            [(dz + 1.5) as f32, y, (2.0 - dx) as f32]
        })
        .collect();
    let diag = diagonal(&now);
    let off = f64::from(worst(&now.positions, &turned)) / diag;
    eprintln!("b: every point within {off:.2e} of the bounding box diagonal of Blender's move");
    assert!(
        off <= 1e-6,
        "a point is {off:e} of the diagonal off Blender's move"
    );
    // And Blender shows every point where the scene has it (Blender's axes back to the scene's).
    let path = t.extras.join("world.bin");
    let raw = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let values: Vec<f64> = (raw.as_chunks::<8>().0.iter())
        .map(|&b| f64::from_le_bytes(b))
        .collect();
    let nearest = |p: [f64; 3]| {
        let gap = |q: &[f32; 3]| norm(sub(p, q.map(f64::from)));
        now.positions.iter().map(gap).fold(f64::INFINITY, f64::min)
    };
    let shown = (values.as_chunks::<3>().0.iter())
        .map(|&[x, y, z]| nearest([x, z, -y]))
        .fold(0.0, f64::max);
    eprintln!(
        "b: Blender shows every point within {:.2e} of the diagonal",
        shown / diag
    );
    assert!(shown / diag <= 1e-6, "Blender shows a point {shown:e} away");
}

#[test]
fn c_points_moved_in_blender_come_back_and_the_levels_stay() {
    let mut s = sculpted();
    let before = s.hash();
    let Some(t) = trip(&mut s, "t2-c", "mesh", "point") else {
        return;
    };
    assert!(
        bytes(&t.to, "position") != bytes(&t.back, "position"),
        "Blender moved a point"
    );
    assert_eq!(read(&mut s, &t.back)["how"], "in_place");
    let levels = s.query("level.describe", json!({}));
    assert_eq!(levels["count"], 3, "the part keeps its levels: {levels}");
    // Sent again, the part goes exactly as Blender sent it.
    let again = folder("t2-c-again");
    s.ok("exchange.write", json!({ "folder": arg(&again) }));
    for name in ["position", ".sculpt_mask", ".sculpt_face_set", ".hide_poly"] {
        assert!(
            bytes(&again, name) == bytes(&t.back, name),
            "{name} is not what Blender sent"
        );
    }
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo takes the whole read back");
}

#[test]
fn d_a_topology_change_comes_back_as_a_new_part_with_a_note() {
    let mut s = sculpted();
    let old = s.mesh("c");
    let Some(t) = trip(&mut s, "t2-d", "mesh", "topology") else {
        return;
    };
    assert_eq!(
        t.blender["polygons"],
        json!([old.faces.len() - 1]),
        "two faces became one in Blender"
    );
    let came = read(&mut s, &t.back);
    assert_eq!(
        (came["how"].clone(), came["split"].clone()),
        (json!("new"), json!(1)),
        "{came}"
    );
    let note = text(&came["note"]);
    assert!(
        note.contains("is a new part and 'c' is unchanged"),
        "{note}"
    );
    let (now, new) = (s.mesh("c"), s.mesh(&text(&came["part"])));
    assert!(
        now.positions == old.positions && now.faces == old.faces,
        "the old part stays"
    );
    assert_eq!(
        new.positions.len(),
        old.positions.len() + 1,
        "the loose point"
    );
    assert_eq!(
        new.faces.len(),
        old.faces.len() + 2,
        "a hexagon is four triangles"
    );
    for column in ["mask", "group", "hide"] {
        assert!(
            new.attributes.contains_key(column),
            "the new part has no {column}"
        );
    }
    eprintln!("d: {note}");
}

#[test]
fn e_the_levels_form_brings_three_levels_back_within_a_millionth() {
    let mut s = sculpted();
    let had = every_level(&mut s);
    let Some(t) = trip(&mut s, "t2-e", "levels", "none") else {
        return;
    };
    assert_eq!(
        t.blender["levels"],
        json!([2]),
        "Multires levels above the base"
    );
    let back = manifest(&t.back)["parts"][0].clone();
    assert_eq!(
        (&back["form"], &back["levels"]),
        (&json!("levels"), &json!(3))
    );
    let came = read(&mut s, &t.back);
    assert!(
        came["how"] == "in_place" || came["how"] == "unchanged",
        "{came}"
    );
    let now = every_level(&mut s);
    assert_eq!(now.len(), 3, "the part keeps three levels");
    let diag = diagonal(&had[2]);
    for (level, (a, b)) in had.iter().zip(&now).enumerate() {
        assert_eq!(a.faces, b.faces, "level {} has its faces", level + 1);
        let off = f64::from(worst(&a.positions, &b.positions)) / diag;
        eprintln!(
            "e: level {} within {off:.2e} of the bounding box diagonal",
            level + 1
        );
        assert!(
            off <= 1e-6,
            "level {} is {off:e} of the diagonal off",
            level + 1
        );
    }
}

/// The manifest line of the first part's color column in `dir`.
fn color_line(dir: &Path) -> Value {
    let m = manifest(dir);
    let columns = m["parts"][0]["columns"].as_array().cloned();
    let found = columns.into_iter().flatten().find(|c| c["gs"] == "color");
    found.unwrap_or_else(|| panic!("no color column in {m}"))
}

#[test]
fn g_a_painted_part_goes_as_the_active_and_render_color_and_comes_back_bit_for_bit() {
    let mut s = painted();
    // Hidden paint goes too, with its switch.
    s.ok("part.set_paint_visible", json!({ "visible": false }));
    let (before, steps) = (s.hash(), s.history()["undoable"].clone());
    let Some(t) = trip(&mut s, "t2-g", "mesh", "none") else {
        return;
    };
    let shown = json!([["Color", "POINT", "BYTE_COLOR", "Color"]]);
    assert_eq!(t.blender["color"], shown, "active and render color");
    assert_eq!(t.blender["paint"], json!([0.0]), "the Show Paint factor");
    let (went, came) = (bytes(&t.to, "Color"), bytes(&t.back, "Color"));
    assert!(
        !went.is_empty() && went == came,
        "Color came back different"
    );
    let line = color_line(&t.back);
    assert_eq!(
        (&line["domain"], &line["type"]),
        (&json!("point"), &json!("u8"))
    );
    eprintln!("g: Color, {} bytes, bit for bit", came.len());
    let came = read(&mut s, &t.back);
    assert_eq!(came["how"], "unchanged", "{came}");
    assert_eq!(s.hash(), before, "the scene changed");
    assert_eq!(s.history()["undoable"], steps, "an undo step for nothing");
}

#[test]
fn h_a_color_painted_in_blender_comes_back() {
    let mut s = sculpted();
    let before = s.hash();
    let Some(t) = trip(&mut s, "t2-h", "mesh", "paint") else {
        return;
    };
    let line = color_line(&t.back);
    assert_eq!(
        (&line["domain"], &line["type"]),
        (&json!("corner"), &json!("f32"))
    );
    assert_eq!(read(&mut s, &t.back)["how"], "in_place");
    let mesh = s.mesh("c");
    let colors = column::<u32>(&mesh, "color", AttrDomain::Vertex).unwrap_or_default();
    // Point 0's corners were (0, 0, 0.5) at alpha 0.5, which is (0.5, 0.5, 1) over white,
    // (188, 188, 255) in sRGB bytes; every other corner red.
    let want: Vec<u32> = (0..mesh.positions.len())
        .map(|v| if v == 0 { 0xffff_bcbc } else { 0xff00_00ff })
        .collect();
    assert!(
        colors == want.as_slice(),
        "{:x?}",
        &colors[..colors.len().min(8)]
    );
    s.ok("history.undo", json!({}));
    assert_eq!(s.hash(), before, "one undo takes the whole read back");
}

#[test]
fn f_the_name_table_matches_blender_5_2() {
    // Painted, so every row GigaSculpt writes is in its manifest.
    let mut s = painted();
    let to = folder("t2-f-to");
    s.ok("exchange.write", json!({ "folder": arg(&to) }));
    let Some(r) = blender("t2-f", "names_check.py", &[arg(&to)], &[]) else {
        return;
    };
    assert_eq!(r["fails"], json!([]), "{r}");
    eprintln!(
        "f: {} names checked against Blender {}",
        r["checked"], r["blender"]
    );
}

#[test]
fn the_add_on_gets_and_sends_parts_through_the_socket() {
    let session = "exchange-bridge";
    if binary(session).is_none() {
        return;
    }
    let server = Server::start(session, &[]);
    let mut c = server.client("setup", "agent");
    assert!(c.run("primitive.cube", json!({})).get("ok").is_some());
    let cache = folder("t2-socket-cache");
    let args = [arg(&server.socket), session.into()];
    let env = [("XDG_CACHE_HOME", cache.as_path())];
    let Some(r) = blender("t2-socket", "socket_trip.py", &args, &env) else {
        return;
    };
    let expected = json!({ "first": ["new"], "again": ["in_place"], "same_object": true,
                           "uv_maps": ["Made in Blender"], "left_in_to_blender": [] });
    for (key, want) in expected.as_object().into_iter().flatten() {
        assert_eq!(&r[key], want, "{key}: {r}");
    }
    assert_eq!(r["read"]["parts"][0]["how"], "in_place", "{r}");
    assert_eq!(r["unit_mm"].as_f64(), Some(50.0), "{r}");
    let scale = r["scale_length"].as_f64().unwrap_or(f64::NAN);
    assert!((scale - 0.05).abs() < 1e-7, "scale_length {scale}");
    // GigaSculpt has the point Blender moved: written again, the points are Blender's exactly.
    let from = cache.join(format!("gigasculpt/exchange/{session}/from-blender"));
    let again = folder("t2-socket-again");
    let wrote = c.run("exchange.write", json!({ "folder": arg(&again) }));
    assert!(wrote.get("ok").is_some(), "{wrote}");
    assert_eq!(bytes(&again, "position"), bytes(&from, "position"));
    drop(c);
    server.stop();
}
