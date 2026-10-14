//! One quad split for every place that splits a quad (decision 0040, section 2): the shorter
//! diagonal, with a tie-break mirroring cannot change, so a symmetrized, sculpted part is drawn
//! and picked alike on both sides of its plane.

use std::collections::BTreeSet;

use gs_core::{MeshStore, NodeConfig};
use gs_protocol::PlainMesh;
use gs_render::{AdapterKind, PoolBytes, RenderMesh, Renderer, UploadOptions};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::{line, sphere_session, stroke};

/// A sphere sculpted on the positive x side, where the strokes stretch and shear its quads,
/// then made exactly symmetric across x.
fn symmetrized() -> Session {
    let on = |p: [f32; 3]| {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        p.map(|c| c / l)
    };
    let mut s = sphere_session(None);
    let strokes = [
        ("Draw", [0.3, -0.4, 0.8], [0.6, 0.4, 0.7]),
        ("Move", [0.5, 0.0, 0.8], [0.7, 0.3, 0.4]),
        ("Clay Build", [0.2, 0.6, 0.7], [0.8, -0.2, 0.5]),
    ];
    for (brush, a, b) in strokes {
        let points = line(on(a), on(b), 12);
        s.ok(
            "brush.stroke",
            stroke(brush, &points, &json!({ "size": 0.3 })),
        );
    }
    s.ok("mesh.symmetrize", json!({ "axis": "x" }));
    s
}

/// A triangle as its three corners' positions in bits, sorted, so that a triangle and its
/// mirror image's mirror compare equal whichever corner each starts at.
type Corners = [[u32; 3]; 3];

/// The triangles in `pools`, as [`Corners`].
fn triangles(pools: &PoolBytes) -> Vec<Corners> {
    let word = |b: &[u8], i: usize| u32::from_le_bytes(std::array::from_fn(|k| b[4 * i + k]));
    let at =
        |v: u32| -> [u32; 3] { std::array::from_fn(|k| word(&pools.vertices, 4 * v as usize + k)) };
    (0..pools.indices.len() / 12)
        .map(|t| {
            let mut c: Corners = std::array::from_fn(|k| at(word(&pools.indices, 3 * t + k)));
            c.sort_unstable();
            c
        })
        .collect()
}

/// The mirror image of `t` across x = 0, with 0 for -0 so that points on the plane are their
/// own image.
fn mirrored(t: &Corners) -> Corners {
    let mut c = t.map(|[x, y, z]| [(-f32::from_bits(x) + 0.0).to_bits(), y, z]);
    c.sort_unstable();
    c
}

/// The triangles of `drawn` whose mirror image is not drawn.
fn unmatched(drawn: &[Corners]) -> Vec<Corners> {
    let all: BTreeSet<&Corners> = drawn.iter().collect();
    (drawn.iter())
        .filter(|t| !all.contains(&mirrored(t)))
        .copied()
        .collect()
}

/// Decision 0040, section 2 (registry property): every quad of a symmetrized, sculpted part
/// and its mirror image are drawn as mirror-image triangles, from the part's store and from its
/// plain mesh. Symmetrize winds a mirrored face from the other end, so a split that follows the
/// corner order splits every quad and its image along different diagonals.
#[test]
fn mirror_image_quads_of_a_symmetrized_sculpted_part_are_drawn_alike() {
    let plain: PlainMesh = symmetrized().mesh("sphere");
    let quads = plain
        .faces
        .iter()
        .filter(|f| gs_protocol::arity(**f) == 4)
        .count();
    assert!(quads > 1000, "{quads} quads");
    let r = Renderer::new(AdapterKind::Lavapipe).unwrap_or_else(|e| panic!("{e}"));
    let store = MeshStore::from_plain(&plain, NodeConfig::default()).expect("a valid mesh");
    let from_plain =
        (r.upload(RenderMesh::from(&plain), UploadOptions::default())).expect("uploads");
    for (path, mesh) in [
        ("store", r.upload_store(&store)),
        ("plain mesh", from_plain),
    ] {
        let drawn = triangles(&r.read_pools(&mesh).expect("reads back"));
        let odd = unmatched(&drawn);
        assert!(
            odd.is_empty(),
            "from the {path}, {} of {} triangles have no mirror image drawn, as {:?}",
            odd.len(),
            drawn.len(),
            odd[0].map(|p| p.map(f32::from_bits))
        );
    }
}

/// A hit's position and distance, or `None` for a miss.
fn hit(s: &mut Session, origin: [f32; 3], direction: [f32; 3]) -> Option<([f32; 3], f32)> {
    let ask = json!({ "origin": origin, "direction": direction });
    let found = s.query("pick.ray", ask);
    let h = &found["hit"];
    let num = |v: &Value| {
        v.as_f64()
            .unwrap_or_else(|| panic!("not a number: {found}")) as f32
    };
    h.is_object().then(|| {
        let at = std::array::from_fn(|k| num(&h["position"][k]));
        (at, num(&h["distance"]))
    })
}

/// Decision 0040, section 2 (metamorphic): `pick.ray` at mirrored points of a symmetrized,
/// sculpted part gives mirrored hits, within rounding. The two triangles of a quad that is not
/// flat meet at its diagonal, so a mirror-image quad split the other way is hit elsewhere.
#[test]
fn pick_ray_at_mirrored_points_gives_mirrored_hits() {
    let mut s = symmetrized();
    let (mut worst, mut hits) = (0.0f32, 0);
    for i in 0..16 {
        for j in 0..16 {
            let (x, y) = (0.02 + 0.06 * i as f32, -0.9 + 0.12 * j as f32);
            for d in [[0.0, 0.0, -1.0], [-0.4, 0.2, -1.0]] {
                let a = hit(&mut s, [x, y, 3.0], d);
                let b = hit(&mut s, [-x, y, 3.0], [-d[0], d[1], d[2]]);
                match (a, b) {
                    (None, None) => {}
                    (Some((p, t)), Some((q, u))) => {
                        hits += 1;
                        let image = [-q[0], q[1], q[2]];
                        let gap = (0..3)
                            .map(|k| (p[k] - image[k]).abs())
                            .fold((t - u).abs(), f32::max);
                        worst = worst.max(gap);
                    }
                    _ => panic!("the ray from [{x}, {y}, 3] along {d:?} hits on one side only"),
                }
            }
        }
    }
    assert!(hits > 200, "{hits} hits");
    assert!(worst < 1e-5, "mirrored hits land {worst} apart");
}
