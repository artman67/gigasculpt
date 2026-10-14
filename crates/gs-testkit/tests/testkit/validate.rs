//! The validator passes good meshes and reports each kind of damage as exactly that kind.

use gs_protocol::{AttrValues, PlainMesh};
use gs_testkit::fixtures;
use gs_testkit::{Defect, Expect, validate};

use crate::data::{TRI, joined, moved, tetra, triangles};

fn defects(mesh: &PlainMesh, expect: Expect) -> Vec<Defect> {
    validate(mesh, expect).err().unwrap_or_default()
}

#[test]
fn good_meshes_pass() {
    assert_eq!(validate(&tetra(), Expect::closed(2)), Ok(()), "tetrahedron");
    assert_eq!(
        validate(&PlainMesh::default(), Expect::ANY),
        Ok(()),
        "empty"
    );
    for n in 1..=3 {
        assert_eq!(
            validate(&fixtures::cube(n), Expect::closed(2)),
            Ok(()),
            "cube {n}"
        );
        assert_eq!(
            validate(&fixtures::patch(n), Expect::open(1)),
            Ok(()),
            "patch {n}"
        );
    }
    for fixture in fixtures::all() {
        assert_eq!(
            validate(&fixture.mesh, fixture.expect),
            Ok(()),
            "{}",
            fixture.name
        );
    }
}

type Damaged = Vec<(&'static str, PlainMesh, Vec<Defect>)>;

fn edit(change: &dyn Fn(&mut PlainMesh)) -> PlainMesh {
    let mut m = tetra();
    change(&mut m);
    m
}

/// One kind of damage to positions or faces each, with the defects it must produce.
fn damaged_geometry() -> Damaged {
    vec![
        (
            "NaN",
            edit(&|m| m.positions[2][1] = f32::NAN),
            vec![Defect::NonFinitePosition(2)],
        ),
        (
            "inf",
            edit(&|m| m.positions[0][0] = f32::INFINITY),
            vec![Defect::NonFinitePosition(0)],
        ),
        (
            "marker",
            edit(&|m| m.faces[0] = [0, TRI, 2, 1]),
            vec![Defect::MisplacedMarker(0)],
        ),
        (
            "range",
            edit(&|m| m.faces[3] = [1, 2, 4, TRI]),
            vec![Defect::IndexOutOfRange(3)],
        ),
        (
            "repeat",
            edit(&|m| m.faces[3] = [1, 2, 2, TRI]),
            vec![Defect::RepeatedCorner(3)],
        ),
        (
            "quad repeat",
            edit(&|m| m.faces[1] = [0, 1, 3, 1]),
            vec![Defect::RepeatedCorner(1)],
        ),
        (
            "unused",
            edit(&|m| {
                m.attributes.clear();
                m.positions.push([5.0, 5.0, 5.0]);
            }),
            vec![Defect::UnusedVertex(4)],
        ),
    ]
}

/// One kind of damage to attributes each, with the defects it must produce.
fn damaged_attributes() -> Damaged {
    let name = |s: &str| s.to_string();
    vec![
        (
            "short mask",
            edit(&|m| {
                if let Some(a) = m.attributes.get_mut("mask") {
                    a.values = AttrValues::F32(vec![0.0; 3]);
                }
            }),
            vec![Defect::AttributeLength {
                name: name("mask"),
                found: 3,
                expected: 4,
            }],
        ),
        (
            "long group",
            edit(&|m| {
                if let Some(a) = m.attributes.get_mut("group") {
                    a.values = AttrValues::U32(vec![0; 5]);
                }
            }),
            vec![Defect::AttributeLength {
                name: name("group"),
                found: 5,
                expected: 4,
            }],
        ),
        (
            "NaN mask",
            edit(&|m| {
                if let Some(a) = m.attributes.get_mut("mask") {
                    a.values = AttrValues::F32(vec![0.0, 0.0, f32::NAN, 0.0]);
                }
            }),
            vec![Defect::NonFiniteAttribute {
                name: name("mask"),
                index: 2,
            }],
        ),
    ]
}

#[test]
fn every_kind_of_damage_is_reported_as_itself() {
    for (what, mesh, want) in damaged_geometry().into_iter().chain(damaged_attributes()) {
        assert_eq!(defects(&mesh, Expect::ANY), want, "{what}");
    }
}

/// Faces of zero area are legal (sculpting flattens faces on purpose): a collinear face and a
/// face whose corners coincide are not defects.
#[test]
fn zero_area_faces_are_not_defects() {
    let collinear = triangles(
        &[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [2.0, 2.0, 2.0]],
        &[[0, 1, 2]],
    );
    assert_eq!(validate(&collinear, Expect::ANY), Ok(()), "collinear");
    let collapsed = triangles(
        &[[1.0, 2.0, 3.0], [1.0, 2.0, 3.0], [1.0, 2.0, 3.0]],
        &[[0, 1, 2]],
    );
    assert_eq!(validate(&collapsed, Expect::ANY), Ok(()), "one point");
}

#[test]
fn manifold_damage_is_reported_only_where_declared() {
    let mut fin = tetra();
    fin.attributes.clear();
    fin.positions.push([5.0, 5.0, 5.0]);
    fin.faces.push([0, 1, 4, TRI]);
    assert_eq!(validate(&fin, Expect::ANY), Ok(()), "not declared");
    assert_eq!(
        defects(&fin, Expect::closed(2)),
        [
            Defect::NonManifoldEdge(0, 1),
            Defect::FlippedEdge(0, 1),
            Defect::BoundaryEdge(0, 4),
            Defect::BoundaryEdge(1, 4),
        ],
        "a fin on an edge"
    );
    let mut flipped = tetra();
    flipped.faces[3] = [1, 3, 2, TRI];
    assert_eq!(
        defects(&flipped, Expect::closed(2)),
        [
            Defect::FlippedEdge(1, 2),
            Defect::FlippedEdge(1, 3),
            Defect::FlippedEdge(2, 3)
        ],
        "one face wound the wrong way"
    );
}

/// Two closed shapes touching at one vertex: every edge is fine, the vertex is not.
#[test]
fn a_shared_vertex_between_two_cones_is_not_manifold() {
    let mut a = tetra();
    a.attributes.clear();
    let b = moved(&a, [-1.0, 0.0, 0.0]);
    let mut pinched = joined(&a, &b);
    // b's vertex 1 lands on a's vertex 0; merge it.
    pinched.positions.remove(5);
    for face in &mut pinched.faces[4..] {
        for v in face.iter_mut().filter(|v| **v != TRI) {
            *v = match *v {
                5 => 0,
                v if v > 5 => v - 1,
                v => v,
            };
        }
    }
    assert_eq!(
        validate(&joined(&a, &b), Expect::closed(4)),
        Ok(()),
        "apart: two spheres"
    );
    assert_eq!(
        defects(&pinched, Expect::closed(3)),
        [Defect::NonManifoldVertex(0)],
        "touching at a vertex"
    );
    assert_eq!(defects(&pinched, Expect::ANY), [], "only when declared");
}

#[test]
fn openness_and_euler_characteristic_are_checked_where_declared() {
    let patch = fixtures::patch(2);
    let boundary = defects(&patch, Expect::closed(1));
    assert_eq!(boundary.len(), 8, "the rim of a 2x2 patch: {boundary:?}");
    assert!(
        boundary
            .iter()
            .all(|d| matches!(d, Defect::BoundaryEdge(..))),
        "{boundary:?}"
    );
    assert_eq!(
        defects(&fixtures::torus(), Expect::closed(2)),
        [Defect::Euler {
            expected: 2,
            found: 0
        }],
        "a torus is not a sphere"
    );
    assert_eq!(
        defects(&fixtures::torus(), Expect::open(0)),
        [],
        "open allows closed meshes"
    );
}
