//! gs-geo part tests (decision 0028, section 4): split carries every column face by face and
//! merge puts it back; group ids move only on a collision; shells and the mask rule; extract
//! makes a closed shell that faces out.

use std::collections::BTreeMap;

use gs_geo::parts::{self, MergeError};
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};
use gs_testkit::fixtures::{cube, patch};
use gs_testkit::measure::stats;
use gs_testkit::validate::{Expect, validate};

fn column(domain: AttrDomain, values: AttrValues) -> Attribute {
    Attribute { domain, values }
}

/// Merge's empty value for every column: 0.
fn zero(_: &str, _: &Attribute) -> u32 {
    0
}

fn u32s(mesh: &PlainMesh, name: &str) -> Vec<u32> {
    match &mesh.attributes[name].values {
        AttrValues::U32(v) => v.clone(),
        AttrValues::F32(_) => panic!("{name} is not whole numbers"),
    }
}

/// A 2 by 2 cube with every kind of column: a mask and a color per vertex, and per face the
/// groups 7 (centers with x > 0) and 3 (the rest), and a hidden bit on every fifth face.
fn painted_cube() -> PlainMesh {
    let mut mesh = cube(2);
    let (nv, nf) = (mesh.positions.len(), mesh.faces.len());
    let center_x = |f: &[u32; 4]| {
        f.iter()
            .map(|&v| mesh.positions[v as usize][0])
            .sum::<f32>()
    };
    let group = mesh
        .faces
        .iter()
        .map(|f| if center_x(f) > 0.0 { 7 } else { 3 })
        .collect();
    let attributes = [
        (
            "mask",
            AttrDomain::Vertex,
            AttrValues::F32((0..nv).map(|i| i as f32 / 32.0).collect()),
        ),
        (
            "color",
            AttrDomain::Vertex,
            AttrValues::U32((0..nv as u32).map(|i| i * 0x0101_0101).collect()),
        ),
        ("group", AttrDomain::Face, AttrValues::U32(group)),
        (
            "hide",
            AttrDomain::Face,
            AttrValues::U32((0..nf as u32).map(|f| u32::from(f % 5 == 0)).collect()),
        ),
    ];
    for (name, domain, values) in attributes {
        mesh.attributes
            .insert(name.to_string(), column(domain, values));
    }
    mesh
}

/// Everything face `f` of `mesh` says: its corners' positions, its corners' vertex values and
/// its face values, column by column.
fn face_record(mesh: &PlainMesh, f: usize) -> Vec<String> {
    let corners: Vec<u32> = mesh.faces[f]
        .into_iter()
        .filter(|&v| v != NO_VERTEX)
        .collect();
    let mut record = vec![format!(
        "{:?}",
        corners
            .iter()
            .map(|&v| mesh.positions[v as usize])
            .collect::<Vec<_>>()
    )];
    for (name, col) in &mesh.attributes {
        let at: Vec<usize> = match col.domain {
            AttrDomain::Vertex => corners.iter().map(|&v| v as usize).collect(),
            AttrDomain::Face => vec![f],
        };
        let values = match &col.values {
            AttrValues::F32(v) => format!("{:?}", at.iter().map(|&i| v[i]).collect::<Vec<_>>()),
            AttrValues::U32(v) => format!("{:?}", at.iter().map(|&i| v[i]).collect::<Vec<_>>()),
        };
        record.push(format!("{name}={values}"));
    }
    record
}

#[test]
fn split_by_groups_carries_every_column_and_merge_puts_it_back() {
    let mesh = painted_cube();
    let labels = parts::by_groups(&mesh);
    let pieces = parts::split(&mesh, &labels);
    assert_eq!(
        pieces.iter().map(|p| p.label).collect::<Vec<_>>(),
        [labels[0], 10 - labels[0]]
    );

    // Each piece's faces are the original's faces of its label, in order, column for column.
    let mut order = Vec::new();
    for piece in &pieces {
        let theirs: Vec<usize> = (0..mesh.faces.len())
            .filter(|&f| labels[f] == piece.label)
            .collect();
        assert_eq!(piece.mesh.faces.len(), theirs.len());
        assert_eq!(
            validate(&piece.mesh, Expect::ANY),
            Ok(()),
            "a piece is well formed"
        );
        for (k, &f) in theirs.iter().enumerate() {
            assert_eq!(
                face_record(&piece.mesh, k),
                face_record(&mesh, f),
                "piece {} face {k}",
                piece.label
            );
        }
        order.extend(theirs);
    }
    assert_eq!(order.len(), mesh.faces.len(), "the faces add up exactly");
    let seam = pieces.iter().map(|p| p.mesh.positions.len()).sum::<usize>() - mesh.positions.len();
    assert!(seam > 0, "the vertices on the seam go into both pieces");

    // Merging the pieces gives the original faces back, face for face, with the same group ids.
    let merged = parts::merge(&pieces.iter().map(|p| &p.mesh).collect::<Vec<_>>(), zero).unwrap();
    assert_eq!(merged.faces.len(), mesh.faces.len());
    assert_eq!(
        merged.attributes.keys().collect::<Vec<_>>(),
        mesh.attributes.keys().collect::<Vec<_>>()
    );
    for (k, &f) in order.iter().enumerate() {
        assert_eq!(
            face_record(&merged, k),
            face_record(&mesh, f),
            "merged face {k}"
        );
    }
}

#[test]
fn group_ids_move_up_only_when_they_collide() {
    let with_groups = |ids: &[u32]| {
        let mut mesh = cube(1);
        let values = (0..6).map(|f| ids[f % ids.len()]).collect();
        mesh.attributes.insert(
            "group".into(),
            column(AttrDomain::Face, AttrValues::U32(values)),
        );
        mesh
    };
    let groups = |meshes: &[&PlainMesh]| u32s(&parts::merge(meshes, zero).unwrap(), "group");

    let (a, b, c) = (
        with_groups(&[1, 2]),
        with_groups(&[3]),
        with_groups(&[0, 2]),
    );
    assert_eq!(
        groups(&[&a, &b]),
        [1, 2, 1, 2, 1, 2, 3, 3, 3, 3, 3, 3],
        "no collision: kept"
    );
    assert_eq!(
        groups(&[&a, &a]),
        [1, 2, 1, 2, 1, 2, 3, 4, 3, 4, 3, 4],
        "moved up by 2"
    );
    assert_eq!(
        groups(&[&a, &b, &c]),
        [1, 2, 1, 2, 1, 2, 3, 3, 3, 3, 3, 3, 0, 5, 0, 5, 0, 5],
        "0 stays 0 and the rest move up by the largest id so far"
    );
    let plain = cube(1);
    assert_eq!(
        groups(&[&plain, &a])[..6],
        [0; 6],
        "a mesh without groups reads 0"
    );

    let mut clash = cube(1);
    clash.attributes.insert(
        "group".into(),
        column(AttrDomain::Vertex, AttrValues::U32(vec![1; 8])),
    );
    assert_eq!(
        parts::merge(&[&a, &clash], zero).unwrap_err(),
        MergeError::Column("group".into())
    );
}

#[test]
fn shells_are_joined_by_edges_not_by_points() {
    let far = |mesh: &PlainMesh, by: f32| PlainMesh {
        positions: mesh.positions.iter().map(|p| p.map(|c| c + by)).collect(),
        ..mesh.clone()
    };
    let one = cube(1);
    let two = parts::merge(&[&one, &far(&one, 5.0)], zero).unwrap();
    assert_eq!(parts::by_shells(&two), [[0; 6], [1; 6]].concat());

    // A cube touching another at one corner, the corner shared: still two shells.
    let mut touching = parts::merge(&[&far(&one, 2.0), &one], zero).unwrap();
    // The far cube's lowest corner and this cube's highest are both the point (1, 1, 1).
    let find = |half: std::ops::Range<usize>, at: fn(f32) -> bool| {
        half.into_iter()
            .find(|&v| touching.positions[v].iter().all(|&c| at(c)))
            .unwrap() as u32
    };
    let (theirs, ours) = (find(0..8, |c| c < 1.5), find(8..16, |c| c > 0.5));
    for face in &mut touching.faces[6..] {
        *face = face.map(|v| if v == ours { theirs } else { v });
    }
    assert_eq!(parts::by_shells(&touching), [[0; 6], [1; 6]].concat());
    let pieces = parts::split(&touching, &parts::by_shells(&touching));
    assert_eq!(
        pieces
            .iter()
            .map(|p| p.mesh.positions.len())
            .collect::<Vec<_>>(),
        [8, 8],
        "the shared corner goes to both"
    );
}

#[test]
fn a_face_is_masked_when_its_corner_mean_reaches_the_threshold() {
    let mut mesh = PlainMesh {
        positions: vec![
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
        ],
        faces: vec![[0, 1, 2, 3], [1, 4, 2, NO_VERTEX]],
        attributes: BTreeMap::new(),
    };
    assert_eq!(
        parts::by_mask(&mesh, 0.5),
        [0, 0],
        "no mask column: unmasked"
    );
    let mask = vec![0.5, 0.5, 0.5, 0.5, 0.0];
    mesh.attributes.insert(
        "mask".into(),
        column(AttrDomain::Vertex, AttrValues::F32(mask)),
    );
    assert_eq!(
        parts::by_mask(&mesh, 0.5),
        [1, 0],
        "a mean of exactly the threshold counts"
    );
    assert_eq!(
        parts::by_mask(&mesh, 0.333),
        [1, 1],
        "the triangle's mean is 1/3"
    );
    assert_eq!(parts::by_mask(&mesh, 0.34), [1, 0]);
    assert_eq!(parts::by_mask(&mesh, 0.51), [0, 0]);
}

#[test]
fn extract_makes_a_closed_shell_that_faces_out() {
    let mut mesh = patch(4);
    let nv = mesh.positions.len();
    for (name, domain, values) in [
        ("mask", AttrDomain::Vertex, AttrValues::F32(vec![1.0; nv])),
        (
            "color",
            AttrDomain::Vertex,
            AttrValues::U32((0..nv as u32).collect()),
        ),
        ("group", AttrDomain::Face, AttrValues::U32(vec![9; 16])),
        (
            "label",
            AttrDomain::Face,
            AttrValues::U32((0..16).collect()),
        ),
    ] {
        mesh.attributes
            .insert(name.to_string(), column(domain, values));
    }
    let all: Vec<u32> = (0..16).collect();
    let corner: Vec<u32> = vec![0, 1, 4, 5]; // a 2 by 2 block of quads at one corner
    for (faces, rim) in [(&all, 16), (&corner, 8)] {
        for (thickness, double) in [(0.1, false), (-0.1, false), (0.05, true)] {
            let shell = parts::extract(&mesh, faces, thickness, double);
            let n = faces.len();
            assert_eq!(
                validate(&shell, Expect::closed(2)),
                Ok(()),
                "{n} faces, {thickness}, {double}"
            );
            let s = stats(&shell);
            assert!(s.volume > 0.0, "the shell faces out: volume {}", s.volume);
            assert!(
                (s.volume - s.area / 2.0 * 0.1).abs() < 0.5 * s.volume,
                "about area times thickness"
            );
            assert_eq!(
                u32s(&shell, "group"),
                [vec![1; n], vec![2; n], vec![3; rim]].concat()
            );
            assert!(
                !shell.attributes.contains_key("mask"),
                "the shell starts unmasked"
            );
            let color = u32s(&shell, "color");
            assert_eq!(
                color[..color.len() / 2],
                color[color.len() / 2..],
                "both layers carry the vertex columns"
            );
            let label = u32s(&shell, "label");
            assert_eq!(label[..n], faces[..], "the outer layer's face columns");
            assert_eq!(label[n..2 * n], faces[..], "the inner layer's");
            assert!(
                label[2 * n..].iter().all(|f| faces.contains(f)),
                "the rim's from beside it"
            );
        }
    }
}
