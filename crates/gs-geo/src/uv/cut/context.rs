//! What the cutter reads about each face and edge, worked out once: normals, areas, hints, the
//! hard borders and the protected regions (decision 0061 sections 5 and 6, UV2's amendment).

use gs_protocol::{arity, corners, face_triangles};

use super::edges::{CROWDED, Edges, OPEN, next};
use super::mirror::Mirror;
use crate::math::{cross, dot, sub};
use crate::uv::{Refusal, Steer, Surface};
use crate::weld::{find, join};

/// No chart, region or face.
pub(super) const NONE: u32 = u32::MAX;

/// A face as growth reads it.
#[derive(Clone, Copy, Debug)]
pub(super) struct Face {
    /// Its unit normal, the sum of its triangles' (zero on a face with no area).
    pub normal: [f64; 3],
    /// Its area, its triangles' together.
    pub area: f64,
    /// The mean of its corners.
    pub centre: [f64; 3],
    /// The mean of its corners' `uv_density`.
    pub density: f64,
}

/// The part as the cutter reads it.
pub(super) struct Ctx<'a> {
    pub surface: Surface<'a>,
    pub edges: Edges,
    pub face: Vec<Face>,
    /// `uv_hint` per point (averaged with its partner's under symmetry), empty when left out.
    hint: Vec<f32>,
    /// Bit k of face f: its edge from corner k is a border growth never crosses.
    pub hard: Vec<u8>,
    /// The kept layout's seams, when there is one.
    pub kept: Option<Vec<u8>>,
    /// Each face's protected region, named by its lowest face, or [`NONE`].
    pub region: Vec<u32>,
    /// Every protected face by (region, face).
    members: Vec<(u32, u32)>,
    pub mirror: Option<Mirror>,
}

impl<'a> Ctx<'a> {
    pub(super) fn new(surface: Surface<'a>, steer: &Steer) -> Result<Ctx<'a>, Refusal> {
        let faces = surface.faces;
        let edges = Edges::new(faces);
        let mirror = (steer.symmetry)
            .map(|s| Mirror::new(surface, s))
            .transpose()?;
        let paired = |values: Option<&[f32]>| -> Vec<f32> {
            let Some(values) = values else {
                return Vec::new();
            };
            match &mirror {
                Some(m) => (0..values.len())
                    .map(|v| f32::midpoint(values[v], values[m.partner[v] as usize]))
                    .collect(),
                None => values.to_vec(),
            }
        };
        let (hint, density) = (paired(steer.hint), paired(steer.density));
        let mean = |values: &[f32], f: usize| -> f32 {
            let c = corners(&faces[f]);
            let sum: f32 = c.iter().map(|&v| values[v as usize]).sum();
            sum / c.len() as f32
        };
        let face = gs_par::map(faces.len(), |f| {
            let mut face = shape(surface, f);
            if !density.is_empty() {
                face.density = f64::from(mean(&density, f));
            }
            face
        });
        let guarded: Vec<bool> =
            gs_par::map(faces.len(), |f| !hint.is_empty() && mean(&hint, f) <= -0.7);
        let kept = steer.keep.map(|uvs| super::seams(faces, uvs));
        let lines = Lines {
            groups: steer.groups,
            kept: kept.as_deref(),
            mirror: mirror.as_ref(),
            guarded: &guarded,
        };
        let hard = borders(faces, &edges, &lines);
        let mut ctx = Ctx {
            surface,
            edges,
            face,
            hint,
            hard,
            kept,
            region: vec![NONE; faces.len()],
            members: Vec::new(),
            mirror,
        };
        ctx.protect(&guarded)?;
        ctx.seal();
        Ok(ctx)
    }

    /// Under symmetry, closes off each protected region that reaches across the plane, so it is
    /// a chart by itself and its own mirror image (UV2.6).
    fn seal(&mut self) {
        let Some(m) = &self.mirror else {
            return;
        };
        let faces = self.surface.faces;
        let mut across = vec![false; faces.len()];
        for f in (0..faces.len()).filter(|&f| self.protected(f)) {
            let r = self.region[f] as usize;
            across[r] |= m.side(f) != m.side(r);
        }
        let out: Vec<(usize, usize)> = (0..faces.len())
            .filter(|&f| self.protected(f) && across[self.region[f] as usize])
            .flat_map(|f| (0..arity(faces[f])).map(move |k| 4 * f + k))
            .filter_map(|h| {
                let t = self.cross(h)?;
                (self.region[t / 4] != self.region[h / 4]).then_some((h, t))
            })
            .collect();
        for (h, t) in out {
            self.hard[h / 4] |= 1 << (h % 4);
            self.hard[t / 4] |= 1 << (t % 4);
        }
    }

    /// Joins the protected faces into regions through the edges growth may cross, and refuses
    /// when a border runs between two of them.
    fn protect(&mut self, guarded: &[bool]) -> Result<(), Refusal> {
        let faces = self.surface.faces;
        let mut root: Vec<usize> = (0..faces.len()).collect();
        for f in (0..faces.len()).filter(|&f| guarded[f]) {
            for k in 0..arity(faces[f]) {
                if let Some(t) = self.cross(4 * f + k)
                    && guarded[t / 4]
                {
                    join(&mut root, f, t / 4);
                }
            }
        }
        for f in (0..faces.len()).filter(|&f| guarded[f]) {
            self.region[f] = find(&mut root, f) as u32;
            self.members.push((self.region[f], f as u32));
        }
        self.members.sort_unstable();
        for f in (0..faces.len()).filter(|&f| guarded[f]) {
            for k in 0..arity(faces[f]) {
                let h = 4 * f + k;
                if self.cross(h).is_none() && self.guarded(h) {
                    return Err(self.refusal(h));
                }
            }
        }
        Ok(())
    }

    /// The twin of half-edge `h` when growth may cross it.
    pub(super) fn cross(&self, h: usize) -> Option<usize> {
        let t = self.edges.twin[h];
        (t < CROWDED && self.hard[h / 4] >> (h % 4) & 1 == 0).then_some(t as usize)
    }

    /// Whether face `f` is protected.
    pub(super) fn protected(&self, f: usize) -> bool {
        self.region[f] != NONE
    }

    /// Whether half-edge `h` runs between two protected faces.
    pub(super) fn guarded(&self, h: usize) -> bool {
        let faces = self.surface.faces;
        self.protected(h / 4)
            && match self.edges.twin[h] {
                OPEN => false,
                CROWDED => (self.edges.others(h, faces)).any(|g| self.protected(g / 4)),
                t => self.protected(t as usize / 4),
            }
    }

    /// The faces of the protected region named `root`.
    pub(super) fn members(&self, root: u32) -> impl Iterator<Item = u32> + '_ {
        let from = self.members.partition_point(|m| m.0 < root);
        (self.members[from..].iter())
            .take_while(move |m| m.0 == root)
            .map(|m| m.1)
    }

    /// The refusal for a cut through half-edge `h` of a protected face, naming the end of `h`
    /// painted more strongly (at least one end of an edge between protected faces is painted).
    pub(super) fn refusal(&self, h: usize) -> Refusal {
        let (a, b) = (self.point(h), self.point(next(self.surface.faces, h)));
        let paint = |v: u32| self.hint.get(v as usize).copied().unwrap_or(0.0);
        let end = if paint(b) < paint(a) { b } else { a };
        Refusal::Protected {
            faces: self.members(self.region[h / 4]).count(),
            point: self.surface.points[end as usize],
        }
    }

    /// The point at corner `c`.
    pub(super) fn point(&self, c: usize) -> u32 {
        self.surface.faces[c / 4][c % 4]
    }

    /// The position of point `v`.
    pub(super) fn at(&self, v: u32) -> [f64; 3] {
        self.surface.points[v as usize].map(f64::from)
    }

    /// Half-edge `h`'s length.
    pub(super) fn len(&self, h: usize) -> f64 {
        let d = sub(
            self.at(self.point(h)),
            self.at(self.point(next(self.surface.faces, h))),
        );
        dot(d, d).sqrt()
    }

    /// Half-edge `h`'s attract: the smaller of its two points' `uv_hint`, when above 0.
    pub(super) fn attract(&self, h: usize) -> f64 {
        if self.hint.is_empty() {
            return 0.0;
        }
        let (a, b) = (self.point(h), self.point(next(self.surface.faces, h)));
        f64::from(self.hint[a as usize].min(self.hint[b as usize])).max(0.0)
    }

    /// What a cut through half-edge `h` costs: its length, less where attract is painted, and a
    /// million times it inside a protected region.
    pub(super) fn weight(&self, h: usize) -> f64 {
        if self.guarded(h) {
            1e6 * self.len(h)
        } else {
            self.len(h) * (1.0 - 0.8 * self.attract(h))
        }
    }

    /// Whether face `f` is cut: every face, or under symmetry the positive half's and those that
    /// map to themselves.
    pub(super) fn active(&self, f: usize) -> bool {
        self.mirror.as_ref().is_none_or(|m| m.active(f))
    }
}

/// Face `f`'s normal, area and centre, its density 0.
fn shape(surface: Surface, f: usize) -> Face {
    let face = surface.faces[f];
    let p = |k: usize| surface.points[face[k] as usize].map(f64::from);
    let (mut normal, mut area) = ([0.0; 3], 0.0);
    for t in face_triangles(face, |k| surface.points[face[k] as usize]) {
        let c = cross(sub(p(t[1]), p(t[0])), sub(p(t[2]), p(t[0])));
        area += dot(c, c).sqrt() / 2.0;
        normal = std::array::from_fn(|i| normal[i] + c[i]);
    }
    let length = dot(normal, normal).sqrt();
    if length > 0.0 {
        normal = normal.map(|x| x / length);
    }
    let n = arity(face);
    let centre = std::array::from_fn(|i| (0..n).map(|k| p(k)[i]).sum::<f64>() / n as f64);
    Face {
        normal,
        area,
        centre,
        density: 0.0,
    }
}

/// What draws hard borders: the groups, the kept layout's seams, the mirror, and which faces
/// are protected.
struct Lines<'a> {
    groups: Option<&'a [u32]>,
    kept: Option<&'a [u8]>,
    mirror: Option<&'a Mirror>,
    guarded: &'a [bool],
}

/// Each face's hard borders: open and crowded edges, edges between two groups, the kept
/// layout's seams, and under symmetry the edges between two sides (but between two protected
/// faces) and the mirror image of each of these.
fn borders(faces: &[[u32; 4]], edges: &Edges, lines: &Lines) -> Vec<u8> {
    let hard = gs_par::map(faces.len(), |f| {
        let mut mask = 0;
        for k in 0..arity(faces[f]) {
            let t = edges.twin[4 * f + k];
            let g = t as usize / 4;
            let border = t >= CROWDED
                || lines.groups.is_some_and(|groups| groups[f] != groups[g])
                || lines.kept.is_some_and(|s| s[f] >> k & 1 == 1)
                || lines.mirror.is_some_and(|m| {
                    m.side(f) != m.side(g) && !(lines.guarded[f] && lines.guarded[g])
                });
            mask |= u8::from(border) << k;
        }
        mask
    });
    let Some(m) = lines.mirror else {
        return hard;
    };
    gs_par::map(faces.len(), |f| {
        let mut mask = hard[f];
        for k in 0..arity(faces[f]) {
            let g = m.half(faces, 4 * f + k);
            mask |= (hard[g / 4] >> (g % 4) & 1) << k;
        }
        mask
    })
}
