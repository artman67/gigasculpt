//! A chart as a mesh of its own: its wedges (a point's corners joined through edges inside the
//! chart that are not cut), which of its half-edges join two of its faces, and its boundary
//! loops. The disc cut, the check and [`Charts::meshes`] all read a chart this way.

use gs_protocol::{NO_VERTEX, arity};

use super::context::NONE;
use super::edges::{CROWDED, Edges, OPEN};
use crate::uv::{Chart, Charts, Corners, Surface};
use crate::weld::{find, join};

/// A chart's mesh, and which of its half-edges lie inside it.
pub(super) struct Local {
    pub chart: Chart,
    /// Each local half-edge's twin (`4 i + k`, `i` a face's place in the chart), or [`OPEN`]
    /// on the chart's boundary.
    pub twin: Vec<u32>,
}

/// The faces of each chart, lowest first, and each face's place among its chart's.
pub(super) struct Places {
    pub faces: Vec<Vec<u32>>,
    pub slot: Vec<u32>,
}

/// Where each face of `chart` ([`NONE`] for none) sits, among `count` charts.
pub(super) fn places(chart: &[u32], count: usize) -> Places {
    let mut faces = vec![Vec::new(); count];
    let mut slot = vec![NONE; chart.len()];
    for (f, &c) in chart.iter().enumerate().filter(|&(_, &c)| c != NONE) {
        slot[f] = faces[c as usize].len() as u32;
        faces[c as usize].push(f as u32);
    }
    Places { faces, slot }
}

impl Local {
    /// The mesh of the chart holding `members` (lowest first, `slot` their places), whose
    /// corners join across every half-edge between two of its faces that `cut` leaves (`cut`
    /// must say the same of a half-edge and its twin).
    pub(super) fn new(
        faces: &[[u32; 4]],
        edges: &Edges,
        (chart, slot): (&[u32], &[u32]),
        members: Vec<u32>,
        cut: impl Fn(usize) -> bool,
    ) -> Local {
        let m = members.len();
        let mut root: Vec<usize> = (0..4 * m).collect();
        let mut twin = vec![OPEN; 4 * m];
        for (i, &f) in members.iter().enumerate() {
            let (f, n) = (f as usize, arity(faces[f as usize]));
            for k in 0..n {
                let (h, t) = (4 * f + k, edges.twin[4 * f + k]);
                if t >= CROWDED || chart[t as usize / 4] != chart[f] || cut(h) {
                    continue;
                }
                let (g, j) = (t as usize / 4, t as usize % 4);
                let s = 4 * slot[g] as usize;
                twin[4 * i + k] = (s + j) as u32;
                join(&mut root, 4 * i + k, s + (j + 1) % arity(faces[g]));
                join(&mut root, 4 * i + (k + 1) % n, s + j);
            }
        }
        let mut wedge = vec![NONE; 4 * m];
        let mut points = Vec::new();
        let mut corners = vec![[NO_VERTEX; 4]; m];
        for (i, &f) in members.iter().enumerate() {
            for k in 0..arity(faces[f as usize]) {
                let r = find(&mut root, 4 * i + k);
                if wedge[r] == NONE {
                    wedge[r] = points.len() as u32;
                    points.push(faces[f as usize][k]);
                }
                corners[i][k] = wedge[r];
            }
        }
        let chart = Chart {
            faces: members,
            points,
            corners,
        };
        Local { chart, twin }
    }

    /// The wedge local half-edge `h` starts at, and the one it ends at.
    pub(super) fn ends(&self, h: usize) -> (usize, usize) {
        let c = self.chart.corners[h / 4];
        (c[h % 4] as usize, c[(h % 4 + 1) % arity(c)] as usize)
    }

    /// Every local half-edge.
    pub(super) fn halves(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.chart.corners.len())
            .flat_map(|i| (0..arity(self.chart.corners[i])).map(move |k| 4 * i + k))
    }

    /// The part's half-edge local half-edge `h` is.
    pub(super) fn global(&self, h: usize) -> usize {
        4 * self.chart.faces[h / 4] as usize + h % 4
    }

    /// The boundary loops, each its half-edges in order, or `None` when a wedge has two
    /// boundary half-edges leaving it (the chart pinches there).
    pub(super) fn loops(&self) -> Option<Vec<Vec<usize>>> {
        let mut out = vec![NONE; self.chart.points.len()];
        for h in self.halves().filter(|&h| self.twin[h] == OPEN) {
            let (a, _) = self.ends(h);
            if out[a] != NONE {
                return None;
            }
            out[a] = h as u32;
        }
        let mut seen = vec![false; self.twin.len()];
        let mut loops = Vec::new();
        for h in self.halves().filter(|&h| self.twin[h] == OPEN) {
            if seen[h] {
                continue;
            }
            let mut ring = Vec::new();
            let mut e = h;
            loop {
                seen[e] = true;
                ring.push(e);
                let after = out[self.ends(e).1];
                if after == NONE || seen[after as usize] && after as usize != h {
                    return None;
                }
                e = after as usize;
                if e == h {
                    break;
                }
            }
            loops.push(ring);
        }
        Some(loops)
    }

    /// Whether it is a disc: Euler characteristic 1 (wedges less edges plus faces) and one
    /// boundary loop.
    pub(super) fn is_disc(&self) -> bool {
        let (mut inner, mut outer) = (0, 0);
        for h in self.halves() {
            if self.twin[h] == OPEN {
                outer += 1;
            } else {
                inner += 1;
            }
        }
        let (points, faces) = (self.chart.points.len(), self.chart.faces.len());
        points + faces == inner / 2 + outer + 1 && self.loops().is_some_and(|l| l.len() == 1)
    }
}

impl Charts {
    /// Each chart as a mesh of its own, the input a flattener takes: its wedges are joined
    /// through every edge inside it that is not a seam.
    pub fn meshes(&self, surface: Surface) -> Vec<Chart> {
        let edges = Edges::new(surface.faces);
        let places = places(&self.chart, self.count as usize);
        let seam = |h: usize| self.seams[h / 4] >> (h % 4) & 1 == 1;
        gs_par::map(self.count as usize, |c| {
            let at = (self.chart.as_slice(), places.slot.as_slice());
            Local::new(surface.faces, &edges, at, places.faces[c].clone(), seam).chart
        })
    }
}

impl Chart {
    /// Writes `uvs`, a UV per point of this chart, into its faces' corners in `out`.
    pub fn write(&self, uvs: &[[f32; 2]], out: &mut [Corners]) {
        for (i, &f) in self.faces.iter().enumerate() {
            for k in 0..arity(self.corners[i]) {
                out[f as usize][k] = uvs[self.corners[i][k] as usize];
            }
        }
    }
}
