//! Face edges and the one seam rule (decision 0061 section 8).
//!
//! A half-edge is `4 f + k`, face f's edge from corner k to corner k + 1; a corner is numbered
//! the same way, so half-edge h starts at corner h.

use gs_protocol::{arity, corners};

use crate::uv::Corners;

/// The twin of a half-edge on an open edge.
pub(super) const OPEN: u32 = u32::MAX;
/// The twin of a half-edge on an edge with three faces or more, or whose two faces run along it
/// the same way.
pub(super) const CROWDED: u32 = u32::MAX - 1;

/// Every face edge, found by sorting each one's two points, lower first.
pub(super) struct Edges {
    /// Every half-edge by its edge's key (its two points), in order.
    keyed: Vec<(u64, u32)>,
    /// Each half-edge's twin, [`OPEN`] or [`CROWDED`]; past a face's corners, [`OPEN`].
    pub twin: Vec<u32>,
}

/// The corner after corner `c` around its face.
pub(super) fn next(faces: &[[u32; 4]], c: usize) -> usize {
    let (f, k) = (c / 4, c % 4);
    4 * f + (k + 1) % arity(faces[f])
}

impl Edges {
    pub(super) fn new(faces: &[[u32; 4]]) -> Edges {
        let mut keyed = Vec::with_capacity(4 * faces.len());
        for (f, face) in faces.iter().enumerate() {
            let c = corners(face);
            for k in 0..c.len() {
                let (a, b) = (c[k], c[(k + 1) % c.len()]);
                let key = u64::from(a.min(b)) << 32 | u64::from(a.max(b));
                keyed.push((key, (4 * f + k) as u32));
            }
        }
        gs_par::sort(&mut keyed);
        let mut twin = vec![OPEN; 4 * faces.len()];
        let start = |h: u32| faces[h as usize / 4][h as usize % 4];
        for run in keyed.chunk_by(|x, y| x.0 == y.0) {
            match *run {
                [_] => {}
                [(_, h), (_, g)] if start(h) != start(g) => {
                    twin[h as usize] = g;
                    twin[g as usize] = h;
                }
                _ => run.iter().for_each(|&(_, h)| twin[h as usize] = CROWDED),
            }
        }
        Edges { keyed, twin }
    }

    /// The other half-edges on half-edge `h`'s edge.
    pub(super) fn others(&self, h: usize, faces: &[[u32; 4]]) -> impl Iterator<Item = usize> + '_ {
        let c = &faces[h / 4];
        let (a, b) = (c[h % 4], c[next(faces, h) % 4]);
        let key = u64::from(a.min(b)) << 32 | u64::from(a.max(b));
        let from = self.keyed.partition_point(|x| x.0 < key);
        (self.keyed[from..].iter())
            .take_while(move |x| x.0 == key)
            .map(|x| x.1 as usize)
            .filter(move |&g| g != h)
    }

    /// The edges, each as the half-edges along it.
    pub(super) fn runs(&self) -> impl Iterator<Item = &[(u64, u32)]> {
        self.keyed.chunk_by(|x, y| x.0 == y.0)
    }

    /// Each face's seams by `same(a, b)`, whether corners a and b hold the same thing: bit k of
    /// face f is set when another face on its edge from corner k differs from it at either end.
    /// Open edges are never seams.
    pub(super) fn seams_by(
        &self,
        faces: &[[u32; 4]],
        same: impl Fn(usize, usize) -> bool + Sync,
    ) -> Vec<u8> {
        let point = |c: usize| faces[c / 4][c % 4];
        let differs = |h: usize, g: usize| {
            let (h1, g1) = (next(faces, h), next(faces, g));
            if point(h) == point(g) {
                !same(h, g) || !same(h1, g1)
            } else {
                !same(h, g1) || !same(h1, g)
            }
        };
        gs_par::map(faces.len(), |f| {
            let mut mask = 0;
            for k in 0..arity(faces[f]) {
                let h = 4 * f + k;
                let seam = match self.twin[h] {
                    OPEN => false,
                    CROWDED => self.others(h, faces).any(|g| differs(h, g)),
                    g => differs(h, g as usize),
                };
                mask |= u8::from(seam) << k;
            }
            mask
        })
    }
}

/// Each face's seams in `uvs`, a UV per face corner: bit k of face f is set when the edge from
/// its corner k to corner k + 1 is a seam, an edge another face holds with a different UV,
/// compared by bits, at either end (decision 0061 section 8). Open edges are never seams, and a
/// non-manifold edge is compared against every other face on it.
pub fn seams(faces: &[[u32; 4]], uvs: &[Corners]) -> Vec<u8> {
    let uv = |c: usize| uvs[c / 4][c % 4].map(f32::to_bits);
    Edges::new(faces).seams_by(faces, |a, b| uv(a) == uv(b))
}
