//! A slow, obvious reference for one dab of each brush kernel, in `f64`, written from the
//! specification in the doc comment of gs-core's `brush` module. It reads the preset file
//! itself and shares nothing else with the engine (ARCHITECTURE 10.1, kind 3).
//!
//! "One dab" means: for a dab stroke, the press alone; for a hook, a press and a second sample
//! closer than half a spacing step (the dab in the middle of the unfinished first cell); for a
//! drag, a grab and one pointer position. No symmetry. Presets with automasks are refused: what those
//! add is checked by other tests. A preset with an alpha needs the alpha's levels as data
//! ([`Dab::alpha`]); the reference picks the level, reads it and applies the sign rule itself.
//! A `hide` face column hides faces as the specification says: hidden vertices keep still, the
//! frame reads visible vertices, and neighbours are found across visible faces only.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, HIDE, MASK, PlainMesh};
use toml::{Table, Value};

use super::alpha::{Level, sample};
use crate::measure::{P, add, corners, cross, dot, norm, scale, sub};

/// The stroke that makes the one dab.
#[derive(Clone, Copy, Debug)]
pub struct Dab<'a> {
    /// The levels of the preset's alpha (its picture through its tone settings), finest first.
    pub alpha: Option<&'a [Level]>,
    /// The press: a surface point, or the grab of a drag.
    pub at: P,
    /// The second sample of a hook or drag stroke.
    pub to: Option<P>,
    /// Brush radius at the samples, before the size pressure curve.
    pub radius: f64,
    pub pressure: f64,
    /// The direction the camera looks, unit length.
    pub view: P,
    /// The stroke's strength; the preset's when `None`.
    pub strength: Option<f64>,
    pub invert: bool,
}

/// The mesh after the dab.
#[derive(Clone, Debug, PartialEq)]
pub struct After {
    pub positions: Vec<P>,
    /// The vertex mask (zeros where the mesh has none).
    pub mask: Vec<f64>,
}

/// The preset file, read key by key with the documented defaults.
struct Doc(Table);

impl Doc {
    fn get(&self, path: &str) -> Option<&Value> {
        let mut parts = path.split('.');
        let first = self.0.get(parts.next()?)?;
        parts.try_fold(first, |v, k| v.get(k))
    }
    fn num(&self, path: &str, default: f64) -> f64 {
        number(self.get(path)).unwrap_or(default)
    }
    fn text(&self, path: &str, default: &str) -> String {
        self.get(path)
            .and_then(Value::as_str)
            .unwrap_or(default)
            .to_string()
    }
    fn flag(&self, path: &str, default: bool) -> bool {
        self.get(path).and_then(Value::as_bool).unwrap_or(default)
    }
    fn vec3(&self, path: &str, default: P) -> P {
        let got = self.get(path).and_then(Value::as_array).map(|a| {
            let v: Vec<f64> = a.iter().filter_map(|x| number(Some(x))).collect();
            v.try_into().ok()
        });
        got.flatten().unwrap_or(default)
    }
    /// A curve: a list of points (all sharp), or `{ points, smooth, sharp }`.
    fn curve(&self, path: &str, default: [[f64; 2]; 2]) -> Curve {
        let read = |a: &Vec<Value>| -> Vec<[f64; 2]> {
            a.iter()
                .filter_map(|p| Some([number(p.get(0))?, number(p.get(1))?]))
                .collect()
        };
        match self.get(path) {
            Some(Value::Array(a)) => Curve::sharp(read(a)),
            Some(Value::Table(t)) => {
                let pts = t.get("points").and_then(Value::as_array).map(read);
                let smooth = t.get("smooth").and_then(Value::as_bool).unwrap_or(false);
                let sharp: Vec<usize> = (t.get("sharp").and_then(Value::as_array))
                    .map(|s| s.iter().filter_map(Value::as_integer).map(|i| i as usize))
                    .into_iter()
                    .flatten()
                    .collect();
                let pts = pts.unwrap_or_default();
                let kinds = (0..pts.len())
                    .map(|i| smooth && !sharp.contains(&i))
                    .collect();
                Curve { pts, smooth: kinds }
            }
            _ => Curve::sharp(default.to_vec()),
        }
    }
}

fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Float(x) => Some(*x),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// A curve as the brush module specifies it: straight segments between sharp points, cubic
/// Hermite segments with Fritsch-Carlson tangents at smooth points, read through a table of
/// 257 values when any point is smooth.
pub struct Curve {
    pts: Vec<[f64; 2]>,
    smooth: Vec<bool>,
}

impl Curve {
    fn sharp(pts: Vec<[f64; 2]>) -> Curve {
        let n = pts.len();
        Curve {
            pts,
            smooth: vec![false; n],
        }
    }

    /// The curve through `pts`, smooth where `smooth` says.
    pub fn new(pts: Vec<[f64; 2]>, smooth: Vec<bool>) -> Curve {
        Curve { pts, smooth }
    }

    /// The value at `x`, clamped to the curve's ends.
    pub fn at(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        if !self.smooth.iter().any(|&s| s) {
            return self.straight(x);
        }
        let s = x * 256.0;
        let i = (s.floor() as usize).min(255);
        let (a, b) = (
            self.cubic(i as f64 / 256.0),
            self.cubic((i + 1) as f64 / 256.0),
        );
        a + (b - a) * (s - i as f64)
    }

    fn straight(&self, x: f64) -> f64 {
        let c = &self.pts;
        let last = &c[c.len().saturating_sub(2)..];
        let w = c.windows(2).find(|w| x <= w[1][0]).unwrap_or(last);
        w[0][1] + (w[1][1] - w[0][1]) * (x - w[0][0]) / (w[1][0] - w[0][0])
    }

    /// The exact Hermite curve at `x`.
    pub fn cubic(&self, x: f64) -> f64 {
        let c = &self.pts;
        let n = c.len();
        let slope = |k: usize| (c[k + 1][1] - c[k][1]) / (c[k + 1][0] - c[k][0]);
        // The segment holding x: the last one starting at or before it.
        let k = (1..n - 1).filter(|&i| c[i][0] <= x).count();
        let tangent = |i: usize| {
            if !self.smooth[i] || i == 0 || i == n - 1 {
                return slope(k);
            }
            let (a, b) = (slope(i - 1), slope(i));
            if a * b <= 0.0 {
                0.0
            } else {
                2.0 * a * b / (a + b)
            }
        };
        let h = c[k + 1][0] - c[k][0];
        let t = ((x - c[k][0]) / h).clamp(0.0, 1.0);
        let (m0, m1) = (tangent(k) * h, tangent(k + 1) * h);
        let basis = [
            2.0 * t * t * t - 3.0 * t * t + 1.0,
            t * t * t - 2.0 * t * t + t,
            -2.0 * t * t * t + 3.0 * t * t,
            t * t * t - t * t,
        ];
        basis[0] * c[k][1] + basis[1] * m0 + basis[2] * c[k + 1][1] + basis[3] * m1
    }
}

/// A curve at `x` (see [`Curve::at`]).
fn curve_at(c: &Curve, x: f64) -> f64 {
    c.at(x)
}

/// The tip falloff at `d` brush radii.
fn falloff(doc: &Doc, d: f64) -> f64 {
    if d >= 1.0 || d.is_nan() {
        return 0.0;
    }
    let h = doc.num("tip.hardness", 0.0);
    let mut d = ((d - h) / (1.0 - h)).max(0.0);
    // Falloff 0 raises to the power 1, which leaves `d` exactly as it is.
    d = libm::pow(d, libm::exp2(-doc.num("tip.focal", 0.0) / 50.0));
    match doc.text("tip.profile", "smooth").as_str() {
        "sphere" => (1.0 - d * d).sqrt(),
        "sharp" => (1.0 - d) * (1.0 - d),
        "linear" => 1.0 - d,
        "constant" => 1.0,
        "custom" => curve_at(&doc.curve("tip.points", [[0.0, 1.0], [1.0, 0.0]]), d),
        _ => 3.0 * d * d * d * d - 4.0 * d * d * d + 1.0,
    }
}

/// The mesh with what every kernel reads, in `f64`.
struct Mesh {
    pos: Vec<P>,
    nrm: Vec<P>,
    /// Each face's corners.
    faces: Vec<Vec<usize>>,
    /// Edge neighbours of each vertex across visible faces, with how many of those faces use
    /// each edge.
    nb: Vec<BTreeMap<usize, u32>>,
    mask: Vec<f64>,
    /// Vertices whose faces are all hidden.
    hidden: Vec<bool>,
}

/// Vertex normals: the sum of the face normals around each vertex (a quad's from its
/// diagonals), made unit length.
fn normals(pos: &[P], faces: &[Vec<usize>]) -> Vec<P> {
    let mut nrm = vec![[0.0; 3]; pos.len()];
    for c in faces {
        let at = |k: usize| pos[c[k]];
        let n = if c.len() == 3 {
            cross(sub(at(1), at(0)), sub(at(2), at(0)))
        } else {
            cross(sub(at(2), at(0)), sub(at(3), at(1)))
        };
        for &v in c {
            nrm[v] = add(nrm[v], n);
        }
    }
    let unit = |n: P| {
        if norm(n) == 0.0 {
            n
        } else {
            scale(n, 1.0 / norm(n))
        }
    };
    nrm.into_iter().map(unit).collect()
}

impl Mesh {
    fn new(m: &PlainMesh) -> Mesh {
        let pos: Vec<P> = m.positions.iter().map(|p| p.map(f64::from)).collect();
        let faces: Vec<Vec<usize>> = (m.faces.iter())
            .map(|f| corners(f).iter().map(|&v| v as usize).collect())
            .collect();
        let mut nb = vec![BTreeMap::new(); pos.len()];
        let hide: Vec<bool> = match m.attributes.get(HIDE).map(|a| &a.values) {
            Some(AttrValues::U32(h)) => h.iter().map(|&x| x != 0).collect(),
            _ => vec![false; m.faces.len()],
        };
        // A vertex is hidden when it has faces and every one of them is hidden.
        let mut faces_of = vec![(0u32, 0u32); pos.len()];
        for (c, &h) in faces.iter().zip(&hide) {
            for &v in c {
                faces_of[v].0 += 1;
                faces_of[v].1 += u32::from(h);
            }
        }
        let hidden = faces_of.iter().map(|&(n, h)| n > 0 && n == h).collect();
        // Neighbours across visible faces only; normals (below) count every face.
        for (c, _) in faces.iter().zip(&hide).filter(|(_, h)| !**h) {
            for (k, &v) in c.iter().enumerate() {
                for u in [c[(k + 1) % c.len()], c[(k + c.len() - 1) % c.len()]] {
                    *nb[v].entry(u).or_insert(0) += 1;
                }
            }
        }
        let mask = match m.attributes.get(MASK) {
            Some(a) if a.domain == AttrDomain::Vertex => match &a.values {
                AttrValues::F32(v) => v.iter().map(|&x| f64::from(x)).collect(),
                AttrValues::U32(_) => vec![0.0; pos.len()],
            },
            _ => vec![0.0; pos.len()],
        };
        Mesh {
            nrm: normals(&pos, &faces),
            pos,
            faces,
            nb,
            mask,
            hidden,
        }
    }

    /// The same mesh with its points at `pos`, normals recomputed.
    fn moved(&self, pos: Vec<P>) -> Mesh {
        Mesh {
            nrm: normals(&pos, &self.faces),
            pos,
            faces: self.faces.clone(),
            nb: self.nb.clone(),
            mask: self.mask.clone(),
            hidden: self.hidden.clone(),
        }
    }

    /// The neighbours a vertex averages: its boundary ones if it has any, else all.
    fn chosen(&self, v: usize) -> Vec<usize> {
        let edge = self.nb[v].values().any(|&c| c == 1);
        let keep = |c: u32| !edge || c == 1;
        (self.nb[v].iter())
            .filter(|e| keep(*e.1))
            .map(|e| *e.0)
            .collect()
    }

    fn mean(&self, v: usize, f: impl Fn(usize) -> P) -> Option<P> {
        let us = self.chosen(v);
        let sum = us.iter().fold([0.0; 3], |s, &u| add(s, f(u)));
        (!us.is_empty()).then(|| scale(sum, 1.0 / us.len() as f64))
    }

    /// Distance along edges from `from` to every vertex within `limit`: Bellman-Ford over the
    /// vertices close enough to matter, slow, bounded and obviously right.
    fn along_edges(&self, from: usize, limit: f64) -> BTreeMap<usize, f64> {
        let near: Vec<usize> = (0..self.pos.len())
            .filter(|&u| norm(sub(self.pos[u], self.pos[from])) <= limit)
            .collect();
        let mut dist = BTreeMap::from([(from, 0.0)]);
        for _ in 0..near.len() {
            for &v in &near {
                let Some(&dv) = dist.get(&v) else { continue };
                for &u in self.nb[v].keys() {
                    let d = dv + norm(sub(self.pos[u], self.pos[v]));
                    if d <= limit {
                        let e = dist.entry(u).or_insert(f64::INFINITY);
                        *e = e.min(d);
                    }
                }
            }
        }
        dist
    }
}

/// Area normal and area center around `c`.
fn frame(doc: &Doc, m: &Mesh, c: P, r: f64, view: P) -> (P, P) {
    let rn = doc.num("sampling.normal_radius", 0.5) * r;
    let rp = doc.num("sampling.position_radius", 0.5) * r;
    let front = doc.flag("sampling.front_only", true);
    let (mut sn, mut sp, mut sw) = ([0.0; 3], [0.0; 3], 0.0);
    for ((p, n), &hidden) in m.pos.iter().zip(&m.nrm).zip(&m.hidden) {
        if hidden || (front && dot(*n, view) >= 0.0) {
            continue;
        }
        let d = norm(sub(*p, c));
        let (wn, wp) = ((1.0 - d / rn).max(0.0), (1.0 - d / rp).max(0.0));
        sn = add(sn, scale(*n, wn));
        sp = add(sp, scale(*p, wp));
        sw += wp;
    }
    let normal = if norm(sn) > 0.0 {
        scale(sn, 1.0 / norm(sn))
    } else {
        scale(view, -1.0)
    };
    let origin = if sw > 0.0 { scale(sp, 1.0 / sw) } else { c };
    (normal, origin)
}

/// One dab of the preset `text` on `mesh`.
pub fn dab(text: &str, mesh: &PlainMesh, dab: &Dab) -> Result<After, String> {
    let doc = Doc(toml::from_str(text).map_err(|e| e.to_string())?);
    let auto = ["backface", "cavity", "topology", "face_group"];
    if auto
        .iter()
        .any(|k| doc.num(&format!("automask.{k}"), 0.0) != 0.0)
    {
        return Err("the reference does not model automasks".into());
    }
    let m = Mesh::new(mesh);
    let p = dab.pressure;
    let flat = [[0.0, 1.0], [1.0, 1.0]];
    let r = dab.radius * curve_at(&doc.curve("pressure.size", flat), p);
    let strength = dab.strength.unwrap_or(doc.num("strength", 0.0))
        * curve_at(&doc.curve("pressure.strength", [[0.0, 0.0], [1.0, 1.0]]), p);
    let kind = doc.text("kernel.type", "");
    let (center, delta) = match (kind.as_str(), dab.to) {
        ("hook", Some(to)) => (dab.at, scale(sub(to, dab.at), 0.5)),
        ("move", Some(to)) => (dab.at, sub(to, dab.at)),
        _ => (dab.at, [0.0; 3]),
    };
    // Subtract turns every stroke the other way, Alt back again.
    let invert = dab.invert != (doc.text("mode", "add") == "subtract");
    let (normal, origin) = frame(&doc, &m, center, r, dab.view);
    let delta = if kind == "move" && invert {
        scale(normal, dot(normal, delta))
    } else {
        delta
    };
    // A drag stands for the length of its offset; a press for `tap` radii.
    let amount = if kind == "move" {
        strength * norm(delta) / r
    } else {
        strength * doc.num("stroke.tap", 0.5)
    };
    let stamp = match (doc.get("tip.alpha"), dab.alpha) {
        (Some(_), Some(levels)) => Some(Stamp::new(&doc, &m, levels, center, r, normal, delta)),
        (Some(_), None) => return Err("the preset names an alpha: give its levels".into()),
        _ => None,
    };
    let d = Kernel {
        stamp,
        doc: &doc,
        m: &m,
        c: center,
        r,
        amount,
        strength,
        normal,
        origin,
        view: dab.view,
        delta,
        invert,
        imbed: doc.num("depth.imbed", 0.0) * curve_at(&doc.curve("pressure.imbed", flat), p),
        modifier: doc.num("modifier", 0.0) / 100.0
            * curve_at(&doc.curve("pressure.modifier", flat), p),
    };
    d.run(&kind)
}

/// A dab's alpha: the level it reads, its axes and how the brush uses it.
#[derive(Clone)]
struct Stamp<'a> {
    level: &'a Level,
    t: P,
    b: P,
    /// Where the weights reach: the picture's square spans it.
    reach: f64,
    mid: f64,
    tiles: [u32; 2],
    /// The Offset kernel keeps the factor's sign.
    signed: bool,
}

impl<'a> Stamp<'a> {
    fn new(doc: &Doc, m: &Mesh, levels: &'a [Level], c: P, r: f64, n: P, dir: P) -> Stamp<'a> {
        // The axes: the dab's direction within the plane across n, or, when that part is under
        // a hundred-thousandth of the direction, the coordinate axis most across n (the first
        // on a tie).
        let across = |e: P| cross(cross(n, e), n);
        let mut t = across(dir);
        if norm(t) <= 1e-5 * norm(dir) {
            let k = (0..3).fold(0, |k, i| if n[i].abs() < n[k].abs() { i } else { k });
            let mut e = [0.0; 3];
            e[k] = 1.0;
            t = across(e);
        }
        let t = scale(t, 1.0 / norm(t));
        // The level: vertices within the larger sampling radius set the spacing.
        let big = doc
            .num("sampling.normal_radius", 0.5)
            .max(doc.num("sampling.position_radius", 0.5))
            * r;
        let near = (m.pos.iter().zip(&m.hidden))
            .filter(|(p, hidden)| !**hidden && norm(sub(**p, c)) <= big)
            .count();
        let spacing = big * (std::f64::consts::PI / near.max(1) as f64).sqrt();
        let tile = |k: usize| {
            let t = doc.get("tip.alpha.tiles").and_then(Value::as_array);
            t.and_then(|a| a.get(k))
                .and_then(Value::as_integer)
                .unwrap_or(1) as u32
        };
        let tiles = [tile(0), tile(1)];
        let fits = |l: &&Level| {
            let texels =
                (f64::from(tiles[0]) * l.width as f64).max(f64::from(tiles[1]) * l.height as f64);
            2.0 * r / texels >= 0.5 * spacing
        };
        let level = levels
            .iter()
            .find(fits)
            .unwrap_or(&levels[levels.len() - 1]);
        let elastic = doc.text("kernel.select", "") == "elastic";
        Stamp {
            level,
            t,
            b: cross(n, t),
            reach: if elastic {
                r * doc.num("kernel.support", 4.0)
            } else {
                r
            },
            mid: doc.num("tip.alpha.mid", 0.0),
            tiles,
            signed: doc.text("kernel.type", "") == "offset",
        }
    }

    /// The factor at `p` of a dab centered at `c`; `size` for a post effect, which reads only
    /// the size of a signed factor.
    fn factor(&self, p: P, c: P, size: bool) -> f64 {
        let rel = sub(p, c);
        let (u, v) = (dot(rel, self.t) / self.reach, dot(rel, self.b) / self.reach);
        let x = sample(self.level, u, v, self.tiles);
        match (self.signed, size) {
            (true, false) => x - self.mid,
            (true, true) => (x - self.mid).abs(),
            _ if self.mid >= 1.0 => 0.0,
            _ => (x - self.mid).max(0.0) / (1.0 - self.mid),
        }
    }
}

/// One dab's settings and frame.
#[derive(Clone)]
struct Kernel<'a> {
    stamp: Option<Stamp<'a>>,
    doc: &'a Doc,
    m: &'a Mesh,
    c: P,
    r: f64,
    amount: f64,
    strength: f64,
    normal: P,
    origin: P,
    view: P,
    delta: P,
    invert: bool,
    /// The plane's height above the area center, in radii.
    imbed: f64,
    /// The Brush Modifier, -1 to 1.
    modifier: f64,
}

impl Kernel<'_> {
    /// Influence of vertex `v`: shape times the alpha's factor, times `1 - mask` (not for the
    /// mask brush); 0 when hidden. `post` for a post effect.
    fn weight(&self, v: usize, pos: &[P], connected: &BTreeMap<usize, f64>, post: bool) -> f64 {
        let doc = self.doc;
        if self.m.hidden[v] {
            return 0.0;
        }
        let d = norm(sub(pos[v], self.c));
        let mut shape = match doc.text("kernel.select", "").as_str() {
            "elastic" => {
                let x = d / (self.r * doc.num("kernel.support", 4.0));
                let u = (1.0 - x * x).max(0.0);
                u * u
            }
            "connected" => connected.get(&v).map_or(0.0, |g| falloff(doc, g / self.r)),
            _ => falloff(doc, d / self.r),
        };
        if let Some(s) = &self.stamp
            && shape != 0.0
        {
            shape *= s.factor(pos[v], self.c, post);
        }
        let painting = doc.text("kernel.type", "") == "attribute";
        if painting {
            shape
        } else {
            shape * (1.0 - self.m.mask[v].clamp(0.0, 1.0))
        }
    }

    fn run(&self, kind: &str) -> Result<After, String> {
        let m = self.m;
        let nearest = (m.pos.iter().enumerate())
            .filter(|&(i, _)| !m.hidden[i])
            .map(|(i, p)| (norm(sub(*p, self.c)), i))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let connected = match (self.doc.text("kernel.select", "").as_str(), nearest) {
            ("connected", Some((_, v))) => m.along_edges(v, self.r),
            _ => BTreeMap::new(),
        };
        // A Smooth dab runs as sub-steps, each a whole pass with an equal share of the amount on
        // the positions and normals the one before left; the post pass runs once, after them.
        let n = if kind == "smooth" {
            smooth_steps(self.doc.num("kernel.gain", 0.0), self.amount)
        } else {
            1
        };
        let share = Kernel {
            amount: self.amount / f64::from(n),
            ..self.clone()
        };
        let mut out = share.pass(kind, &connected)?;
        for _ in 1..n {
            let now = m.moved(out.positions);
            let step = Kernel {
                m: &now,
                ..share.clone()
            };
            out = step.pass(kind, &connected)?;
        }
        // With no auto-smooth (gain 0), or on a part with more faces than its limit, this
        // pass changes nothing.
        let gain = self.doc.num("post.auto_smooth", 0.0);
        let limit = self.doc.num("post.auto_smooth_max_faces", 0.0);
        if gain > 0.0 && (limit == 0.0 || self.m.faces.len() as f64 <= limit) {
            out.positions = self.auto_smooth(out.positions, gain, &connected);
        }
        Ok(out)
    }

    /// One pass of the kernel over every vertex, gravity included.
    fn pass(&self, kind: &str, connected: &BTreeMap<usize, f64>) -> Result<After, String> {
        let m = self.m;
        let mut out = After {
            positions: m.pos.clone(),
            mask: m.mask.clone(),
        };
        for v in 0..m.pos.len() {
            // Negative only below a signed alpha's mid value, where it carves.
            let w = self.weight(v, &m.pos, connected, false);
            if w == 0.0 {
                continue;
            }
            if kind == "attribute" {
                out.mask[v] = self.paint(v, w)?;
                continue;
            }
            let g = self.doc.num("post.gravity", 0.0);
            let gdir = self.doc.vec3("post.gravity_dir", [0.0, -1.0, 0.0]);
            let pull = scale(gdir, g * w.abs() * self.amount * self.r / norm(gdir));
            out.positions[v] = add(add(m.pos[v], self.translate(kind, v, w)?), pull);
        }
        Ok(out)
    }

    /// The Smooth pass after the kernel in `post.auto_smooth_mode`, read from the moved
    /// positions and their normals.
    fn auto_smooth(&self, pos: Vec<P>, gain: f64, connected: &BTreeMap<usize, f64>) -> Vec<P> {
        let moved = self.m.moved(pos);
        let mode = self.doc.text("post.auto_smooth_mode", "uniform");
        (0..moved.pos.len())
            .map(|v| {
                let w = self.weight(v, &moved.pos, connected, true);
                let at = moved.pos[v];
                match smooth_target(&moved, v, &mode) {
                    Some(d) if w > 0.0 => {
                        add(at, scale(d, 1.0 - libm::exp(-gain * w * self.amount)))
                    }
                    _ => at,
                }
            })
            .collect()
    }

    fn translate(&self, kind: &str, v: usize, w: f64) -> Result<P, String> {
        let (doc, p) = (self.doc, self.m.pos[v]);
        Ok(match kind {
            "offset" => self.offset(v, w),
            "plane" => {
                let n = if self.invert {
                    scale(self.normal, -1.0)
                } else {
                    self.normal
                };
                let q = add(self.origin, scale(n, self.imbed * self.r));
                let d = dot(sub(p, q), n);
                let height = doc.num("kernel.height", f64::INFINITY) * self.r;
                let depth = doc.num("kernel.depth", f64::INFINITY) * self.r;
                if d > height || -d > depth {
                    return Ok([0.0; 3]);
                }
                // Only the part of the distance on the moving side counts.
                let d = match doc.text("kernel.side", "both").as_str() {
                    "above" => d.max(0.0),
                    "below" => d.min(0.0),
                    _ => d,
                };
                scale(
                    n,
                    -d * (1.0 - libm::exp(-doc.num("kernel.gain", 0.0) * w * self.amount)),
                )
            }
            "smooth" => {
                let mode = doc.text("kernel.mode", "uniform");
                let raw = smooth_target(self.m, v, &mode).unwrap_or([0.0; 3]);
                scale(raw, {
                    let k = 1.0 - libm::exp(-doc.num("kernel.gain", 0.0) * w * self.amount);
                    if self.invert { -k } else { k }
                })
            }
            "move" => {
                let field = if doc.text("kernel.select", "") == "elastic" {
                    kelvinlet(
                        sub(p, self.c),
                        self.delta,
                        self.r,
                        doc.num("kernel.poisson", 0.45),
                    )
                } else {
                    self.delta
                };
                scale(field, w * self.strength)
            }
            "hook" => {
                // Without a step there is no line of motion, so nothing moves.
                let len = norm(self.delta);
                if len == 0.0 {
                    return Ok([0.0; 3]);
                }
                let dir = scale(self.delta, 1.0 / len);
                let to = sub(self.c, p);
                let across = sub(to, scale(dir, dot(dir, to)));
                scale(
                    add(self.delta, scale(across, doc.num("kernel.pinch", 0.0))),
                    w * self.strength,
                )
            }
            other => return Err(format!("the reference has no kernel {other}")),
        })
    }

    /// The preset's terms and the Brush Modifier's: `(dir, gain, exponent, tangent)`.
    fn terms(&self) -> Vec<(String, f64, f64, bool)> {
        let listed = self.doc.get("kernel.terms").and_then(Value::as_array);
        let mut terms: Vec<_> = (listed.into_iter().flatten())
            .map(|t| {
                (
                    t.get("dir")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    number(t.get("gain")).unwrap_or(0.0),
                    number(t.get("exponent")).unwrap_or(1.0),
                    t.get("tangent").and_then(Value::as_bool).unwrap_or(false),
                )
            })
            .collect();
        let m = self.modifier;
        if m > 0.0 {
            terms.push(("to_center".into(), 2.0 * m, 1.0, true));
        } else if m < 0.0 {
            terms.push(("vertex_normal".into(), -0.5 * m, 1.0, false));
        }
        terms
    }

    fn offset(&self, v: usize, w: f64) -> P {
        let (m, p) = (self.m, self.m.pos[v]);
        let mut d = [0.0; 3];
        for (dir, gain, exponent, tangent) in self.terms() {
            let gain = if self.invert { -gain } else { gain };
            // Below an alpha's mid value the weight is negative and pushes the other way.
            let a = gain * libm::pow(w.abs(), exponent).copysign(w) * self.amount;
            let along = |dir: P| scale(dir, a * self.r);
            let term = match dir.as_str() {
                "to_center" => {
                    let mut to = sub(self.c, p);
                    if tangent {
                        to = sub(to, scale(self.normal, dot(self.normal, to)));
                    }
                    scale(to, 1.0 - libm::exp(-a))
                }
                "area_normal" => along(self.normal),
                "vertex_normal" => along(m.nrm[v]),
                "view" => along(scale(self.view, -1.0)),
                "x" => along([1.0, 0.0, 0.0]),
                "y" => along([0.0, 1.0, 0.0]),
                _ => along([0.0, 0.0, 1.0]),
            };
            d = add(d, term);
        }
        d
    }

    /// The mask brush.
    fn paint(&self, v: usize, w: f64) -> Result<f64, String> {
        let doc = self.doc;
        if doc.text("kernel.column", "") != "mask" {
            return Err("the reference paints only the mask".into());
        }
        let (m, old) = (self.m, self.m.mask[v]);
        let around = || {
            let us: Vec<usize> = m.nb[v].keys().copied().collect();
            us.iter().map(|&u| m.mask[u]).sum::<f64>() / us.len().max(1) as f64
        };
        let target = match doc.text("kernel.op", "set").as_str() {
            "blur" => around(),
            "sharpen" => (2.0 * old - around()).clamp(0.0, 1.0),
            _ if self.invert => 0.0,
            _ => doc.num("kernel.value", 0.0),
        };
        let frac = 1.0 - libm::exp(-doc.num("kernel.gain", 0.0) * w * self.amount);
        Ok(old + (target - old) * frac)
    }
}

/// The Smooth target's offset from vertex `v` of `m`, by mode.
/// How many sub-steps a Smooth dab runs: `ceil(gain * amount / 0.5)`, from 1 to 32, found in
/// `f32` as the specification says, so the count is gs-core's.
fn smooth_steps(gain: f64, amount: f64) -> u32 {
    let x = gain as f32 * amount as f32 / 0.5;
    if x > 1.0 {
        (x.ceil() as u32).min(32)
    } else {
        1
    }
}

fn smooth_target(m: &Mesh, v: usize, mode: &str) -> Option<P> {
    let (p, n) = (m.pos[v], m.nrm[v]);
    let flat = |d: P| sub(d, scale(n, dot(n, d)));
    match mode {
        "stronger" => m
            .mean(v, |u| m.mean(u, |x| m.pos[x]).unwrap_or(m.pos[u]))
            .map(|t| sub(t, p)),
        "tangent" => m.mean(v, |u| m.pos[u]).map(|t| flat(sub(t, p))),
        "relax" => {
            let len = m.mean(v, |u| [norm(sub(m.pos[u], p)), 0.0, 0.0])?[0];
            let pull = m.mean(v, |u| {
                let e = sub(m.pos[u], p);
                if norm(e) == 0.0 {
                    [0.0; 3]
                } else {
                    scale(e, 1.0 - len / norm(e))
                }
            })?;
            Some(flat(pull))
        }
        _ => m.mean(v, |u| m.pos[u]).map(|t| sub(t, p)),
    }
}

/// A regularized Kelvinlet pulled by `t` at offset `rel`, scaled so the grab moves by `t`.
fn kelvinlet(rel: P, t: P, eps: f64, nu: f64) -> P {
    // With the stiffness term `a` set to 1 (it cancels in the scaling): b = a / (4 (1 - nu)).
    let b = 1.0 / (4.0 * (1.0 - nu));
    let re = (dot(rel, rel) + eps * eps).sqrt();
    let alpha = (1.0 - b) / re + eps * eps / (2.0 * re * re * re);
    let beta = b / (re * re * re);
    let u = add(scale(t, alpha), scale(rel, beta * dot(rel, t)));
    scale(u, eps / (1.5 - b))
}
