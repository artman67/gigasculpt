//! ZTL tools and ZPR projects through `file.import`, in files built with gs-io's test writer
//! from the format notes (decision 0005). Positions lie on a twisted cubic, where no three
//! points line up, so no face of any level has zero area.

use gs_core::{level_vertex_counts, subdivide_faces};
use gs_protocol::PlainMesh;
use gs_testkit::{Session, code};
use serde_json::{Value, json};

use crate::sample::{MESH, REST_11, hat_settings, primitive_tool};
use crate::writer::{
    RECORD_TAIL, absent, array_slot, companion_object, entry, faces, file, item, mesh_object,
    name_object, object, points, project_object, record, tool_object, tools_section,
};
use crate::{NO, folder, names, parts, put};

/// One part to write.
#[derive(Clone)]
struct Sub {
    name: &'static str,
    faces: Vec<[u32; 4]>,
    base: u32,
    /// The level it was saved at, and its top level when higher.
    current: u32,
    top: Option<u32>,
    uvs: Option<Vec<[f32; 8]>>,
    groups: Option<Vec<u32>>,
    /// Mask and color at every stored level.
    paint: bool,
    export: Option<([f32; 3], [f32; 3])>,
    /// One more mesh array the reader keeps by location.
    extra: bool,
    /// Every stored level holds the top level's first points (no edits pending below it).
    prefix: bool,
}

/// A quad and a triangle beside it.
fn body() -> Sub {
    Sub {
        name: "Body",
        faces: vec![[0, 1, 2, 3], [1, 4, 2, NO]],
        base: 5,
        current: 0,
        top: Some(1),
        uvs: Some(vec![
            [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 2.0, 0.5, 1.0, 1.0, 9.0, 9.0],
        ]),
        groups: Some(vec![4 | 0x0010_0000, 70]),
        paint: false,
        export: Some(([2.0; 3], [1.0, 0.0, -1.0])),
        extra: true,
        prefix: false,
    }
}

/// One quad saved at level 1 with a top level 3, painted and masked.
fn hat() -> Sub {
    Sub {
        name: "Hat",
        faces: vec![[0, 1, 2, 3]],
        base: 4,
        current: 1,
        top: Some(3),
        uvs: None,
        groups: None,
        paint: true,
        export: None,
        extra: false,
        prefix: false,
    }
}

fn count(s: &Sub, level: u32) -> usize {
    level_vertex_counts(&s.faces, s.base, level).unwrap()[level as usize] as usize
}

/// Stored positions (y down, z toward the back) of `n` points on a twisted cubic.
fn curve(n: usize, seed: f32) -> Vec<[f32; 3]> {
    (0..n)
        .map(|i| {
            let t = i as f32 / 4.0 + seed;
            [t, t * t, t * t * t]
        })
        .collect()
}

/// The per vertex data: every third vertex fully masked, colors from the index.
fn paint(n: usize) -> Vec<u8> {
    (0..n)
        .flat_map(|i| {
            let mask: u16 = if i % 3 == 0 { 0 } else { 0xFFFF };
            let [lo, hi] = mask.to_le_bytes();
            [1, 0, lo, hi, (i % 256) as u8, 7, 200, 0]
        })
        .collect()
}

/// The stored points of level `l` of `s`.
fn stored(s: &Sub, l: u32) -> Vec<[f32; 3]> {
    let n = count(s, l);
    match s.top.filter(|_| s.prefix) {
        Some(top) => curve(count(s, top), top as f32)[..n].to_vec(),
        None => curve(n, l as f32),
    }
}

fn mesh_and_companion(s: &Sub) -> (Vec<u8>, Vec<u8>) {
    let level = |l: u32| {
        let n = count(s, l);
        let data = if s.paint {
            array_slot(8, &paint(n), 1 << 17)
        } else {
            absent()
        };
        [array_slot(12, &points(&stored(s, l)), 1 << 17), data]
    };
    let mut slots = level(s.current).to_vec();
    slots.push(absent());
    if let Some(top) = s.top {
        slots.extend(level(top));
    }
    if s.extra {
        slots.push(array_slot(12, &points(&curve(3, 9.0)), 1 << 17));
    }
    let mesh = mesh_object(0x52, 408, &slots, &[]);
    let mut comp = vec![array_slot(16, &faces(&s.faces), 1 << 17)];
    if let Some(uvs) = &s.uvs {
        let bytes: Vec<u8> = uvs.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
        comp.push(array_slot(32, &bytes, 1 << 17));
    }
    if let Some(groups) = &s.groups {
        let bytes: Vec<u8> = groups.iter().flat_map(|v| v.to_le_bytes()).collect();
        comp.push(array_slot(4, &bytes, 1 << 17));
    }
    (
        mesh,
        companion_object(18, s.current, s.base, &comp, s.export),
    )
}

/// A tool object holding `subs`, the first selected.
fn tool(subs: &[Sub]) -> Vec<u8> {
    let mut list = [RECORD_TAIL.to_vec(), item(0, MESH)].concat();
    list.extend(entry(0x11, [0xFF; 4], subs[0].name, "", REST_11));
    for s in &subs[1..] {
        let (mesh, comp) = mesh_and_companion(s);
        list.extend(item(0, MESH));
        list.extend(name_object(s.name));
        list.extend(hat_settings());
        list.extend(record(11, 0, 0, &mesh, Some(&comp), &RECORD_TAIL));
        list.extend(entry(0x11, [0xFF; 4], s.name, "", REST_11));
    }
    let (mesh, comp) = mesh_and_companion(&subs[0]);
    let count = subs.len() as u32;
    let selected = record(11, 0, count, &mesh, Some(&comp), &list);
    tool_object(MESH, subs[0].name, &hat_settings(), &selected)
}

/// Stored positions in scene axes: y and z change sign.
fn scene(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    points.iter().map(|p| [p[0], -p[1], -p[2]]).collect()
}

fn col<'a, T: gs_io::mesh::Number>(m: &'a PlainMesh, name: &str) -> &'a [T] {
    gs_io::mesh::column(m, name, m.attributes[name].domain).unwrap()
}

fn import(bytes: &[u8], name: &str, args: &Value) -> (Session, Value) {
    let path = put(&folder(), name, bytes);
    let mut s = Session::new();
    let mut call = json!({ "path": path });
    call.as_object_mut()
        .unwrap()
        .extend(args.as_object().unwrap().clone());
    let result = s.query("file.import", call);
    (s, result)
}

fn levels(s: &mut Session, part: &str) -> Value {
    s.query("level.describe", json!({ "part": part }))
}

#[test]
fn every_part_keeps_its_levels_with_the_saved_level_active() {
    let (mut s, result) = import(
        &file(b"LTZ\0", &tool(&[body(), hat()])),
        "figure.ZTL",
        &json!({}),
    );
    assert_eq!(names(&mut s), ["Body", "Hat"]);
    let parts = parts(&result);
    // The hat: saved at level 1 of 4, shown there with its stored points, mask and colors as
    // edits not carried up yet.
    let d = levels(&mut s, "Hat");
    assert_eq!(
        (
            d["count"].clone(),
            d["active"].clone(),
            d["pending"].clone()
        ),
        (json!(4), json!(2), json!(true))
    );
    let hat = s.mesh("Hat");
    let n = count(&self::hat(), 1);
    let (level1, _) = subdivide_faces(&self::hat().faces, 4).unwrap();
    assert_eq!((n, hat.faces.clone()), (9, level1));
    assert_eq!(hat.positions, scene(&curve(n, 1.0)));
    let mask = col::<f32>(&hat, "mask");
    assert!(
        (0..n).all(|i| mask[i].to_bits() == if i % 3 == 0 { 1f32 } else { 0.0 }.to_bits()),
        "{mask:?}"
    );
    assert_eq!(
        col::<u32>(&hat, "color")[5],
        u32::from_le_bytes([200, 7, 5, 255]),
        "stored blue, green, red"
    );
    assert_eq!(
        parts[1]["levels"],
        json!({ "count": 4, "saved_at": 1, "stored": [1, 3], "imported": 1, "not_kept": [] })
    );
    assert_eq!(
        (parts[1]["vertices"].clone(), parts[1]["faces"].clone()),
        (json!(9), json!(4))
    );
    // Its top, when reached, has every point of level 3 (the level-1 edits carried up).
    let up = s.query("level.set", json!({ "part": "Hat", "level": 4 }));
    assert!(up["carried"].as_u64().is_some_and(|c| c > 0), "{up}");
    assert_eq!(s.mesh("Hat").positions.len(), count(&self::hat(), 3));
    // The body: saved at its base with a top at level 1; groups per base face, and the base
    // UVs as stored (each corner from its descendant at the top).
    let d = levels(&mut s, "Body");
    assert_eq!(
        (d["count"].clone(), d["active"].clone()),
        (json!(2), json!(1))
    );
    let body = s.mesh("Body");
    assert_eq!(body.faces, self::body().faces);
    assert_eq!(col::<u32>(&body, "group"), [4, 70]);
    let uvs = self::body().uvs.unwrap();
    for (k, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate().take(6) {
        let column: Vec<f32> = uvs.iter().map(|u| u[k]).collect();
        assert_eq!(col::<f32>(&body, name), column, "{name}");
    }
    assert_eq!(parts[0]["levels"]["not_kept"], json!([]));
    // Export scale and offset write the body as the file's export scale says: (p - offset) / 2.
    assert_eq!(parts[0]["export_scale"].as_f64(), Some(0.5));
    assert_eq!(parts[0]["export_offset"], json!([-0.5, 0.0, 0.5]));
    let notes = result["notes"].to_string();
    assert!(
        notes.contains("Hat: kept its 4 detail levels, level 1 active"),
        "{notes}"
    );
    assert!(notes.contains("Body: UVs above the base level"), "{notes}");
    assert!(notes.contains("Body: 1 stored parts not read"), "{notes}");
    assert!(
        !notes.contains("Hat: 1 stored") && !notes.contains("differs"),
        "{notes}"
    );
}

#[test]
fn stored_levels_come_back_exactly_with_their_faces() {
    // Saved at level 1 of 3, every stored level the top's first points: nothing pending, and
    // each level shows the top's points with decision 0006's faces; the top is as stored.
    let sub = Sub {
        top: Some(2),
        prefix: true,
        ..hat()
    };
    let (mut s, _) = import(
        &file(b"LTZ\0", &tool(std::slice::from_ref(&sub))),
        "prefix.ztl",
        &json!({}),
    );
    let d = levels(&mut s, "Hat");
    assert_eq!(
        (
            d["count"].clone(),
            d["active"].clone(),
            d["pending"].clone()
        ),
        (json!(3), json!(2), json!(false))
    );
    let mut faces = sub.faces.clone();
    let mut points = sub.base;
    for level in 0..3u32 {
        s.ok("level.set", json!({ "level": level + 1 }));
        let m = s.mesh("Hat");
        assert_eq!(m.faces, faces, "level {level} faces");
        assert_eq!(
            m.positions,
            scene(&stored(&sub, level)),
            "level {level} points"
        );
        (faces, points) = subdivide_faces(&faces, points).unwrap();
    }
}

#[test]
fn a_fit_moves_every_level_and_leaves_nothing_pending() {
    let sub = Sub {
        top: Some(2),
        prefix: true,
        ..hat()
    };
    let bytes = file(b"LTZ\0", &tool(&[sub]));
    let (mut s, _) = import(&bytes, "fit.ztl", &json!({ "fit": true }));
    assert_eq!(
        levels(&mut s, "Hat")["pending"],
        json!(false),
        "the fit edits no level"
    );
    let low = s.mesh("Hat").positions;
    s.ok("level.set", json!({ "level": 3 }));
    let top = s.mesh("Hat").positions;
    assert_eq!(
        low[..],
        top[..low.len()],
        "level 1 is still the top's first points"
    );
    let side = (0..3)
        .map(|k| {
            let (lo, hi) = top.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p[k]), hi.max(p[k]))
            });
            hi - lo
        })
        .fold(0.0f32, f32::max);
    assert!(
        (side - 2.0).abs() < 1e-4,
        "the top's largest side is {side}"
    );
}

#[test]
fn a_part_saved_at_its_only_level_keeps_its_uvs_as_stored() {
    let flat = Sub {
        top: None,
        extra: false,
        export: Some(([2.0, 3.0, 4.0], [0.0; 3])),
        ..body()
    };
    let (mut s, result) = import(&file(b"LTZ\0", &tool(&[flat])), "flat.ztl", &json!({}));
    let m = s.mesh("Body");
    assert_eq!(m.faces, body().faces);
    assert_eq!(col::<f32>(&m, "uv1.v"), [0.0, 0.5]);
    assert_eq!(parts(&result)[0]["levels"]["not_kept"], json!([]));
    let notes = result["notes"].to_string();
    assert!(
        !notes.contains("carried up") && !notes.contains("not read"),
        "{notes}"
    );
    assert!(notes.contains("differs per axis"), "{notes}");
    assert_eq!(parts(&result)[0]["export_scale"].as_f64(), Some(0.5));
}

fn project() -> Vec<u8> {
    let sections = [
        (0x800, object(1, &[2, 0, 0, 0])),
        (
            2,
            tools_section(&[
                (1, tool(&[hat()])),
                (2, primitive_tool()),
                (1, tool(&[body()])),
            ]),
        ),
    ];
    file(b"RPZ\0", &project_object(14, &sections))
}

#[test]
fn a_project_imports_its_tools_and_names_what_it_skips() {
    let (mut s, result) = import(&project(), "scene.zpr", &json!({}));
    assert_eq!(names(&mut s), ["Body", "Hat"]);
    let parts = parts(&result);
    assert_eq!(
        (parts[0]["tool"].clone(), parts[1]["tool"].clone()),
        (json!(0), json!(2))
    );
    let notes = result["notes"].to_string();
    assert!(notes.contains("Sphere3D_1: no polygon mesh"), "{notes}");
    let (mut t, _) = import(
        &project(),
        "scene.zpr",
        &json!({ "tool": 2, "name": "only" }),
    );
    assert_eq!(names(&mut t), ["only"]);
    let path = put(&folder(), "scene.zpr", &project());
    let err = t.err("file.import", json!({ "path": path, "tool": 1 }));
    assert!(err.message.contains("holds no faces"), "{}", err.message);
    let err = t.err("file.import", json!({ "path": path, "tool": 3 }));
    assert!(err.message.contains("numbered 0 to 2"), "{}", err.message);
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        ("E_ARGS_INVALID".into(), Some("/tool"))
    );
    let mut broken = project();
    broken.truncate(broken.len() - 40);
    let path = put(&folder(), "broken.zpr", &broken);
    let err = t.err("file.import", json!({ "path": path }));
    assert!(err.message.contains("byte"), "{}", err.message);
}

#[test]
fn uvs_that_are_not_finite_are_refused_and_the_scene_is_left_as_it_was() {
    let mut bad = body();
    bad.name = "Bad";
    bad.uvs.as_mut().unwrap()[1][2] = f32::NAN;
    let bytes = file(b"LTZ\0", &tool(&[hat(), bad]));
    let path = put(&folder(), "nan.ztl", &bytes);
    let mut s = Session::new();
    let err = s.err("file.import", json!({ "path": path }));
    assert!(
        err.message.contains("non_finite_attribute in uv1.u"),
        "{}",
        err.message
    );
    assert!(
        names(&mut s).is_empty(),
        "the hat imported first is rolled back"
    );
    let without = s.query("file.import", json!({ "path": path, "uvs": false }));
    assert_eq!(parts(&without).len(), 2);
}

/// Decision 0014 section 14's sibling: a tool file holds UVs for its base faces only, carried
/// up linearly as they are read, so each level shows its own UVs, the base's carried to it; the
/// file holds none other a level could show.
#[test]
fn every_level_of_a_tool_shows_its_own_uvs() {
    let tall = Sub {
        top: Some(2),
        ..body()
    };
    let (mut s, _) = import(&file(b"LTZ\0", &tool(&[tall])), "tall.ztl", &json!({}));
    let base = body().faces;
    let mut uvs: Vec<[[f32; 2]; 4]> = (body().uvs.unwrap().iter())
        .map(|u| std::array::from_fn(|k| [u[2 * k], u[2 * k + 1]]))
        .collect();
    for level in 1..=3 {
        s.ok("level.set", json!({ "part": "Body", "level": level }));
        let m = s.mesh("Body");
        for (k, name) in gs_io::mesh::UV_COLUMNS.iter().enumerate() {
            let shown = col::<f32>(&m, name);
            // A triangle's fourth corner holds no UV.
            for f in (0..m.faces.len()).filter(|&f| k < 6 || m.faces[f][3] != NO) {
                let want = uvs[f][k / 2][k % 2];
                assert_eq!(
                    shown[f].to_bits(),
                    want.to_bits(),
                    "level {level} {name} {f}"
                );
            }
        }
        let corners = |f: usize| if level == 1 && base[f][3] == NO { 3 } else { 4 };
        let mut next = Vec::new();
        gs_core::subdivide_corners(&uvs, corners, |c| next.push(c));
        uvs = next;
    }
}
