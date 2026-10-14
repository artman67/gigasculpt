//! gs-geo decimate tests (decision 0034): hidden faces come back as they were, paint all one
//! color weighs nothing, and a mirrored face's corner columns follow its points. The command's
//! own tests are in gs-engine.

use std::collections::BTreeMap;

use gs_geo::decimate::{Settings, decimate, mirror, unpaired};
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};

fn face_u32(values: Vec<u32>) -> Attribute {
    Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(values),
    }
}

fn face_f32(values: Vec<f32>) -> Attribute {
    Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::F32(values),
    }
}

/// An `n` by `n` grid of quads on a gentle dome, with the faces of the first row hidden.
fn dome(n: u32) -> PlainMesh {
    let side = n + 1;
    let positions = (0..side * side)
        .map(|i| {
            let (x, y) = ((i % side) as f32 / n as f32, (i / side) as f32 / n as f32);
            [x, y, 0.2 * (x * (1.0 - x) + y * (1.0 - y))]
        })
        .collect();
    let faces: Vec<[u32; 4]> = (0..n * n)
        .map(|f| {
            let v = f % n + f / n * side;
            [v, v + 1, v + side + 1, v + side]
        })
        .collect();
    let hide = (0..n * n).map(|f| u32::from(f < n)).collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::from([("hide".to_owned(), face_u32(hide))]),
    }
}

#[test]
fn hidden_faces_come_back_as_they_were_after_the_triangles() {
    let n = 24;
    let mesh = dome(n);
    let settings = Settings {
        faces: 100,
        uniform: true,
        ..Settings::default()
    };
    let out = decimate(&mesh, &settings).mesh;
    let held = n as usize;
    let shown = out.faces.len() - held;
    assert!(shown <= 100 && shown > 80, "{shown} triangles");
    let Some(AttrValues::U32(hide)) = out.attributes.get("hide").map(|a| &a.values) else {
        panic!("no hide column");
    };
    assert!(hide[..shown].iter().all(|&h| h == 0));
    assert!(hide[shown..].iter().all(|&h| h == 1));
    for (new, old) in out.faces[shown..].iter().zip(&mesh.faces[..held]) {
        let at = |m: &PlainMesh, f: &[u32; 4]| f.map(|v| m.positions[v as usize]);
        assert_eq!(at(&out, new), at(&mesh, old));
    }
    assert!(out.faces[..shown].iter().all(|f| f[3] == NO_VERTEX));
}

#[test]
fn paint_all_one_color_decimates_as_no_paint() {
    // Weighed, even a constant attribute moves meshoptimizer's choices; an unpainted part must
    // come out as it did before paint was weighed.
    let mesh = dome(24);
    let plain = Settings {
        faces: 100,
        uniform: true,
        ..Settings::default()
    };
    let white = Settings {
        paint: vec![[1.0; 3]; mesh.positions.len()],
        paint_weight: gs_geo::decimate::PAINT_WEIGHT,
        ..plain.clone()
    };
    let (a, b) = (decimate(&mesh, &plain).mesh, decimate(&mesh, &white).mesh);
    assert!(
        a.faces == b.faces && a.positions == b.positions,
        "white paint changed the result"
    );
}

#[test]
fn each_corner_keeps_the_value_it_had_at_its_point() {
    let mut mesh = dome(24);
    mesh.attributes.clear();
    let names: Vec<[String; 4]> = ["u", "v"]
        .iter()
        .map(|c| std::array::from_fn(|k| format!("uv{k}.{c}")))
        .collect();
    for (axis, group) in names.iter().enumerate() {
        for (k, name) in group.iter().enumerate() {
            let values = (mesh.faces.iter())
                .map(|f| mesh.positions[f[k] as usize][axis])
                .collect();
            mesh.attributes.insert(name.clone(), face_f32(values));
        }
    }
    let settings = Settings {
        faces: 100,
        corners: names.clone(),
        ..Settings::default()
    };
    let out = decimate(&mesh, &settings).mesh;
    for (j, f) in out.faces.iter().enumerate() {
        for (axis, group) in names.iter().enumerate() {
            for k in 0..3 {
                let AttrValues::F32(v) = &out.attributes[&group[k]].values else {
                    panic!("not f32");
                };
                let want = out.positions[f[k] as usize][axis];
                assert_eq!(v[j], want, "face {j} corner {k} axis {axis}");
            }
        }
    }
}

#[test]
fn a_mirrored_face_takes_each_corner_value_with_its_point() {
    let names = ["c0", "c1", "c2", "c3"].map(str::to_owned);
    let attributes = names
        .iter()
        .enumerate()
        .map(|(k, name)| (name.clone(), face_f32(vec![10.0 + k as f32])))
        .collect();
    let mesh = PlainMesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        faces: vec![[0, 1, 2, 3]],
        attributes,
    };
    let (out, partner) = mirror(&mesh, 0, std::slice::from_ref(&names));
    // The plane points 0 and 3 are shared; 1 and 2 get copies.
    assert_eq!(partner, [0, 4, 5, 3, 1, 2]);
    assert_eq!(out.positions[4], [-1.0, 0.0, 0.0]);
    assert_eq!(out.faces[1], [3, 5, 4, 0]);
    let corner = |k: usize, f: usize| match &out.attributes[&names[k]].values {
        AttrValues::F32(v) => v[f],
        AttrValues::U32(_) => panic!("not f32"),
    };
    for k in 0..4 {
        let original = (0..4).find(|&s| out.faces[0][s] == partner[out.faces[1][k] as usize]);
        assert_eq!(
            Some(corner(k, 1)),
            original.map(|s| corner(s, 0)),
            "corner {k}"
        );
    }
    assert_eq!(unpaired(&out.positions, 0, 1e-6), None);
    assert_eq!(unpaired(&mesh.positions, 0, 1e-6), Some(1));
}
