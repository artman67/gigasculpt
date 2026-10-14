//! The mesh validator: everything structurally wrong with a `PlainMesh`.
//!
//! Always checked: finite coordinates, the triangle marker only in the last slot, indices in
//! range, no corner repeated within a face, no unused vertex, attribute
//! lengths and finite attribute values. Checked where declared ([`Expect`]): manifold (every
//! edge in at most two faces that wind the same way, and the faces around each vertex forming
//! one fan), closed (no boundary edges), and the Euler characteristic `V - E + F`.
//!
//! Written independently of the engine's `mesh.validate`, so each can check the other.

use std::collections::{BTreeMap, BTreeSet};

use gs_protocol::{AttrDomain, AttrValues, NO_VERTEX, PlainMesh};

use crate::measure::{corners, edges};

/// What a mesh is declared to be, beyond the checks every mesh gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expect {
    /// Every edge in at most two faces, neighbouring faces wound the same way, and the faces
    /// around every vertex form a single fan.
    pub manifold: bool,
    /// No boundary edges: every edge in exactly two faces.
    pub closed: bool,
    /// The Euler characteristic `V - E + F`: 2 for a sphere, 0 for a torus, 1 for a disc.
    pub euler: Option<i64>,
}

impl Expect {
    /// Only the checks every mesh gets.
    pub const ANY: Self = Self {
        manifold: false,
        closed: false,
        euler: None,
    };

    /// A closed manifold with Euler characteristic `euler`.
    pub const fn closed(euler: i64) -> Self {
        Self {
            manifold: true,
            closed: true,
            euler: Some(euler),
        }
    }

    /// A manifold with boundaries and Euler characteristic `euler`.
    pub const fn open(euler: i64) -> Self {
        Self {
            manifold: true,
            closed: false,
            euler: Some(euler),
        }
    }
}

/// One thing wrong with a mesh. Vertex and face numbers are indices; an edge is its two
/// vertices, lower first.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Defect {
    /// A vertex has a coordinate that is NaN or infinite.
    NonFinitePosition(usize),
    /// A face has the triangle marker in one of its first three slots.
    MisplacedMarker(usize),
    /// A face uses a vertex index past the end of the vertex list.
    IndexOutOfRange(usize),
    /// A face uses the same vertex twice.
    RepeatedCorner(usize),
    /// No face uses this vertex.
    UnusedVertex(usize),
    /// An attribute has the wrong number of values for its domain.
    AttributeLength {
        /// The attribute.
        name: String,
        /// How many values it has.
        found: usize,
        /// How many it should have.
        expected: usize,
    },
    /// An attribute value is NaN or infinite.
    NonFiniteAttribute {
        /// The attribute.
        name: String,
        /// The value's index.
        index: usize,
    },
    /// An edge is used by more than two faces.
    NonManifoldEdge(u32, u32),
    /// Two faces run along an edge in the same direction, so their windings disagree.
    FlippedEdge(u32, u32),
    /// The faces around a vertex form more than one fan (two cones touching at a point).
    NonManifoldVertex(u32),
    /// An edge used by only one face, in a mesh declared closed.
    BoundaryEdge(u32, u32),
    /// The Euler characteristic is not the declared one.
    Euler {
        /// Declared.
        expected: i64,
        /// Found.
        found: i64,
    },
}

/// Every defect in `mesh`, sorted, or `Ok` if there are none.
pub fn validate(mesh: &PlainMesh, expect: Expect) -> Result<(), Vec<Defect>> {
    let mut found = Vec::new();
    for (v, p) in mesh.positions.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            found.push(Defect::NonFinitePosition(v));
        }
    }
    let usable = check_faces(mesh, &mut found);
    check_attributes(mesh, &mut found);
    if expect != Expect::ANY {
        check_topology(mesh, &usable, expect, &mut found);
    }
    found.sort();
    if found.is_empty() { Ok(()) } else { Err(found) }
}

/// Per-face checks and unused vertices. Returns the faces whose corners are usable for
/// topology: valid, distinct indices.
fn check_faces(mesh: &PlainMesh, found: &mut Vec<Defect>) -> Vec<usize> {
    let mut used = vec![false; mesh.positions.len()];
    let mut usable = Vec::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let corners = corners(face);
        if corners.contains(&NO_VERTEX) {
            found.push(Defect::MisplacedMarker(f));
            continue;
        }
        if corners.iter().any(|&v| v as usize >= used.len()) {
            found.push(Defect::IndexOutOfRange(f));
            continue;
        }
        for &v in corners {
            used[v as usize] = true;
        }
        let distinct: BTreeSet<&u32> = corners.iter().collect();
        if distinct.len() != corners.len() {
            found.push(Defect::RepeatedCorner(f));
            continue;
        }
        usable.push(f);
    }
    for (v, _) in used.iter().enumerate().filter(|(_, used)| !**used) {
        found.push(Defect::UnusedVertex(v));
    }
    usable
}

fn check_attributes(mesh: &PlainMesh, found: &mut Vec<Defect>) {
    for (name, attribute) in &mesh.attributes {
        let expected = match attribute.domain {
            AttrDomain::Vertex => mesh.positions.len(),
            AttrDomain::Face => mesh.faces.len(),
        };
        let (len, floats) = match &attribute.values {
            AttrValues::F32(values) => (values.len(), values.as_slice()),
            AttrValues::U32(values) => (values.len(), [].as_slice()),
        };
        if len != expected {
            found.push(Defect::AttributeLength {
                name: name.clone(),
                found: len,
                expected,
            });
        }
        for (index, _) in floats.iter().enumerate().filter(|(_, v)| !v.is_finite()) {
            let name = name.clone();
            found.push(Defect::NonFiniteAttribute { name, index });
        }
    }
}

/// The corners of the usable faces, and how they meet along edges.
struct Corners {
    /// The vertex of each corner; a corner's id is its index here.
    vertex: Vec<u32>,
    /// How many faces use each undirected edge (lower vertex first).
    edges: BTreeMap<(u32, u32), u32>,
    /// How many times each directed edge is used.
    directed: BTreeMap<(u32, u32), u32>,
    /// For each undirected edge and each of its two vertices, the corners at that vertex of
    /// the faces along the edge.
    links: BTreeMap<((u32, u32), u32), Vec<usize>>,
}

fn corners_of(mesh: &PlainMesh, usable: &[usize]) -> Corners {
    let mut out = Corners {
        vertex: Vec::new(),
        edges: BTreeMap::new(),
        directed: BTreeMap::new(),
        links: BTreeMap::new(),
    };
    for &f in usable {
        let first = out.vertex.len();
        let corners = corners(&mesh.faces[f]);
        out.vertex.extend(corners);
        let ids: Vec<usize> = (first..out.vertex.len()).collect();
        let next = ids.iter().cycle().skip(1);
        for ((a, b), (&ia, &ib)) in edges(corners).zip(ids.iter().zip(next)) {
            let key = (a.min(b), a.max(b));
            *out.edges.entry(key).or_default() += 1;
            *out.directed.entry((a, b)).or_default() += 1;
            out.links.entry((key, a)).or_default().push(ia);
            out.links.entry((key, b)).or_default().push(ib);
        }
    }
    out
}

fn check_topology(mesh: &PlainMesh, usable: &[usize], expect: Expect, found: &mut Vec<Defect>) {
    let corners = corners_of(mesh, usable);
    for (&(a, b), &faces) in &corners.edges {
        if expect.manifold && faces >= 3 {
            found.push(Defect::NonManifoldEdge(a, b));
        }
        if expect.closed && faces == 1 {
            found.push(Defect::BoundaryEdge(a, b));
        }
    }
    if expect.manifold {
        for (&(a, b), &uses) in &corners.directed {
            if uses >= 2 {
                found.push(Defect::FlippedEdge(a.min(b), a.max(b)));
            }
        }
        found.extend(fans(&corners).map(Defect::NonManifoldVertex));
    }
    if let Some(expected) = expect.euler {
        let count = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
        let euler =
            count(mesh.positions.len()) - count(corners.edges.len()) + count(mesh.faces.len());
        if euler != expected {
            found.push(Defect::Euler {
                expected,
                found: euler,
            });
        }
    }
}

/// Vertices whose faces form more than one fan. Corners at the same vertex are joined when
/// their faces share an edge at that vertex; a vertex is fine when all its corners end up in
/// one group.
fn fans(corners: &Corners) -> impl Iterator<Item = u32> {
    let mut parent: Vec<usize> = (0..corners.vertex.len()).collect();
    for ids in corners.links.values() {
        for &id in &ids[1..] {
            join(&mut parent, ids[0], id);
        }
    }
    let mut groups: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
    for (id, &v) in corners.vertex.iter().enumerate() {
        let root = find(&mut parent, id);
        groups.entry(v).or_default().insert(root);
    }
    groups
        .into_iter()
        .filter(|(_, roots)| roots.len() >= 2)
        .map(|(v, _)| v)
}

fn find(parent: &mut [usize], x: usize) -> usize {
    let up = parent[x];
    if up == x {
        return x;
    }
    let root = find(parent, up);
    parent[x] = root;
    root
}

fn join(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    parent[a.max(b)] = a.min(b);
}
