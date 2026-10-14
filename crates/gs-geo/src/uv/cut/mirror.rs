//! The mirror the cutter keeps (decision 0061 section 7, UV2.6): each point's and face's
//! partner, which half of the part is cut, and the other half's charts and cuts as mirror images.

use std::collections::BTreeMap;

use gs_protocol::{NO_VERTEX, arity, corners};

use super::context::NONE;
use super::grow::State;
use crate::uv::{Refusal, Surface, Symmetry};
use crate::weld;

/// Which half of the part a face is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    /// Further along the axis than its mirror face (ties to the lower face): cut.
    Positive,
    /// Its mirror face's mirror image: given its charts and cuts.
    Negative,
    /// Its own mirror image, across the plane: cut, among the others like it.
    Plane,
}

/// Each point's and face's mirror partner.
pub(crate) struct Mirror {
    /// Each point's partner.
    pub partner: Vec<u32>,
    /// Each face's mirror face.
    face: Vec<u32>,
    /// The corner of its mirror face that face corner 0 maps to; corner k maps to `shift - k`.
    shift: Vec<u8>,
    side: Vec<Side>,
}

impl Mirror {
    pub(crate) fn new(surface: Surface, symmetry: Symmetry) -> Result<Mirror, Refusal> {
        let (points, faces) = (surface.points, surface.faces);
        let n = points.len();
        let axis = symmetry.axis;
        let mirrored = points.iter().map(|&p| {
            let mut q = p;
            q[axis] = -q[axis];
            q
        });
        let both: Vec<[f32; 3]> = points.iter().copied().chain(mirrored).collect();
        let near = weld::near(&both, symmetry.tolerance);
        let partner: Vec<u32> = (0..n).map(|v| near[n + v]).collect();
        let lone: Vec<usize> = (0..n)
            .filter(|&v| partner[v] as usize >= n || near[partner[v] as usize + n] as usize != v)
            .collect();
        if let Some(&v) = lone.first() {
            return Err(Refusal::Unpaired {
                count: lone.len(),
                point: points[v],
            });
        }
        let key = |c: &[u32]| {
            let mut k = [NO_VERTEX; 4];
            k[..c.len()].copy_from_slice(c);
            k[..c.len()].sort_unstable();
            k
        };
        let mut sorted = BTreeMap::new();
        for (f, face) in faces.iter().enumerate() {
            sorted.entry(key(corners(face))).or_insert(f as u32);
        }
        let paired = gs_par::map(faces.len(), |f| {
            let c = corners(&faces[f]);
            let image: Vec<u32> = c.iter().map(|&v| partner[v as usize]).collect();
            let g = *sorted.get(&key(&image))?;
            let d = corners(&faces[g as usize]);
            let j0 = (0..d.len()).find(|&j| d[j] == image[0])?;
            let len = c.len();
            (0..len)
                .all(|k| d[(j0 + len - k) % len] == image[k])
                .then_some((g, j0 as u8))
        });
        if let Some(f) = paired.iter().position(Option::is_none) {
            let mut lone: Vec<u32> = (paired.iter().enumerate())
                .filter(|(_, p)| p.is_none())
                .flat_map(|(f, _)| corners(&faces[f]).to_vec())
                .collect();
            lone.sort_unstable();
            lone.dedup();
            return Err(Refusal::Unpaired {
                count: lone.len(),
                point: points[faces[f][0] as usize],
            });
        }
        let (face, shift): (Vec<u32>, Vec<u8>) = paired.into_iter().flatten().unzip();
        let centre = |f: usize| {
            let c = corners(&faces[f]);
            c.iter()
                .map(|&v| f64::from(points[v as usize][axis]))
                .sum::<f64>()
                / c.len() as f64
        };
        let side = gs_par::map(faces.len(), |f| {
            let g = face[f] as usize;
            let (here, there) = (centre(f), centre(g));
            if g == f {
                Side::Plane
            } else if here > there || (here >= there && f < g) {
                Side::Positive
            } else {
                Side::Negative
            }
        });
        Ok(Mirror {
            partner,
            face,
            shift,
            side,
        })
    }

    /// Face `f`'s side.
    pub(super) fn side(&self, f: usize) -> Side {
        self.side[f]
    }

    /// Where corner `k` of face `f` lands in the mirror: the mirror face and its corner.
    pub(crate) fn corner(&self, faces: &[[u32; 4]], f: usize, k: usize) -> (usize, usize) {
        let n = arity(faces[f]);
        (
            self.face[f] as usize,
            (usize::from(self.shift[f]) + n - k) % n,
        )
    }

    /// Whether face `f` is cut rather than mirrored.
    pub(super) fn active(&self, f: usize) -> bool {
        self.side[f] != Side::Negative
    }

    /// The mirror image of half-edge `h`: the half-edge of the mirror face that runs along the
    /// mirrored edge, the other way.
    pub(super) fn half(&self, faces: &[[u32; 4]], h: usize) -> usize {
        let (f, k) = (h / 4, h % 4);
        let n = arity(faces[f]);
        let g = self.face[f] as usize;
        4 * g + (usize::from(self.shift[f]) + n - (k + 1) % n) % n
    }

    /// Gives each face on the negative side its mirror face's chart, numbered past the cut
    /// charts, and its mirror face's cuts; a face already in a chart (a protected region across
    /// the plane) keeps it.
    pub(super) fn apply(&self, state: &mut State, faces: &[[u32; 4]]) {
        let offset = state.count;
        let negative: Vec<usize> = (0..faces.len())
            .filter(|&f| !self.active(f) && state.chart[f] == NONE)
            .collect();
        for &f in &negative {
            let g = self.face[f] as usize;
            debug_assert!(state.chart[g] != NONE, "a mirror face is cut");
            state.chart[f] = state.chart[g] + offset;
            let mut mask = 0;
            for k in 0..arity(faces[f]) {
                let m = self.half(faces, 4 * f + k);
                mask |= (state.cut[m / 4] >> (m % 4) & 1) << k;
            }
            state.cut[f] = mask;
        }
        state.count += offset;
    }
}
