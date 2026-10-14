//! Making a part fit for the program (decision 0038, section 3, steps 2 to 6): triangles,
//! welded and repaired relative to their edge length, tiny shells dropped, lightened above
//! [`MOST_TRIANGLES`], open rims resampled to about the quad size, and the sharp edges found.

use std::collections::BTreeSet;

use gs_protocol::{
    AttrDomain, AttrValues, Attribute, GROUP, MASK, NO_VERTEX, PlainMesh, quad_diagonal,
};

use super::{Preparation, Settings};
use crate::decimate::{self, Settings as Lighten};
use crate::math::{cross, dist2, dot, sub};
use crate::weld;

/// Inputs above this many triangles are decimated to it first (research 20 section 7.8).
pub const MOST_TRIANGLES: usize = 200_000;
/// Points closer than this fraction of the median edge merge.
const WELD: f32 = 0.01;
/// Shells of fewer faces are dropped; the program drops them anyway.
const TINY_SHELL: usize = 10;
/// Each point's welded input point, carried through repair and decimation as a point column.
const ORIGIN: &str = "quad.origin";

/// One edge for the sharp-edge file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sharp {
    /// The triangle, in file order.
    pub face: u32,
    /// The edge, from corner `slot` to the next.
    pub slot: u8,
    /// Whether the surface turns outward across it.
    pub convex: bool,
}

/// Triangles ready for step 1.
#[derive(Debug, Default)]
pub(crate) struct Tris {
    pub positions: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    /// Each triangle's group, empty when groups are not kept.
    pub group: Vec<u32>,
    /// Each point's welded input point.
    pub origin: Vec<u32>,
}

/// `mesh` made ready for step 1 for `settings`, the sharp edges, and what it took.
pub(crate) fn prepare(mesh: &PlainMesh, settings: &Settings) -> (Tris, Vec<Sharp>, Preparation) {
    let mut report = Preparation::default();
    let (welded, stand_in) = welded(mesh, settings.keep_groups, &mut report);
    let at = |v: u32| stand_in.get(v as usize).copied().unwrap_or(NO_VERTEX);
    let creases: BTreeSet<(u32, u32)> = (settings.creases.iter())
        .map(|&[a, b]| pair(at(a), at(b)))
        .filter(|&(_, b)| b != NO_VERTEX)
        .filter(|(a, b)| a != b)
        .collect();
    let crease_points: BTreeSet<u32> = creases.iter().flat_map(|&(a, b)| [a, b]).collect();
    let (repaired, _, _) = decimate::repair(&welded);
    let mut t = without_tiny_shells(&Tris::from_mesh(&repaired), &mut report);
    if t.faces.len() > MOST_TRIANGLES {
        report.decimated_from = Some(t.faces.len() as u64);
        t = lightened(&t, settings.keep_groups, &crease_points);
    }
    let area: f64 = (0..t.faces.len()).map(|f| f64::from(t.area(f))).sum();
    let step = (area / settings.faces.max(1) as f64).sqrt() as f32;
    let mut locked: Vec<bool> = (t.origin.iter())
        .map(|o| crease_points.contains(o))
        .collect();
    if !t.group.is_empty() {
        for (v, b) in group_borders(&t).into_iter().enumerate() {
            locked[v] |= b;
        }
    }
    report.rim_points_removed = resample_rims(&mut t, step, &locked);
    let sharp = sharp_edges(&t, &creases);
    report.triangles = t.faces.len() as u64;
    report.sharp_edges = sharp.len() as u64;
    (t, sharp, report)
}

/// An edge's two points, lower first.
fn pair(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// `mesh` as triangles, each quad split as it is drawn (its shorter diagonal, decision 0040
/// section 2), welded at [`WELD`] of the median edge, without faces left with a repeated point;
/// and each input point's stand-in.
fn welded(mesh: &PlainMesh, keep_groups: bool, report: &mut Preparation) -> (PlainMesh, Vec<u32>) {
    let p = &mesh.positions;
    let d2 = |a: u32, b: u32| dist2(p[a as usize], p[b as usize]);
    let mut tris: Vec<([u32; 3], u32)> = Vec::with_capacity(2 * mesh.faces.len());
    for (f, &[a, b, c, d]) in mesh.faces.iter().enumerate() {
        let f = f as u32;
        if d == NO_VERTEX {
            tris.push(([a, b, c], f));
        } else if quad_diagonal([a, b, c, d].map(|v| p[v as usize])) == 0 {
            tris.extend([([a, b, c], f), ([a, c, d], f)]);
        } else {
            tris.extend([([a, b, d], f), ([b, c, d], f)]);
        }
    }
    let mut lengths: Vec<f32> = (tris.iter())
        .flat_map(|([a, b, c], _)| [d2(*a, *b), d2(*b, *c), d2(*c, *a)])
        .collect();
    let mid = lengths.len() / 2;
    let median = if lengths.is_empty() {
        0.0
    } else {
        lengths.select_nth_unstable_by(mid, f32::total_cmp).1.sqrt()
    };
    let stand_in = weld::near(p, WELD * median);
    let mut used = vec![false; p.len()];
    for (t, _) in &mut tris {
        *t = t.map(|v| stand_in[v as usize]);
    }
    tris.retain(|([a, b, c], _)| a != b && b != c && c != a);
    for &v in tris.iter().flat_map(|(t, _)| t) {
        used[v as usize] = true;
    }
    report.merged = (0..p.len())
        .filter(|&v| stand_in[v] as usize != v && used[stand_in[v] as usize])
        .count() as u64;
    let mut attributes = std::collections::BTreeMap::new();
    let origin = (0..p.len() as u32).collect();
    attributes.insert(ORIGIN.to_string(), column(AttrDomain::Vertex, origin));
    let groups = (mesh.attributes.get(GROUP)).filter(|a| a.domain == AttrDomain::Face);
    if let (true, Some(AttrValues::U32(g))) = (keep_groups, groups.map(|a| &a.values)) {
        let groups = tris.iter().map(|&(_, f)| g[f as usize]).collect();
        attributes.insert(GROUP.to_string(), column(AttrDomain::Face, groups));
    }
    let faces = tris
        .iter()
        .map(|&([a, b, c], _)| [a, b, c, NO_VERTEX])
        .collect();
    let out = PlainMesh {
        positions: p.clone(),
        faces,
        attributes,
    };
    (out, stand_in)
}

/// A `u32` column.
fn column(domain: AttrDomain, values: Vec<u32>) -> Attribute {
    Attribute {
        domain,
        values: AttrValues::U32(values),
    }
}

impl Tris {
    /// The triangles of `mesh` (all its faces are triangles here) and its two columns.
    fn from_mesh(mesh: &PlainMesh) -> Self {
        let u32s = |name: &str| match mesh.attributes.get(name).map(|a| &a.values) {
            Some(AttrValues::U32(v)) => v.clone(),
            _ => Vec::new(),
        };
        Self {
            positions: mesh.positions.clone(),
            faces: mesh.faces.iter().map(|f| [f[0], f[1], f[2]]).collect(),
            group: u32s(GROUP),
            origin: u32s(ORIGIN),
        }
    }

    /// As a `PlainMesh` with its columns, and a point column `mask` of 1 at `masked` points.
    fn to_mesh(&self, masked: &[bool]) -> PlainMesh {
        let mut attributes = std::collections::BTreeMap::new();
        attributes.insert(
            ORIGIN.to_string(),
            column(AttrDomain::Vertex, self.origin.clone()),
        );
        if !self.group.is_empty() {
            let groups = column(AttrDomain::Face, self.group.clone());
            attributes.insert(GROUP.to_string(), groups);
        }
        let mask = masked.iter().map(|&m| if m { 1.0 } else { 0.0 }).collect();
        let mask = Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(mask),
        };
        attributes.insert(MASK.to_string(), mask);
        PlainMesh {
            positions: self.positions.clone(),
            faces: self
                .faces
                .iter()
                .map(|&[a, b, c]| [a, b, c, NO_VERTEX])
                .collect(),
            attributes,
        }
    }

    /// The corners of triangle `f`.
    fn corners(&self, f: usize) -> [[f32; 3]; 3] {
        self.faces[f].map(|v| self.positions[v as usize])
    }

    /// Twice the area vector of triangle `f`.
    fn normal(&self, f: usize) -> [f32; 3] {
        let [a, b, c] = self.corners(f);
        cross(sub(b, a), sub(c, a))
    }

    /// The area of triangle `f`.
    fn area(&self, f: usize) -> f32 {
        let n = self.normal(f);
        dot(n, n).sqrt() / 2.0
    }

    /// The triangles where `keep` says, and the points they use, in order.
    fn keep(&self, keep: &[bool]) -> Self {
        let mut renumber = vec![NO_VERTEX; self.positions.len()];
        let mut out = Self::default();
        for f in (0..self.faces.len()).filter(|&f| keep[f]) {
            let t = self.faces[f].map(|v| {
                let r = &mut renumber[v as usize];
                if *r == NO_VERTEX {
                    *r = out.positions.len() as u32;
                    out.positions.push(self.positions[v as usize]);
                    out.origin.push(self.origin[v as usize]);
                }
                *r
            });
            out.faces.push(t);
            if !self.group.is_empty() {
                out.group.push(self.group[f]);
            }
        }
        out
    }
}

/// The root of `v` in a union-find forest, halving the path on the way.
pub(crate) fn root(up: &mut [u32], mut v: u32) -> u32 {
    while up[v as usize] != v {
        up[v as usize] = up[up[v as usize] as usize];
        v = up[v as usize];
    }
    v
}

/// Each point's shell: points joined by faces share a root.
pub(crate) fn shells<'a>(points: usize, faces: impl Iterator<Item = &'a [u32]>) -> Vec<u32> {
    let mut up: Vec<u32> = (0..points as u32).collect();
    for face in faces {
        for w in face.windows(2) {
            let (a, b) = (root(&mut up, w[0]), root(&mut up, w[1]));
            up[a.max(b) as usize] = a.min(b);
        }
    }
    (0..points as u32).map(|v| root(&mut up, v)).collect()
}

/// `t` without shells of fewer than [`TINY_SHELL`] triangles.
fn without_tiny_shells(t: &Tris, report: &mut Preparation) -> Tris {
    let shell = shells(t.positions.len(), t.faces.iter().map(<[u32; 3]>::as_slice));
    let mut size = vec![0_usize; t.positions.len()];
    for tri in &t.faces {
        size[shell[tri[0] as usize] as usize] += 1;
    }
    let keep: Vec<bool> = (t.faces.iter())
        .map(|tri| size[shell[tri[0] as usize] as usize] >= TINY_SHELL)
        .collect();
    report.shells_dropped = keep.iter().filter(|&&k| !k).count() as u64;
    t.keep(&keep)
}

/// `t` decimated to [`MOST_TRIANGLES`] (decision 0034): open rims, group borders when groups
/// are kept and the crease points locked. Paint is not weighed: the quads are new faces, and
/// the source's columns reach them later through decision 0037's carry.
fn lightened(t: &Tris, keep_groups: bool, crease_points: &BTreeSet<u32>) -> Tris {
    let masked: Vec<bool> = t.origin.iter().map(|o| crease_points.contains(o)).collect();
    let settings = Lighten {
        faces: MOST_TRIANGLES,
        keep_groups,
        keep_borders: true,
        uniform: true,
        corners: Vec::new(),
        paint: Vec::new(),
        paint_weight: 0.0,
    };
    Tris::from_mesh(&decimate::decimate(&t.to_mesh(&masked), &settings).mesh)
}

/// The points where triangles of two groups meet.
fn group_borders(t: &Tris) -> Vec<bool> {
    let mut first: Vec<Option<u32>> = vec![None; t.positions.len()];
    let mut border = vec![false; t.positions.len()];
    for (tri, &g) in t.faces.iter().zip(&t.group) {
        for &v in tri {
            border[v as usize] |= *first[v as usize].get_or_insert(g) != g;
        }
    }
    border
}

/// Every edge as (its points, lower first; triangle; slot), sorted.
fn edges(t: &Tris) -> Vec<((u32, u32), u32, u8)> {
    let mut all: Vec<((u32, u32), u32, u8)> = (t.faces.iter().enumerate())
        .flat_map(|(f, tri)| {
            (0..3).map(move |j| (pair(tri[j], tri[(j + 1) % 3]), f as u32, j as u8))
        })
        .collect();
    gs_par::sort(&mut all);
    all
}

/// The edges for the sharp-edge file: creases, and group borders when groups are kept, each
/// once, from the first of its two triangles. Rims and crowded edges are left out.
fn sharp_edges(t: &Tris, creases: &BTreeSet<(u32, u32)>) -> Vec<Sharp> {
    let mut out = Vec::new();
    for run in edges(t).chunk_by(|a, b| a.0 == b.0) {
        let [(e, f, slot), (_, g, _)] = run else {
            continue;
        };
        let crease = creases.contains(&pair(t.origin[e.0 as usize], t.origin[e.1 as usize]));
        let border = !t.group.is_empty() && t.group[*f as usize] != t.group[*g as usize];
        if crease || border {
            let other = (t.faces[*g as usize].iter()).find(|&&v| v != e.0 && v != e.1);
            let far = other.map_or([0.0; 3], |&v| t.positions[v as usize]);
            let [a, _, _] = t.corners(*f as usize);
            let convex = dot(t.normal(*f as usize), sub(far, a)) <= 0.0;
            out.push(Sharp {
                face: *f,
                slot: *slot,
                convex,
            });
        }
    }
    out
}

/// Open rims resampled to about `step` (decision 0038, section 3, step 5): walking each rim
/// loop, a point closer than `step` to the last one kept collapses into it, if it is not
/// `locked`, the mesh stays manifold (the link condition) and no triangle flips. Loops through
/// a point with two rim edges out, and loops of three, are left alone. Returns how many points
/// went.
pub(crate) fn resample_rims(t: &mut Tris, step: f32, locked: &[bool]) -> u64 {
    let n = t.positions.len();
    let mut next = vec![NO_VERTEX; n];
    let mut out_edges = vec![0_u8; n];
    for run in edges(t)
        .chunk_by(|a, b| a.0 == b.0)
        .filter(|r| r.len() == 1)
    {
        let (_, f, j) = run[0];
        let tri = t.faces[f as usize];
        let (a, b) = (tri[j as usize], tri[(j as usize + 1) % 3]);
        next[a as usize] = b;
        out_edges[a as usize] = out_edges[a as usize].saturating_add(1);
    }
    let mut mesh = Collapse::new(t);
    let mut seen = vec![false; n];
    let mut removed = 0;
    for start in 0..n as u32 {
        let Some(len) = rim_loop(start, &next, &out_edges, &mut seen) else {
            continue;
        };
        let (mut kept, mut left, mut p) = (start, len, next[start as usize]);
        while p != start && left > 3 {
            let q = next[p as usize];
            let near = dist2(t.positions[kept as usize], t.positions[p as usize]) < step * step;
            if near && !locked[p as usize] && mesh.collapse(t, kept, p) {
                next[kept as usize] = q;
                left -= 1;
                removed += 1;
            } else {
                kept = p;
            }
            p = q;
        }
    }
    if removed > 0 {
        *t = t.keep(&mesh.alive);
    }
    removed
}

/// The length of the rim loop from `start`, marking its points seen, if it is a simple loop
/// not seen before.
fn rim_loop(start: u32, next: &[u32], out_edges: &[u8], seen: &mut [bool]) -> Option<usize> {
    if next[start as usize] == NO_VERTEX || seen[start as usize] {
        return None;
    }
    let (mut p, mut len, mut simple) = (start, 0, true);
    loop {
        seen[p as usize] = true;
        simple &= out_edges[p as usize] == 1;
        len += 1;
        p = next[p as usize];
        if p == start || p == NO_VERTEX || seen[p as usize] {
            simple &= p == start;
            break;
        }
    }
    simple.then_some(len)
}

/// The triangles around each point, for edge collapses.
struct Collapse {
    around: Vec<Vec<u32>>,
    alive: Vec<bool>,
}

impl Collapse {
    fn new(t: &Tris) -> Self {
        let mut around = vec![Vec::new(); t.positions.len()];
        for (f, tri) in t.faces.iter().enumerate() {
            for &v in tri {
                around[v as usize].push(f as u32);
            }
        }
        Self {
            around,
            alive: vec![true; t.faces.len()],
        }
    }

    /// The points joined to `v` by a living triangle.
    fn ring(&self, t: &Tris, v: u32) -> BTreeSet<u32> {
        (self.around[v as usize].iter())
            .filter(|&&f| self.alive[f as usize])
            .flat_map(|&f| t.faces[f as usize])
            .filter(|&w| w != v)
            .collect()
    }

    /// Collapses rim point `p` into its rim neighbour `k`, if the link condition holds and no
    /// triangle flips or flattens.
    fn collapse(&mut self, t: &mut Tris, k: u32, p: u32) -> bool {
        let live = |f: &&u32| self.alive[**f as usize];
        let both: Vec<u32> = (self.around[p as usize].iter().filter(live))
            .copied()
            .filter(|&f| t.faces[f as usize].contains(&k))
            .collect();
        let shared = self.ring(t, k).intersection(&self.ring(t, p)).count();
        if both.len() != 1 || shared != 1 {
            return false;
        }
        let moved: Vec<u32> = (self.around[p as usize].iter().filter(live))
            .copied()
            .filter(|&f| f != both[0])
            .collect();
        let target = t.positions[k as usize];
        for &f in &moved {
            let before = t.normal(f as usize);
            let mut c = t.corners(f as usize);
            let at = t.faces[f as usize]
                .iter()
                .position(|&v| v == p)
                .unwrap_or(0);
            c[at] = target;
            let after = cross(sub(c[1], c[0]), sub(c[2], c[0]));
            if dot(before, after) <= 0.0 || dot(after, after) <= 1e-6 * dot(before, before) {
                return false;
            }
        }
        self.alive[both[0] as usize] = false;
        for &f in &moved {
            for v in &mut t.faces[f as usize] {
                if *v == p {
                    *v = k;
                }
            }
            self.around[k as usize].push(f);
        }
        true
    }
}
