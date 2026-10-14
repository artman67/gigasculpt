//! The `closed` claim (decision 0057, section 7): every part a call adds is closed and manifold,
//! each directed edge used once and its reverse present, and no two of its faces that share no
//! point cross.
//!
//! The crossings are found as research 26's bench found them, with a grid of the harness's own
//! and nothing of the engine it checks: cells of about two mean edges; in each cell, every two
//! triangles that share no corner are tested both ways for an edge of one crossing the inside of
//! the other, strictly beyond 1e-10 times the mean edge cubed (so touching and lying flat on
//! each other do not count, nor do two triangles that meet only where edges cross edges, which
//! takes points placed just so). Positions are the mesh's own 32-bit values, measured in `f64`;
//! faces split into triangles as they are drawn (`PlainMesh::triangles`).

use gs_protocol::PlainMesh;

use crate::measure::{corners, edges};
use crate::session::Scene;

type P = [f64; 3];

/// Checks every part in `after` that `before` does not have; says whether there was one.
pub fn closed(before: &Scene, after: &Scene) -> Result<bool, String> {
    let mut added = false;
    for (name, mesh) in after.iter().filter(|(name, _)| !before.contains_key(*name)) {
        added = true;
        if let Some(why) = unclosed(mesh) {
            return Err(format!("the new part '{name}' {why}"));
        }
    }
    Ok(added)
}

/// Why `mesh` is not a closed, manifold surface that crosses nowhere, or `None` if it is.
pub fn unclosed(mesh: &PlainMesh) -> Option<String> {
    if mesh.faces.is_empty() {
        return Some("has no faces".into());
    }
    if let Some(why) = sided(mesh) {
        return Some(why);
    }
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    let pairs = crossings(&mesh.positions, &tris);
    let (s, t) = pairs.first()?;
    Some(format!(
        "crosses itself: triangles {s} and {t} cross ({} pairs in all)",
        pairs.len()
    ))
}

/// Why the faces' edges are not each used once in each direction, or `None`.
fn sided(mesh: &PlainMesh) -> Option<String> {
    let mut directed: Vec<(u32, u32)> = (mesh.faces.iter())
        .flat_map(|face| edges(corners(face)))
        .collect();
    directed.sort_unstable();
    if let Some(w) = directed.windows(2).find(|w| w[0] == w[1]) {
        let (a, b) = w[0];
        return Some(format!(
            "is not manifold: two faces run along edge {a}-{b} the same way, or three or more \
             faces share it"
        ));
    }
    let (a, b) = *(directed.iter()).find(|&&(a, b)| directed.binary_search(&(b, a)).is_err())?;
    Some(format!("is open: edge {a}-{b} has a face on one side only"))
}

/// The pairs of triangles (indices into `tris`, the lower first, sorted) that cross.
pub fn crossings(positions: &[[f32; 3]], tris: &[[u32; 3]]) -> Vec<(u32, u32)> {
    if tris.is_empty() {
        return Vec::new();
    }
    let at = |v: u32| positions[v as usize].map(f64::from);
    let corners = |t: usize| tris[t].map(at);
    let edges: f64 = (0..tris.len())
        .map(|t| {
            let c = corners(t);
            (0..3).map(|k| dist(c[k], c[(k + 1) % 3])).sum::<f64>()
        })
        .sum();
    let mean = edges / (3 * tris.len()) as f64;
    let eps = 1e-10 * mean * mean * mean;
    let lowest = |a: usize| (tris.iter().flatten()).fold(f64::INFINITY, |m, &v| m.min(at(v)[a]));
    let lo = [0, 1, 2].map(lowest);
    let cell = (2.0 * mean).max(f64::MIN_POSITIVE);
    let key = |p: P| -> [u64; 3] { [0, 1, 2].map(|a| ((p[a] - lo[a]) / cell) as u64) };
    // Every cell a triangle's box touches holds it.
    let mut entries: Vec<([u64; 3], u32)> = Vec::new();
    for t in 0..tris.len() {
        let (a, b) = bounds(&corners(t));
        let (ka, kb) = (key(a), key(b));
        for x in ka[0]..=kb[0] {
            for y in ka[1]..=kb[1] {
                for z in ka[2]..=kb[2] {
                    entries.push(([x, y, z], t as u32));
                }
            }
        }
    }
    entries.sort_unstable();
    let mut pairs = Vec::new();
    for group in entries.chunk_by(|a, b| a.0 == b.0) {
        for (i, &(_, s)) in group.iter().enumerate() {
            for &(_, t) in &group[i + 1..] {
                let (s, t) = (s.min(t), s.max(t));
                let shared = (tris[s as usize].iter()).any(|v| tris[t as usize].contains(v));
                if !shared && cross(&corners(s as usize), &corners(t as usize), eps) {
                    pairs.push((s, t));
                }
            }
        }
    }
    pairs.sort_unstable();
    pairs.dedup();
    pairs
}

/// True when an edge of `a` crosses the inside of `b` or an edge of `b` the inside of `a`.
fn cross(a: &[P; 3], b: &[P; 3], eps: f64) -> bool {
    let (amin, amax) = bounds(a);
    let (bmin, bmax) = bounds(b);
    if (0..3).any(|k| amax[k] < bmin[k] || bmax[k] < amin[k]) {
        return false;
    }
    let pierce =
        |s: &[P; 3], t: &[P; 3]| (0..3).any(|k| edge_through(s[k], s[(k + 1) % 3], t, eps));
    pierce(a, b) || pierce(b, a)
}

/// True when the segment from `p` to `q` passes through the inside of triangle `t`.
fn edge_through(p: P, q: P, t: &[P; 3], eps: f64) -> bool {
    let (dp, dq) = (orient(t[0], t[1], t[2], p), orient(t[0], t[1], t[2], q));
    if !(dp > eps && dq < -eps || dp < -eps && dq > eps) {
        return false;
    }
    let s = [0, 1, 2].map(|k| orient(p, q, t[k], t[(k + 1) % 3]));
    s.iter().all(|&x| x > eps) || s.iter().all(|&x| x < -eps)
}

/// Six times the signed volume of the tetrahedron `a b c d`.
fn orient(a: P, b: P, c: P, d: P) -> f64 {
    let (u, v, w) = (sub(b, a), sub(c, a), sub(d, a));
    u[0] * (v[1] * w[2] - v[2] * w[1]) - u[1] * (v[0] * w[2] - v[2] * w[0])
        + u[2] * (v[0] * w[1] - v[1] * w[0])
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dist(a: P, b: P) -> f64 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn bounds(c: &[P; 3]) -> (P, P) {
    let lo = [0, 1, 2].map(|a| c[0][a].min(c[1][a]).min(c[2][a]));
    let hi = [0, 1, 2].map(|a| c[0][a].max(c[1][a]).max(c[2][a]));
    (lo, hi)
}
