//! The Smooth modes and settings decision 0049 added: along and across the stroke (section 2),
//! group borders with Weight Strength (section 3), Projection (section 4) and Min Connected
//! Points (section 5). A spec that uses none of them never runs this code (section 6), so the
//! first four modes run as they did, rounds included.

use super::{Scratch, chosen, mean};
use crate::brush::{SmoothMode, SmoothSpec};
use crate::columns::AttrId;
use crate::config::VertId;
use crate::kernel::SculptSurface;
use crate::kernels::Frame;
use crate::math::{Vec3, arity};
use crate::view::{PosRead, TopoRead, edge_neighbours};

/// The translation of vertex `v` (at `p`, normal `n`) with influence `w` under a spec that is
/// not plain. `group` is the face group column, if the mesh has one; `start` the point's normal
/// at the stroke's start, where its node's ids still name the start's points (else Projection
/// reads `n`). `nb` is the node's scratch.
#[expect(
    clippy::too_many_arguments,
    reason = "one vertex's whole context, read once"
)]
pub(in crate::kernels) fn smooth(
    k: SmoothSpec,
    s: SculptSurface<'_>,
    f: &Frame,
    v: VertId,
    p: Vec3,
    n: Vec3,
    w: f32,
    nb: &mut Scratch,
    group: Option<AttrId>,
    start: Option<Vec3>,
) -> Vec3 {
    let (topo, visible) = (s.topo(), s.visible());
    edge_neighbours(&topo, v, |f| visible.face(f), &mut nb.mine);
    if (chosen(&nb.mine).count() as u32) < k.min_connected {
        return Vec3::ZERO;
    }
    let rate =
        |raw: Option<Vec3>| raw.unwrap_or(Vec3::ZERO) * (1.0 - libm::expf(-k.gain * w * f.amount));
    let d = match k.mode {
        SmoothMode::AlongStroke => rate(on_line(s, v, p, n, f.delta, false)),
        SmoothMode::AcrossStroke => rate(on_line(s, v, p, n, f.delta, true)),
        SmoothMode::GroupBorders => rate(borders(s, v, p, &nb.mine, group, k.weight)),
        // The old path's own function, which gathers the neighbours again: sharing its inner
        // code instead left that code out of line, a call per point on the old path (decision
        // 0049, "Measured").
        old => super::smooth((old, k.gain), s, f, v, p, n, w, nb),
    };
    if k.projection {
        let n0 = start.unwrap_or(n);
        d - n0 * n0.dot(d)
    } else {
        d
    }
}

/// The raw translation toward the curve of the surface along the stroke step `d` laid into the
/// tangent plane (`across`: along the normal crossed with it): the plane through `p` spanned by
/// that direction and `n` cuts the edges of the ring that do not touch `v`, and the target is
/// where the line joining the nearest cut ahead and the nearest behind passes the point. `None`
/// without a step, a step along the normal, or a side with no cut.
fn on_line(
    s: SculptSurface<'_>,
    v: VertId,
    p: Vec3,
    n: Vec3,
    d: Vec3,
    across: bool,
) -> Option<Vec3> {
    let t = d - n * n.dot(d);
    if d.len2() == 0.0 || t.len2() <= 1e-10 * d.len2() {
        return None;
    }
    let t = t.normalized();
    let a = if across { n.cross(t) } else { t };
    // The cutting plane's normal: `a` lies across `n`, so it is a unit vector.
    let c = n.cross(a);
    let (pos, topo, visible) = (s.pos(), s.topo(), s.visible());
    let mut ahead: Option<(f32, Vec3)> = None;
    let mut behind = ahead;
    for &face in topo.ring(v) {
        if !visible.face(face) {
            continue;
        }
        let fc = topo.face(face);
        let k = arity(fc);
        let Some(i) = fc[..k].iter().position(|&u| u == v) else {
            continue;
        };
        for e in 1..k - 1 {
            let (x, y) = (pos.get(fc[(i + e) % k]), pos.get(fc[(i + e + 1) % k]));
            let (sx, sy) = ((x - p).dot(c), (y - p).dot(c));
            if (sx > 0.0 && sy > 0.0) || (sx < 0.0 && sy < 0.0) || (sx == 0.0 && sy == 0.0) {
                continue;
            }
            let q = x + (y - x) * (sx / (sx - sy));
            let at = (q - p).dot(a);
            let side = if at > 0.0 {
                &mut ahead
            } else if at < 0.0 {
                &mut behind
            } else {
                continue;
            };
            if side.is_none_or(|(h, _)| at.abs() < h) {
                *side = Some((at.abs(), q));
            }
        }
    }
    let ((ha, qa), (hb, qb)) = (ahead?, behind?);
    Some(qb + (qa - qb) * (hb / (ha + hb)) - p)
}

/// The raw translation under `group_borders`: toward `m + weight * (m_b - m)`, with `m` the mean
/// of the chosen neighbours `nb` and `m_b` that of the border neighbours, those whose shared
/// visible faces carry more than one face group. As `uniform` with no border neighbour, no
/// group column, or on an open border (whose rule it keeps).
fn borders(
    s: SculptSurface<'_>,
    v: VertId,
    p: Vec3,
    nb: &[(VertId, u8)],
    group: Option<AttrId>,
    weight: f32,
) -> Option<Vec3> {
    let pos = s.pos();
    let m = mean(chosen(nb), |u| pos.get(u))?;
    let Some(col) = group else {
        return Some(m - p);
    };
    if nb.iter().any(|e| e.1 == 1) {
        return Some(m - p);
    }
    let (topo, visible, groups) = (s.topo(), s.visible(), s.attr_reader(col));
    let border = |u: VertId| {
        let mut first = None;
        for &face in topo.ring(v) {
            let fc = topo.face(face);
            let k = arity(fc);
            let Some(i) = fc[..k].iter().position(|&x| x == v) else {
                continue;
            };
            if !visible.face(face) || (fc[(i + 1) % k] != u && fc[(i + k - 1) % k] != u) {
                continue;
            }
            let g = groups.label(face);
            match first {
                None => first = Some(g),
                Some(h) if h != g => return true,
                Some(_) => {}
            }
        }
        false
    };
    let ids = nb.iter().map(|e| e.0).filter(|&u| border(u));
    Some(match mean(ids, |u| pos.get(u)) {
        Some(mb) => m + (mb - m) * weight - p,
        None => m - p,
    })
}
