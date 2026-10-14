//! `mesh.voxel_remesh` through the engine: what it reports, its settings, and its refusals.
//! The geometry itself is tested in gs-geo; these check the command around it.

use std::collections::BTreeSet;

use gs_protocol::{AttrValues, NO_VERTEX, PlainMesh};
use serde_json::{Value, json};

use gs_testkit::{Session, code};

/// Every edge used once in each direction: closed and consistently wound.
fn closed(mesh: &PlainMesh) -> bool {
    let mut edges = BTreeSet::new();
    for f in &mesh.faces {
        let c: Vec<u32> = f.iter().copied().filter(|&v| v != NO_VERTEX).collect();
        for k in 0..c.len() {
            if !edges.insert((c[k], c[(k + 1) % c.len()])) {
                return false;
            }
        }
    }
    edges.iter().all(|&(a, b)| edges.contains(&(b, a)))
}

fn sphere_session() -> Session {
    let mut s = Session::new();
    s.ok("primitive.sphere", json!({ "subdivisions": 6 }));
    s
}

#[test]
fn the_part_is_replaced_by_a_closed_quad_mesh_and_the_reply_describes_it() {
    let mut s = sphere_session();
    let done = s.ok("mesh.voxel_remesh", json!({ "resolution": 20 }));
    let mesh = s.mesh("sphere");
    let reply = &done.result;
    assert_eq!(reply["part"], json!("sphere"), "the part");
    assert_eq!(reply["vertices"], json!(mesh.positions.len()), "vertices");
    assert_eq!(reply["faces"], json!(mesh.faces.len()), "faces");
    assert_eq!(reply["inside"], json!("winding"), "a closed sphere");
    assert!(
        mesh.faces.len() >= 500,
        "non-trivial: {} faces",
        mesh.faces.len()
    );
    assert!(
        closed(&mesh) && mesh.faces.iter().all(|f| f[3] != NO_VERTEX),
        "closed quads"
    );
    let valid = s.query("mesh.validate", json!({ "part": "sphere" }));
    assert_eq!(valid["valid"], json!(true), "{valid}");
    let effect = done
        .effect
        .unwrap_or_else(|| panic!("an edit reports its effect"));
    assert!(effect.topology_changed, "new topology");
    assert_eq!(
        effect.verts_touched,
        mesh.positions.len() as u64,
        "every vertex is new"
    );
}

#[test]
fn the_cell_size_comes_from_the_resolution_or_is_given() {
    let mut s = sphere_session();
    let reply = s.query("mesh.voxel_remesh", json!({ "resolution": 20 }));
    let size = reply["voxel_size"].as_f64().unwrap_or(0.0);
    assert!(
        (size - 0.1).abs() <= 1e-6,
        "longest side 2 over 20 cells: {size}"
    );
    let mut s = sphere_session();
    let reply = s.query("mesh.voxel_remesh", json!({ "voxel_size": "5%" }));
    let size = reply["voxel_size"].as_f64().unwrap_or(0.0);
    let want = 0.05 * 2.0 * 3.0_f64.sqrt();
    assert!((size - want).abs() <= 1e-5, "5% of the diagonal: {size}");
    let mut s = sphere_session();
    let reply = s.query("mesh.voxel_remesh", json!({ "voxel_size": 0.25 }));
    assert_eq!(reply["voxel_size"], json!(0.25), "scene units");
    // Left out, the resolution is 128: tried on a thin rod, so 128 cells stay cheap.
    let mut s = Session::new();
    let rod = json!({
        "positions": [[0, 0, 0], [2, 0, 0], [2, 0.05, 0], [0, 0.05, 0],
                      [0, 0, 0.05], [2, 0, 0.05], [2, 0.05, 0.05], [0, 0.05, 0.05]],
        "faces": [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [0, 4, 7, 3]],
    });
    s.ok("part.create", json!({ "name": "rod", "mesh": rod }));
    let reply = s.query("mesh.voxel_remesh", json!({}));
    let size = reply["voxel_size"].as_f64().unwrap_or(0.0);
    assert!(
        (size - 2.0 / 128.0).abs() <= 1e-7,
        "longest side 2 over 128 cells: {size}"
    );
}

/// A tetrahedron with the given attribute columns.
fn tetra(attributes: &Value) -> Value {
    json!({
        "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
        "faces": [[0, 2, 1, NO_VERTEX], [0, 1, 3, NO_VERTEX], [0, 3, 2, NO_VERTEX], [1, 2, 3, NO_VERTEX]],
        "attributes": attributes,
    })
}

#[test]
fn face_groups_are_kept_only_when_asked_and_other_columns_always() {
    let attributes = json!({
        "group": {"domain": "face", "values": {"u32": [5, 6, 7, 8]}},
        "paint": {"domain": "vertex", "values": {"f32": [0.0, 0.25, 0.5, 1.0]}},
    });
    for keep in [false, true] {
        let mut s = Session::new();
        s.ok(
            "part.create",
            json!({ "name": "t", "mesh": tetra(&attributes) }),
        );
        s.ok(
            "mesh.voxel_remesh",
            json!({ "resolution": 16, "keep_groups": keep }),
        );
        let mesh = s.mesh("t");
        assert!(
            mesh.attributes.contains_key("paint"),
            "keep {keep}: paint carried"
        );
        match mesh.attributes.get("group").map(|g| &g.values) {
            Some(AttrValues::U32(groups)) => {
                assert!(keep, "groups dropped unless kept");
                assert!(
                    groups.iter().all(|g| (5..=8).contains(g)),
                    "old groups only"
                );
                assert_eq!(groups.len(), mesh.faces.len(), "one per face");
            }
            other => assert!(!keep && other.is_none(), "keep {keep}: {other:?}"),
        }
    }
}

/// Each column comes across by its kind (decision 0037, section 3), as a division carries it:
/// a color blends per channel, a label takes the heaviest corner's value. gs-geo's old rules
/// went by storage type, so color took one corner's value whole and never blended.
#[test]
fn a_remesh_carries_each_column_by_its_kind() {
    let (red, blue) = (0xff00_00ff_u32, 0xffff_0000_u32);
    let attributes = json!({
        "color": {"domain": "vertex", "values": {"u32": [red, blue, blue, blue]}},
        "label": {"domain": "vertex", "values": {"u32": [3, 9, 9, 9]}},
    });
    let mut s = Session::new();
    s.ok(
        "part.create",
        json!({ "name": "t", "mesh": tetra(&attributes) }),
    );
    s.ok(
        "mesh.voxel_remesh",
        json!({ "resolution": 16, "project": true }),
    );
    let mesh = s.mesh("t");
    let column = |name: &str| match mesh.attributes.get(name).map(|a| &a.values) {
        Some(AttrValues::U32(v)) => v.clone(),
        other => panic!("{name}: {other:?}"),
    };
    let (color, label) = (column("color"), column("label"));
    let channel = |c: u32, k: u32| c >> (8 * k) & 255;
    let mut blended = 0;
    for &c in &color {
        for k in 0..4 {
            let (a, b) = (
                channel(red, k).min(channel(blue, k)),
                channel(red, k).max(channel(blue, k)),
            );
            assert!(
                (a..=b).contains(&channel(c, k)),
                "{c:08x} lies between the two colors"
            );
        }
        blended += usize::from(c != red && c != blue);
    }
    assert!(blended >= 10, "colors blend per channel: {blended} blended");
    assert!(
        label.iter().all(|l| [3, 9].contains(l)),
        "labels are never blended"
    );
    assert!(
        label.contains(&3) && label.contains(&9),
        "both labels come across"
    );
}

#[test]
fn impossible_sizes_are_refused_and_change_nothing() {
    let mut s = sphere_session();
    let err = s.err("mesh.voxel_remesh", json!({ "voxel_size": 0.0001 }));
    assert_eq!(code(err.code), "E_LIMIT", "too many cells");
    assert!(err.hint.is_some(), "with a way out");
    let err = s.err("mesh.voxel_remesh", json!({ "voxel_size": -1 }));
    assert_eq!(code(err.code), "E_ARGS_INVALID", "negative size");
    assert_eq!(err.path.as_deref(), Some("/voxel_size"), "points at it");
    for resolution in [15, 2049] {
        let err = s.err("mesh.voxel_remesh", json!({ "resolution": resolution }));
        assert_eq!(code(err.code), "E_ARGS_INVALID", "resolution {resolution}");
    }
}

#[test]
fn an_open_part_is_capped_and_says_so() {
    // A tetrahedron missing a face. (A part's edges are never shared by three faces or by two
    // wound the same way: the mesh store refuses those. The generalized winding number is
    // reached only by surfaces that touch without sharing points, as in the next test.)
    let mut s = Session::new();
    let open = json!({
        "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
        "faces": [[0, 2, 1, NO_VERTEX], [0, 1, 3, NO_VERTEX], [0, 3, 2, NO_VERTEX]],
    });
    s.ok("part.create", json!({ "name": "open", "mesh": open }));
    let reply = s.query("mesh.voxel_remesh", json!({ "resolution": 16 }));
    assert_eq!(reply["inside"], json!("capped"), "a hole");
    assert!(closed(&s.mesh("open")), "closed over");
}

#[test]
fn a_part_over_its_copy_is_closed_where_its_inside_reaches_the_grid_edge() {
    // A sheet over a copy of itself winds about twice as much as one sheet, so its inside
    // reaches past the grid's margin (gs-geo's holes tests have the geometry). Both commands
    // panicked there in Surface Nets: `part.remesh` of the fixture and its copy, and
    // `mesh.voxel_remesh` of the two merged into one part.
    let fixture = gs_testkit::fixtures::all()
        .into_iter()
        .find(|f| f.name == "patch");
    let mut s = Session::new();
    s.add_part("patch", &fixture.expect("the patch fixture").mesh);
    s.ok("part.duplicate", json!({ "name": "copy", "part": "patch" }));
    s.ok("part.remesh", json!({ "resolution": 32 }));
    assert!(closed(&s.mesh("remeshed")), "part.remesh");

    let mut s = Session::new();
    s.add_part("patch", &gs_testkit::fixtures::patch(6));
    s.ok("part.duplicate", json!({ "name": "copy", "part": "patch" }));
    let merged = s.ok("part.merge_visible", json!({}));
    let part = merged.result["part"]["part"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let reply = s.query(
        "mesh.voxel_remesh",
        json!({ "part": part, "resolution": 32 }),
    );
    assert_eq!(reply["inside"], json!("generalized_winding"), "{reply}");
    assert!(closed(&s.mesh(&part)), "mesh.voxel_remesh");
}

/// Research 26's wrong result 7: an inside-out part winds -1 inside, and where it overlapped a
/// part facing out the sum was 0, so `part.remesh` left the overlap as a sealed hollow. Each
/// part's inside is now read from its own winding, so which way a part faces cannot matter.
#[test]
fn an_inside_out_part_remeshes_with_another_as_if_it_faced_out() {
    let remeshed = |inside_out: bool| {
        let mut s = Session::new();
        s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
        let mut a = s.mesh("sphere");
        if inside_out {
            // Turned around its first corner, so each quad keeps its diagonal.
            for f in &mut a.faces {
                let n = if f[3] == NO_VERTEX { 3 } else { 4 };
                f[1..n].reverse();
            }
        }
        s.ok("part.delete", json!({ "part": "sphere" }));
        s.add_part("a", &a);
        s.ok(
            "primitive.sphere",
            json!({ "subdivisions": 8, "radius": 0.6, "center": [0.6, 0.2, 0.1] }),
        );
        s.ok("part.remesh", json!({ "resolution": 64 }));
        s.mesh("remeshed")
    };
    let (out, inverted) = (remeshed(false), remeshed(true));
    let volume = |m: &PlainMesh| gs_testkit::measure::stats(m).volume;
    let (want, got) = (volume(&out), volume(&inverted));
    assert!(
        (got / want - 1.0).abs() <= 1e-5,
        "volume {got}, facing out {want}: the overlap is not hollow"
    );
    // The same signs give the same quads; the distances can differ in the last bits, since
    // each triangle's corners come in another order.
    assert!(inverted.faces == out.faces, "the same quads");
    let moved = (out.positions.iter().zip(&inverted.positions))
        .flat_map(|(p, q)| (0..3).map(move |a| (p[a] - q[a]).abs()))
        .fold(0.0, f32::max);
    assert!(moved <= 1e-5, "the same places, within {moved}");
}

/// The cell size a remesh with `args` reports.
fn voxel_size(s: &mut Session, args: Value) -> f64 {
    let done = s.ok("mesh.voxel_remesh", args);
    done.result["voxel_size"].as_f64().unwrap_or(0.0)
}

#[test]
fn a_part_keeps_its_cell_size_for_the_next_remesh() {
    let mut s = sphere_session();
    assert!(
        (voxel_size(&mut s, json!({})) - 2.0 / 128.0).abs() <= 1e-7,
        "a first remesh with no size: resolution 128"
    );
    let mut s = sphere_session();
    let first = voxel_size(&mut s, json!({ "resolution": 20 }));
    // The new surface's box is a little different, but the cell size stays, through other
    // edits too.
    s.ok(
        "mesh.translate",
        json!({ "direction": [1, 0, 0], "distance": 0.5 }),
    );
    assert_eq!(voxel_size(&mut s, json!({})), first, "kept");
    let set = voxel_size(&mut s, json!({ "voxel_size": 0.15 }));
    assert_eq!(
        voxel_size(&mut s, json!({})),
        set,
        "a given size is kept too"
    );
    // Undo puts the part back with the size it had kept.
    s.ok("history.undo", json!({}));
    s.ok("history.undo", json!({}));
    assert_eq!(voxel_size(&mut s, json!({})), first, "undone");
    // So does a checkpoint (STATUS item 20: restore rebuilt parts from meshes and forgot it).
    s.ok("history.checkpoint", json!({ "name": "kept" }));
    voxel_size(&mut s, json!({ "voxel_size": 0.2 }));
    s.ok("history.restore", json!({ "name": "kept" }));
    assert_eq!(voxel_size(&mut s, json!({})), first, "restored");
    // A new part has none.
    s.ok("primitive.cube", json!({}));
    let size = voxel_size(&mut s, json!({ "part": "cube" }));
    assert!((size - 2.0 / 128.0).abs() <= 1e-7, "a new part: {size}");
}

/// What a part went through, the calls that did it, and the part to measure.
type Way<'a> = (&'a str, &'a [(&'a str, Value)], &'a str);

/// Review 0008, m2: the kept cell size is in the part's own units, and a call that moves a
/// scaled part's points into the scene (`part.apply_transform`, `part.mirror_copy`,
/// `part.merge_down`) scales it with them, so the next remesh with no size lays as long a cell
/// in the scene as it did while the part was placed. Before, they kept the number and the cell
/// was half as long.
#[test]
fn the_kept_cell_size_spans_as_much_scene_after_the_points_move_into_it() {
    // A remeshed sphere scaled by 2, and the cell size its remesh kept.
    let start = || {
        let mut s = Session::new();
        s.ok("primitive.sphere", json!({ "subdivisions": 5 }));
        let first = voxel_size(&mut s, json!({ "resolution": 32 }));
        s.ok(
            "transform.apply",
            json!({ "frame": { "origin": [0, 0, 0], "axes": [0, 0, 0, 1] },
                    "op": { "scale": [2.0, 2.0, 2.0] } }),
        );
        (s, first)
    };
    let (mut s, first) = start();
    let placed = voxel_size(&mut s, json!({}));
    assert!(
        (placed - 2.0 * first).abs() <= 1e-6,
        "placed: {placed}, first {first}"
    );
    let ways: [Way; 3] = [
        ("baked", &[("part.apply_transform", json!({}))], "sphere"),
        (
            "a mirror copy",
            &[("part.mirror_copy", json!({ "name": "copy" }))],
            "copy",
        ),
        (
            "merged down",
            &[
                ("primitive.cube", json!({ "center": [4.0, 0.0, 0.0] })),
                ("part.merge_down", json!({ "part": "sphere" })),
            ],
            "sphere",
        ),
    ];
    for (what, calls, part) in ways {
        let (mut s, _) = start();
        for (command, args) in calls {
            s.ok(command, args.clone());
        }
        let size = voxel_size(&mut s, json!({ "part": part }));
        assert!(
            (size - placed).abs() <= 1e-6,
            "{what}: {size}, placed {placed}"
        );
    }
}

#[test]
fn the_same_remesh_gives_the_same_bits_in_a_new_session() {
    let args = json!({ "resolution": 24, "project": true, "blur": 10 });
    let hashes: Vec<String> = (0..2)
        .map(|_| {
            let mut s = sphere_session();
            s.ok("mesh.voxel_remesh", args.clone());
            s.hash()
        })
        .collect();
    assert_eq!(hashes[0], hashes[1], "repeatable");
}

/// A UV belongs to a face's corner, and no new face has old corners, so both remeshes drop the
/// UV columns rather than give each new face the corners of the old face nearest it.
#[test]
fn a_remesh_drops_the_uvs() {
    let uv = json!({ "domain": "face", "values": { "f32": [0.25, 0.5, 0.75, 1.0] } });
    let columns = gs_io::mesh::UV_COLUMNS.map(|name| (name.to_string(), uv.clone()));
    let attributes = Value::Object(columns.into_iter().collect());
    for (command, part) in [("mesh.voxel_remesh", "t"), ("part.remesh", "remeshed")] {
        let mut s = Session::new();
        s.ok(
            "part.create",
            json!({ "name": "t", "mesh": tetra(&attributes) }),
        );
        assert!(
            s.mesh("t").attributes.contains_key("uv0.u"),
            "the part has UVs"
        );
        s.ok(command, json!({ "resolution": 16 }));
        let names: Vec<String> = s.mesh(part).attributes.into_keys().collect();
        assert!(
            names.iter().all(|n| !n.starts_with("uv")),
            "{command}: {names:?}"
        );
    }
}
