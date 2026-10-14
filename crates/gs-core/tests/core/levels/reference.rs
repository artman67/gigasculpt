//! A slow, obviously correct Catmull-Clark step in f64, written from the textbook rules
//! (Catmull and Clark 1978; `OpenSubdiv`'s masks for creases and boundaries with integer
//! sharpness), numbering new points by decision 0006's rule. It shares no code with gs-core.

use std::collections::BTreeMap;

use gs_core::NO;

pub type P = [f64; 3];

fn add(a: P, b: P) -> P {
    std::array::from_fn(|k| a[k] + b[k])
}

fn scale(a: P, s: f64) -> P {
    a.map(|x| x * s)
}

fn mean(ps: &[P]) -> P {
    let sum = ps.iter().fold([0.0; 3], |s, &p| add(s, p));
    scale(sum, 1.0 / ps.len() as f64)
}

fn corners(f: [u32; 4]) -> Vec<u32> {
    f.iter().copied().filter(|&v| v != NO).collect()
}

fn edge(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}

/// One level: faces, positions, sharpness per edge (absent means 0).
pub struct Mesh {
    pub faces: Vec<[u32; 4]>,
    pub pos: Vec<P>,
    pub sharp: BTreeMap<(u32, u32), u8>,
}

/// What one step reads of a level: the faces using each edge and each vertex.
struct Adjacency<'m> {
    m: &'m Mesh,
    edge_faces: BTreeMap<(u32, u32), Vec<usize>>,
    vertex_faces: Vec<Vec<usize>>,
}

impl<'m> Adjacency<'m> {
    fn of(m: &'m Mesh) -> Self {
        let mut edge_faces: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
        let mut vertex_faces: Vec<Vec<usize>> = vec![Vec::new(); m.pos.len()];
        for (fi, &f) in m.faces.iter().enumerate() {
            let c = corners(f);
            for i in 0..c.len() {
                let e = edge(c[i], c[(i + 1) % c.len()]);
                edge_faces.entry(e).or_default().push(fi);
                vertex_faces[c[i] as usize].push(fi);
            }
        }
        Self {
            m,
            edge_faces,
            vertex_faces,
        }
    }

    fn centroid(&self, fi: usize) -> P {
        let c = corners(self.m.faces[fi]);
        mean(
            &c.iter()
                .map(|&v| self.m.pos[v as usize])
                .collect::<Vec<_>>(),
        )
    }

    /// Sharp by its crease, or by having other than two faces (a boundary or non-manifold).
    fn sharp(&self, e: (u32, u32)) -> bool {
        self.m.sharp.get(&e).copied().unwrap_or(0) > 0 || self.edge_faces[&e].len() != 2
    }

    fn edge_point(&self, e: (u32, u32), smooth: bool) -> P {
        let (a, b) = (self.m.pos[e.0 as usize], self.m.pos[e.1 as usize]);
        if !smooth || self.sharp(e) {
            return mean(&[a, b]);
        }
        let fs = &self.edge_faces[&e];
        scale(
            add(add(a, b), add(self.centroid(fs[0]), self.centroid(fs[1]))),
            0.25,
        )
    }

    fn vertex_point(&self, v: u32, smooth: bool, pin_corners: bool) -> P {
        let p = self.m.pos[v as usize];
        let fs = &self.vertex_faces[v as usize];
        let mut nbs: Vec<u32> = Vec::new();
        for &fi in fs {
            let c = corners(self.m.faces[fi]);
            let k = c.len();
            let j = c.iter().position(|&u| u == v).unwrap_or(0);
            for u in [c[(j + k - 1) % k], c[(j + 1) % k]] {
                if !nbs.contains(&u) {
                    nbs.push(u);
                }
            }
        }
        let sharp: Vec<u32> = nbs
            .iter()
            .copied()
            .filter(|&u| self.sharp(edge(v, u)))
            .collect();
        if !smooth || nbs.is_empty() {
            p
        } else if sharp.len() < 2 {
            let k = nbs.len() as f64;
            let us = nbs
                .iter()
                .fold([0.0; 3], |s, &u| add(s, self.m.pos[u as usize]));
            let cs = fs.iter().fold([0.0; 3], |s, &fi| add(s, self.centroid(fi)));
            add(scale(p, (k - 2.0) / k), scale(add(us, cs), 1.0 / (k * k)))
        } else if sharp.len() == 2 && !(pin_corners && fs.len() == 1) {
            let ends = add(self.m.pos[sharp[0] as usize], self.m.pos[sharp[1] as usize]);
            add(scale(p, 0.75), scale(ends, 0.125))
        } else {
            p
        }
    }
}

/// A UV, and the UVs of one face's corners (a triangle's fourth unused).
pub type Uv = [f64; 2];
pub type Corners = [Uv; 4];

fn uv_sum(us: impl Iterator<Item = Uv>) -> Uv {
    us.fold([0.0; 2], |s, u| [s[0] + u[0], s[1] + u[1]])
}

fn uv_mean(us: &[Uv]) -> Uv {
    uv_sum(us.iter().copied()).map(|x| x / us.len() as f64)
}

/// Whether two UVs are the same, bit for bit, as the rule compares them.
fn same(a: Uv, b: Uv) -> bool {
    a.map(f64::to_bits) == b.map(f64::to_bits)
}

/// Face-varying Catmull-Clark with linear boundaries (`OpenSubdiv`'s
/// `FVAR_LINEAR_BOUNDARIES`) on the UVs `uv` of `m`'s face corners: one step, the children's
/// corner UVs in the order [`step`] makes the children. An edge is continuous when it has two
/// faces that agree on the UVs at both its ends; a point is inside its island when every face
/// around it gives it the same UV and every edge at it is continuous. Inside, the rules are the
/// ones positions follow (creases included); on an island border a point keeps its UV and an
/// edge point is its edge's midpoint, so borders stay where they are.
pub fn uv_step(m: &Mesh, uv: &[Corners], smooth: bool) -> Vec<Corners> {
    let adj = Adjacency::of(m);
    let at = |fi: usize, v: u32| -> Uv {
        let c = corners(m.faces[fi]);
        uv[fi][c.iter().position(|&u| u == v).unwrap_or(0)]
    };
    let fp = |fi: usize| uv_mean(&uv[fi][..corners(m.faces[fi]).len()]);
    let continuous = |e: (u32, u32)| {
        let fs = &adj.edge_faces[&e];
        fs.len() == 2 && [e.0, e.1].iter().all(|&v| same(at(fs[0], v), at(fs[1], v)))
    };
    let edge_point = |fi: usize, e: (u32, u32)| -> Uv {
        let (a, b) = (at(fi, e.0), at(fi, e.1));
        let creased = m.sharp.get(&e).copied().unwrap_or(0) > 0;
        if !smooth || creased || !continuous(e) {
            return uv_mean(&[a, b]);
        }
        let fs = &adj.edge_faces[&e];
        uv_mean(&[a, b, fp(fs[0]), fp(fs[1])])
    };
    let vertex_point = |fi: usize, v: u32| -> Uv {
        let own = at(fi, v);
        let fs = &adj.vertex_faces[v as usize];
        let mut nbs: Vec<u32> = Vec::new();
        for &g in fs {
            let c = corners(m.faces[g]);
            let k = c.len();
            let j = c.iter().position(|&u| u == v).unwrap_or(0);
            for u in [c[(j + k - 1) % k], c[(j + 1) % k]] {
                if !nbs.contains(&u) {
                    nbs.push(u);
                }
            }
        }
        let inside =
            fs.iter().all(|&g| same(at(g, v), own)) && nbs.iter().all(|&u| continuous(edge(v, u)));
        if !smooth || !inside {
            return own;
        }
        let nb = |u: u32| at(adj.edge_faces[&edge(v, u)][0], u);
        let creased: Vec<u32> = (nbs.iter().copied())
            .filter(|&u| m.sharp.get(&edge(v, u)).copied().unwrap_or(0) > 0)
            .collect();
        match creased.len() {
            0 | 1 => {
                let k = nbs.len() as f64;
                let us = uv_sum(nbs.iter().map(|&u| nb(u)));
                let cs = uv_sum(fs.iter().map(|&g| fp(g)));
                std::array::from_fn(|i| own[i] * (k - 2.0) / k + (us[i] + cs[i]) / (k * k))
            }
            2 => {
                let (x, y) = (nb(creased[0]), nb(creased[1]));
                std::array::from_fn(|i| own[i] * 0.75 + (x[i] + y[i]) * 0.125)
            }
            _ => own,
        }
    };
    let mut out = Vec::new();
    for (fi, &f) in m.faces.iter().enumerate() {
        let c = corners(f);
        let k = c.len();
        let e: Vec<Uv> = (0..k)
            .map(|i| edge_point(fi, edge(c[i], c[(i + 1) % k])))
            .collect();
        for j in 0..k {
            let mut q = [vertex_point(fi, c[j]), e[j], fp(fi), e[(j + k - 1) % k]];
            q.rotate_right(if j + 1 == k { 3 } else { j });
            out.push(q);
        }
    }
    out
}

/// One step: the faces and positions of the level above, and its sharp edges.
pub fn step(m: &Mesh, smooth: bool, pin_corners: bool) -> Mesh {
    let adj = Adjacency::of(m);
    let n = m.pos.len() as u32;
    let mut pos: Vec<P> = (0..n)
        .map(|v| adj.vertex_point(v, smooth, pin_corners))
        .collect();
    // Numbering: edges on first use in face and corner order, then each face's center.
    let mut number: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut faces = Vec::new();
    for (fi, &f) in m.faces.iter().enumerate() {
        let c = corners(f);
        let k = c.len();
        let mut e = Vec::new();
        for i in 0..k {
            let key = edge(c[i], c[(i + 1) % k]);
            let id = *number.entry(key).or_insert_with(|| {
                pos.push(adj.edge_point(key, smooth));
                pos.len() as u32 - 1
            });
            e.push(id);
        }
        pos.push(adj.centroid(fi));
        let center = pos.len() as u32 - 1;
        for j in 0..k {
            let mut q = [c[j], e[j], center, e[(j + k - 1) % k]];
            q.rotate_right(if j + 1 == k { 3 } else { j });
            faces.push(q);
        }
    }
    // Sharp edges one level up.
    let mut sharp = BTreeMap::new();
    for (&e, &s) in &m.sharp {
        let child = if s == 255 { 255 } else { s.saturating_sub(1) };
        if child > 0 {
            let mid = number[&e];
            sharp.insert(edge(e.0, mid), child);
            sharp.insert(edge(e.1, mid), child);
        }
    }
    Mesh { faces, pos, sharp }
}
