//! `boolean.check` (decision 0057, section 7): what would make a cut fail or look wrong, per part
//! and between the operands of one group. The data behind Show Issues and Show Coplanar.

use std::sync::Arc;

use gs_protocol::{CommandError, Cost, Kind, PartRef, PlainMesh, corners, face_triangles};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::grid::{self, P};
use super::groups;
use crate::math::{cross, dot, sub};
use crate::mesh::{drawn_triangles, face_edges};
use crate::part::{names, pin_refs, shown};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// The most faces an issue lists.
const CAP: usize = 1000;

/// Check parts for what would make a boolean fail or look wrong.
///
/// Per part, in the scene: whether it is closed (every edge has two faces, one running each
/// way), its open edges, its non-manifold edges, its inside-out shells (closed, with a negative
/// volume, and held by no other shell, so a cavity is not one) and its faces that cross without
/// sharing a point. Between every two of the parts in one group (`part.set_operation`'s
/// `start`), their coplanar faces: pairs, one of each, that lie on each other and overlap, where
/// a boolean has no clean answer. Each issue is a count and the faces involved, at most 1000.
/// `mesh.validate` allows open parts on purpose; this asks what a boolean needs.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BooleanCheck {
    /// The parts. Left out or empty: every part that shows, in list order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartRef>,
    /// How near two faces must lie to each other's plane to be coplanar, times the diagonal of
    /// the two parts' bounding box together: 0 to 0.01.
    #[serde(default = "coplanar_threshold")]
    #[schemars(range(min = 0.0, max = 0.01))]
    pub coplanar_threshold: f32,
}

pub(super) fn coplanar_threshold() -> f32 {
    1e-5
}

/// What was found.
#[derive(Serialize, JsonSchema)]
pub struct Checked {
    /// Each part, in the order given.
    pub parts: Vec<PartCheck>,
    /// Each two parts of one group with coplanar faces, in the order given.
    pub coplanar: Vec<Coplanar>,
}

/// One part's issues.
#[derive(Serialize, JsonSchema)]
pub struct PartCheck {
    /// The part.
    pub part: String,
    /// It has faces, and every edge has two, one running each way.
    pub closed: bool,
    /// Edges with a face on one side only, and the faces along them.
    pub open_edges: Issue,
    /// Edges three or more faces share, that two faces run along the same way, or that join a
    /// point to itself, and the faces along them.
    pub non_manifold_edges: Issue,
    /// Closed shells that face inward and lie inside no other shell, and their faces.
    pub inside_out_shells: Issue,
    /// Pairs of faces that cross without sharing a point, and the faces.
    pub self_crossings: Issue,
}

/// How many of something, and the faces involved.
#[derive(Serialize, JsonSchema)]
pub struct Issue {
    /// How many.
    pub count: u64,
    /// The faces, by index, ascending, at most 1000.
    pub faces: Vec<u32>,
}

/// Two parts of one group with coplanar faces.
#[derive(Serialize, JsonSchema)]
pub struct Coplanar {
    /// The two parts, in the order given.
    pub parts: [String; 2],
    /// How many pairs of faces, one of each part, lie on each other and overlap.
    pub count: u64,
    /// Each part's faces among them, by index, ascending, at most 1000 each.
    pub faces: [Vec<u32>; 2],
}

impl Command for BooleanCheck {
    const NAME: &'static str = "boolean.check";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Long;
    type Output = Checked;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw("check every part that shows", json!({}))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({ "center": [1, 0, 0] })),
            Example::raw(
                "check one part, with a looser coplanar threshold",
                json!({ "parts": ["cube"], "coplanar_threshold": 0.001 }),
            )
            .after("primitive.cube", json!({})),
            Example::raw("find the faces two flush cubes share", json!({}))
                .after("primitive.cube", json!({}))
                .after("primitive.cube", json!({ "center": [2, 0, 0] })),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_refs(cx, &mut self.parts)?;
        if self.parts.is_empty() {
            self.parts = shown(cx).into_iter().map(PartRef).collect();
        }
        Ok(())
    }

    fn check(&self, _cx: &Ctx) -> Result<(), Unavailable> {
        if self.parts.is_empty() {
            return Err(Unavailable::new("no part shows")
                .fix("show a part with part.set_visible, or name the parts"));
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<Checked, CommandError> {
        let names = names(&self.parts);
        let mut meshes = Vec::with_capacity(names.len());
        for (k, name) in names.iter().enumerate() {
            cx.progress(
                Some(k as f32 / names.len() as f32),
                format!("checking {name}"),
            );
            meshes.push(cx.doc.get_part(name)?.world());
        }
        let parts = (names.iter().zip(&meshes))
            .map(|(name, mesh)| part_check(name, mesh))
            .collect();
        let threshold = f64::from(self.coplanar_threshold);
        let coplanar = coplanar(cx, &names, &meshes, threshold);
        Ok(Checked { parts, coplanar })
    }
}
register_command!(BooleanCheck, "Check for boolean issues");

pub(super) fn part_check(part: &str, mesh: &PlainMesh) -> PartCheck {
    let (open, non_manifold, broken) = edges(mesh);
    PartCheck {
        part: part.to_string(),
        closed: !mesh.faces.is_empty() && open.count == 0 && non_manifold.count == 0,
        inside_out_shells: inside_out(mesh, &broken),
        self_crossings: self_crossings(mesh),
        open_edges: open,
        non_manifold_edges: non_manifold,
    }
}

fn issue(count: usize, faces: Vec<u32>) -> Issue {
    Issue {
        count: count as u64,
        faces: capped(faces),
    }
}

fn capped(mut faces: Vec<u32>) -> Vec<u32> {
    faces.sort_unstable();
    faces.dedup();
    faces.truncate(CAP);
    faces
}

/// The open edges, the non-manifold edges, and which faces run along either.
fn edges(mesh: &PlainMesh) -> (Issue, Issue, Vec<bool>) {
    let mut all: Vec<([u32; 2], bool, u32)> = Vec::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        for (a, b) in face_edges(corners(face)) {
            all.push(([a.min(b), a.max(b)], a < b, f as u32));
        }
    }
    all.sort_unstable();
    let mut broken = vec![false; mesh.faces.len()];
    let (mut open, mut wrong) = ((0, Vec::new()), (0, Vec::new()));
    for run in all.chunk_by(|x, y| x.0 == y.0) {
        let looped = run[0].0[0] == run[0].0[1];
        if !looped && run.len() == 2 && run[0].1 != run[1].1 {
            continue;
        }
        let into = if !looped && run.len() == 1 {
            &mut open
        } else {
            &mut wrong
        };
        into.0 += 1;
        for &(_, _, f) in run {
            into.1.push(f);
            broken[f as usize] = true;
        }
    }
    (issue(open.0, open.1), issue(wrong.0, wrong.1), broken)
}

/// The closed shells (no face on a broken edge) with a negative volume that no other shell
/// holds: the other faces wind around the shell's first corner less than half a turn.
fn inside_out(mesh: &PlainMesh, broken: &[bool]) -> Issue {
    let shells = gs_geo::parts::by_shells(mesh);
    let count = shells.iter().max().map_or(0, |&s| s as usize + 1);
    let (mut volume, mut closed) = (vec![0.0_f64; count], vec![true; count]);
    for (f, face) in mesh.faces.iter().enumerate() {
        let s = shells[f] as usize;
        closed[s] &= !broken[f];
        let at = |v: &u32| mesh.positions[*v as usize].map(f64::from);
        let points: Vec<P> = corners(face).iter().map(at).collect();
        let six: f64 = drawn_triangles(&points)
            .map(|[a, b, c]| dot(a, cross(b, c)))
            .sum();
        volume[s] += six / 6.0;
    }
    let mut found = (0, Vec::new());
    for s in 0..count as u32 {
        let i = s as usize;
        if !closed[i] || volume[i] >= 0.0 || held(mesh, &shells, s) {
            continue;
        }
        found.0 += 1;
        found
            .1
            .extend((0..mesh.faces.len() as u32).filter(|&f| shells[f as usize] == s));
    }
    issue(found.0, found.1)
}

/// True when the faces outside shell `s` wind around its first corner at least half a turn.
fn held(mesh: &PlainMesh, shells: &[u32], s: u32) -> bool {
    let (mut inside, mut rest) = (None, Vec::new());
    for (face, &t) in mesh.faces.iter().zip(shells) {
        if t == s {
            inside.get_or_insert(face[0]);
        } else {
            rest.push(*face);
        }
    }
    let Some(corner) = inside.filter(|_| !rest.is_empty()) else {
        return false;
    };
    let others = PlainMesh {
        positions: mesh.positions.clone(),
        faces: rest,
        ..PlainMesh::default()
    };
    let point = [mesh.positions[corner as usize]];
    (gs_geo::voxel::winding_numbers(&others, &point))
        .is_ok_and(|w| w.first().is_some_and(|&(turns, _)| turns >= 0.5))
}

/// Each triangle as its faces are drawn, in `f64`, with its corners and its face.
fn soup(mesh: &PlainMesh) -> (Vec<[P; 3]>, Vec<[u32; 3]>, Vec<u32>) {
    let (mut points, mut tris, mut face_of) = (Vec::new(), Vec::new(), Vec::new());
    for (f, &face) in mesh.faces.iter().enumerate() {
        for t in face_triangles(face, |k| mesh.positions[face[k] as usize]) {
            let t = t.map(|k| face[k]);
            points.push(t.map(|v| mesh.positions[v as usize].map(f64::from)));
            tris.push(t);
            face_of.push(f as u32);
        }
    }
    (points, tris, face_of)
}

/// The pairs of faces that cross without sharing a point (triangles that share a corner are
/// not compared): an edge of one through the inside of the other, strictly beyond 1e-10 times
/// the mean edge cubed. Two triangles that meet only where each one's edges cross the other's
/// edges are not counted, as in research 26's bench.
fn self_crossings(mesh: &PlainMesh) -> Issue {
    let (points, tris, face_of) = soup(mesh);
    let mean = grid::mean_edge(&points);
    let eps = 1e-10 * mean * mean * mean;
    let apart = |&(s, t): &(u32, u32)| {
        let (s, t) = (s as usize, t as usize);
        !tris[s].iter().any(|v| tris[t].contains(v)) && grid::crosses(&points[s], &points[t], eps)
    };
    let mut pairs: Vec<(u32, u32)> = (grid::near(&points, 0.0, None).into_iter())
        .filter(apart)
        .map(|(s, t)| (face_of[s as usize], face_of[t as usize]))
        .filter(|(a, b)| a != b)
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    let faces = pairs.iter().flat_map(|&(a, b)| [a, b]).collect();
    issue(pairs.len(), faces)
}

/// The coplanar faces between every two of `names` in one group.
fn coplanar(
    cx: &Ctx,
    names: &[String],
    meshes: &[Arc<PlainMesh>],
    threshold: f64,
) -> Vec<Coplanar> {
    let listed = groups::listed(cx.doc.document());
    let group = |name: &str| listed.iter().find(|p| p.name == name).map(|p| p.group);
    let mut out = Vec::new();
    for i in 0..names.len() {
        for j in i + 1..names.len() {
            if names[i] == names[j] || group(&names[i]) != group(&names[j]) {
                continue;
            }
            let pair = [names[i].clone(), names[j].clone()];
            out.extend(between(pair, [&meshes[i], &meshes[j]], threshold));
        }
    }
    out
}

/// The faces of two parts that lie on each other: within `threshold` times the diagonal of the
/// two parts' bounding box together of each other's plane, and overlapping there.
pub(super) fn between(
    parts: [String; 2],
    meshes: [&PlainMesh; 2],
    threshold: f64,
) -> Option<Coplanar> {
    let ((first, _, first_face), (second, _, second_face)) = (soup(meshes[0]), soup(meshes[1]));
    let n = first.len();
    let all = [first, second].concat();
    let grow = |(lo, hi): (P, P), p: &P| -> (P, P) {
        let lo = std::array::from_fn(|a| lo[a].min(p[a]));
        (lo, std::array::from_fn(|a| hi[a].max(p[a])))
    };
    let (lo, hi) = (all.iter().flatten()).fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), grow);
    let side = sub(hi, lo);
    let depth = threshold * dot(side, side).sqrt();
    let mut pairs: Vec<(u32, u32)> = (grid::near(&all, depth, Some(n)).into_iter())
        .filter(|&(s, t)| grid::lie_on(&all[s as usize], &all[t as usize], depth))
        .map(|(s, t)| (first_face[s as usize], second_face[t as usize - n]))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    if pairs.is_empty() {
        return None;
    }
    let faces = [
        capped(pairs.iter().map(|p| p.0).collect()),
        capped(pairs.iter().map(|p| p.1).collect()),
    ];
    Some(Coplanar {
        parts,
        count: pairs.len() as u64,
        faces,
    })
}
