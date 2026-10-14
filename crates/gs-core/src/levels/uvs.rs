//! Smooth UVs (decision 0014, section 14): face-varying Catmull-Clark with island borders kept.
//!
//! With smooth UVs on, a smooth divide places the child corners' UVs by the rules positions
//! follow ([`super::rules`]), read on the UVs, wherever the layout is continuous, and by
//! decision 0006's linear rule elsewhere. An edge is continuous when it has two faces and they
//! give its ends the same UVs; it takes the edge rule. A point is inside its island when every
//! edge at it is continuous; it takes the vertex rule. A face's center is the mean of its
//! corners either way. On an island border (a seam, or the mesh's boundary) a point keeps its
//! UV and an edge point is its edge's midpoint, so the borders keep their lines: `OpenSubdiv`'s
//! linear boundaries (Apache 2.0, read for its behavior only). Creases act on UVs as they act on
//! positions.
//!
//! The level a smoothing step is made from keeps the UVs it had (`Level::uvs`), since the
//! levels above no longer hold them.
//!
//! [`Levels::write_uvs`] writes level 0's UVs into a stack (decision 0061 as amended
//! 2026-10-13, step UV5): every level above gets what dividing would have given it.

use std::sync::Arc;

use super::cut::Coarse;
use super::file::finer;
use super::fill::{Column, face_slot, held, scalar, value};
use super::rules::{Buf, Point, Rule, Topo, Uv, has_edge};
use super::{Base, Level, Levels, bad};
use crate::columns::{AttrDef, AttrKind, AttrVec, ColumnSet, Domain, MAX_ATTRS};
use crate::config::{FaceId, MeshError, NodeId};
use crate::math::{NO, arity};
use crate::numbering::Numbering;
use crate::store::MeshStore;
use crate::subdivide::{Step, child, corner_child, ends, subdivide_corners};

/// One face's UVs, corner 0 to corner 3, each `[u, v]`.
pub type Corners = [[f32; 2]; 4];

/// A point and its UV's bits, as some face gives them.
type Held = (u32, u32, u32);

/// The UV columns as the base's corner groups name them: `[corner][u or v]`.
type UvDefs = [[AttrDef; 2]; 4];

/// The UVs each level keeps, the base first.
type Sets = Vec<Option<Vec<Corners>>>;

/// A level a smoothing step is made from: its numbering, its faces as canonical indices in
/// canonical order, the step, [`Step::sources`] of the faces and each store face's canonical
/// index ([`Coarse`] without its store).
pub(crate) struct Over<'a> {
    pub num: &'a Numbering,
    pub faces: &'a [[u32; 4]],
    pub step: &'a Step,
    pub src: &'a [u32],
    pub fcanon: &'a [u32],
}

/// The UVs of the level `store` holds, numbered by `num`, by canonical face: corner column
/// groups `groups`, u then v.
fn read(store: &MeshStore, num: &Numbering, groups: &[[usize; 4]; 2]) -> Vec<Corners> {
    gs_par::map(num.face_count(), |fi| {
        let (n, slot) = face_slot(store, num.face(fi as u32).0);
        std::array::from_fn(|k| groups.map(|g| scalar(value(store, g[k], n, slot))))
    })
}

/// The corner column groups' columns in `defs`, u then v, when the base names two groups,
/// `defs` has all their columns and `store` holds one of them.
pub(crate) fn groups(base: &Base, defs: &[AttrDef], store: &MeshStore) -> Option<[[usize; 4]; 2]> {
    let [u, v] = base.corners.as_slice() else {
        return None;
    };
    let ids = |g: &[String; 4]| {
        let mut out = [0; 4];
        for (o, name) in out.iter_mut().zip(g) {
            *o = defs.iter().position(|d| &d.name == name)?;
        }
        Some(out)
    };
    let groups = [ids(u)?, ids(v)?];
    groups
        .iter()
        .flatten()
        .any(|&a| held(store, a))
        .then_some(groups)
}

/// The UV `held` (sorted) gives point `u`; the rules ask only for points of the faces held.
fn at(held: &[Held], u: u32) -> Uv {
    let i = held.partition_point(|h| h.0 < u);
    held.get(i)
        .filter(|h| h.0 == u)
        .map_or(Uv::ZERO, |h| Uv([f32::from_bits(h.1), f32::from_bits(h.2)]))
}

/// The UVs one smoothing step places: the coarse level's, by canonical face; each coarse
/// point's by the vertex rule when it is inside its island; each new edge point's by the edge
/// rule when its edge is continuous and smooth.
pub(crate) struct Smooth {
    pub coarse: Vec<Corners>,
    vertex: Vec<Option<Uv>>,
    /// By new point less the coarse point count; `None` for face centers.
    edge: Vec<Option<Uv>>,
    old: u32,
}

impl Smooth {
    /// The step over `c` by `rule` (a smooth one), its UVs read from the corner columns
    /// `groups`.
    pub fn of(c: &Coarse, rule: &Rule, groups: &[[usize; 4]; 2]) -> Smooth {
        let over = Over {
            num: c.num,
            faces: c.faces,
            step: c.step,
            src: c.src,
            fcanon: c.fcanon,
        };
        Smooth::over(&over, rule, read(c.store, c.num, groups))
    }

    /// The step over `c` by `rule` (a smooth one) from `coarse`, the level's UVs by canonical
    /// face.
    fn over(c: &Over, rule: &Rule, coarse: Vec<Corners>) -> Smooth {
        let old = c.num.vertex_count() as u32;
        let cfg = rule.topo.cfg;
        let starts = c.num.face_starts();
        let canon = |f: FaceId| {
            c.fcanon[(starts[cfg.face_node(f) as usize] + cfg.face_local(f)) as usize] as usize
        };
        // The points of faces `fs` with their UVs, sorted, if the faces agree on every point
        // they share.
        let agreed = |fs: &[usize]| {
            let mut out: Buf<Held, 32> = Buf::new();
            for &fi in fs {
                let f = c.faces[fi];
                for (k, &[u, v]) in coarse[fi].iter().enumerate().take(arity(f)) {
                    out.push((f[k], u.to_bits(), v.to_bits()));
                }
            }
            let one = (out.sorted().windows(2)).all(|w| w[0].0 != w[1].0 || w[0] == w[1]);
            one.then_some(out)
        };
        // Per new point: its edge is continuous, and the edge rule's UV when it is smooth.
        let (open, edge): (Vec<bool>, Vec<Option<Uv>>) =
            gs_par::map((c.step.count - old) as usize, |i| {
                let s = c.src[i] as usize;
                if c.step.center[s / 4] == old + i as u32 {
                    return (false, None);
                }
                let (a, b) = ends(c.faces[s / 4], s % 4);
                let mut fs: Buf<usize, 4> = Buf::new();
                for &f in rule.topo.ring(a) {
                    if has_edge(c.faces[canon(f)], a, b) {
                        fs.push(canon(f));
                    }
                }
                let fs = fs.sorted();
                let Some(mut held) = agreed(fs).filter(|_| fs.len() == 2) else {
                    return (false, None);
                };
                let (held, smooth) = (held.as_mut(), rule.sharpness(a, b) == 0);
                let uv = smooth.then(|| rule.edge(a, b, &|u| at(held, u)));
                (true, uv)
            })
            .into_iter()
            .unzip();
        let vertex = gs_par::map(old as usize, |v| {
            let v = v as u32;
            let mut fs: Buf<usize, 16> = Buf::new();
            for &f in rule.topo.ring(v) {
                let fi = canon(f);
                let face = c.faces[fi];
                let n = arity(face);
                let j = face[..n].iter().position(|&u| u == v)?;
                let near = [j, (j + n - 1) % n].map(|k| c.step.edge[fi * 4 + k]);
                if !near.iter().all(|&e| open[(e - old) as usize]) {
                    return None;
                }
                fs.push(fi);
            }
            let mut held = agreed(fs.sorted())?;
            let held = held.as_mut();
            (!held.is_empty()).then(|| rule.vertex(v, &|u| at(held, u)))
        });
        Smooth {
            coarse,
            vertex,
            edge,
            old,
        }
    }

    /// Child `k`'s corner UVs of coarse face `fi`: the linear rule's, the corner's and the edge
    /// points' replaced by the rules' where they smoothed them.
    pub fn child(&self, c: &Coarse, fi: usize, k: usize) -> Corners {
        self.child_of(c.faces, c.step, fi, k)
    }

    /// [`Smooth::child`] over `faces`, the coarse level's canonical faces, and their `step`.
    fn child_of(&self, faces: &[[u32; 4]], step: &Step, fi: usize, k: usize) -> Corners {
        let f = faces[fi];
        let n = arity(f);
        let linear = corner_child(&self.coarse[fi], n, k);
        let pick = |table: &[Option<Uv>], v: u32, from: u32| {
            if v == NO {
                None
            } else {
                table[(v - from) as usize]
            }
        };
        let vertex = f.map(|v| pick(&self.vertex, v, 0));
        let edge = step.edges(fi).map(|e| pick(&self.edge, e, self.old));
        let smoothed = child(k, n, &vertex, &edge, None);
        std::array::from_fn(|p| smoothed[p].map_or(linear[p], |x| x.0))
    }
}

/// The UV columns the base's corner groups name, or a refusal when they name no two groups.
fn uv_defs(base: &Base) -> Result<UvDefs, MeshError> {
    let [u, v] = base.corners.as_slice() else {
        return Err(bad(
            "the levels were made without the two UV corner column groups",
        ));
    };
    let def = |name: &String| AttrDef::new(name, Domain::Face, AttrKind::Scalar);
    Ok(std::array::from_fn(|k| [def(&u[k]), def(&v[k])]))
}

/// Fails unless `store` can take every column of `defs`: each held as it is, or room for it.
fn fits(store: &MeshStore, defs: &UvDefs) -> Result<(), MeshError> {
    let mut count = store.defs.len();
    for def in defs.iter().flatten() {
        count += usize::from(store.attr_fits(def)?.is_none());
    }
    if count > MAX_ATTRS {
        return Err(bad(&format!("more than {MAX_ATTRS} attributes")));
    }
    Ok(())
}

/// Step `j`'s linear rule (decision 0006) over `uvs`, level `j`'s UVs: level 0's faces as
/// `base` has them, every face above a quad.
fn linear(uvs: &[Corners], j: usize, base: &[[u32; 4]]) -> Vec<Corners> {
    let mut next = Vec::with_capacity(uvs.len() * 4);
    let corners = |f: usize| if j == 0 { arity(base[f]) } else { 4 };
    subdivide_corners(uvs, corners, |c| next.push(c));
    next
}

/// Puts `col` (per node) in place as column `def` of `store`, registering it, marked changed
/// on every node.
fn put(store: &mut MeshStore, def: &AttrDef, col: Column) -> Result<(), MeshError> {
    let id = store.add_attr(def.clone())?;
    store.cols.attrs[id.index()] = col;
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    store.mark(&nodes, ColumnSet::attr(id));
    Ok(())
}

impl Levels {
    /// The active level's UVs by canonical face, `None` when it holds no UV columns.
    pub fn active_uvs(&self) -> Option<Vec<Corners>> {
        let store = self.store();
        let groups = groups(&self.base, &store.defs, store)?;
        Some(read(store, self.numbering(), &groups))
    }

    /// Writes `corners`, four `[u, v]` per level-0 face by canonical face (a triangle's corner
    /// 3 is kept, unused), as level 0's UVs: every level above gets what dividing would have
    /// given it (decision 0061 as amended 2026-10-13, step UV5), the UVs smoothed over each
    /// step whose level below keeps its own and linear (decision 0006) over the others. The top
    /// takes the UV columns, each level keeping its own UVs takes its new ones, and the active
    /// level shows them at once; edits pending on it stay pending. Refuses, changing nothing,
    /// a count other than level 0's faces, levels made without the two UV corner groups, a
    /// store that cannot take the UV columns, and a changed topology (as [`Levels::set`] does).
    pub fn write_uvs(&mut self, corners: Vec<Corners>) -> Result<(), MeshError> {
        let own: Vec<bool> = self.all.iter().map(|l| l.uvs.is_some()).collect();
        self.write_uvs_smoothing(corners, &own)
    }

    /// [`Levels::write_uvs`], the UVs smoothed over step `j` (level `j` to `j + 1`) when
    /// `smooth[j]` and the step is smooth: for levels made again (unfreeze) as others were.
    pub fn write_uvs_smoothing(
        &mut self,
        corners: Vec<Corners>,
        smooth: &[bool],
    ) -> Result<(), MeshError> {
        self.check()?;
        let base = self.base.faces.len();
        if corners.len() != base {
            let n = corners.len();
            return Err(bad(&format!("{n} faces of UVs for a base of {base} faces")));
        }
        let defs = uv_defs(&self.base)?;
        fits(self.store(), &defs)?;
        let (top, sets) = self.carried(corners, smooth)?;
        self.align()?;
        self.put_top(&defs, &top)?;
        self.keep(sets);
        self.regather(&defs)
    }

    /// Level 0's `corners` carried `steps` levels up by decision 0006's linear rule, the carry
    /// [`Levels::write_uvs`] runs over each step it does not smooth, for UVs read before their
    /// levels are built (a ZTL file's): level 0's faces as `base` has them.
    pub fn linear_uvs(base: &[[u32; 4]], corners: Vec<Corners>, steps: usize) -> Vec<Corners> {
        (0..steps).fold(corners, |uvs, j| linear(&uvs, j, base))
    }

    /// Level 0's `corners` carried up every step: the top's UVs, and the UVs each level keeps
    /// (those below a step `smooth` smooths).
    fn carried(
        &self,
        corners: Vec<Corners>,
        smooth: &[bool],
    ) -> Result<(Vec<Corners>, Sets), MeshError> {
        let t = self.top_level();
        let smoothing: Vec<bool> = (0..t)
            .map(|j| smooth.get(j) == Some(&true) && self.all[j + 1].smooth)
            .collect();
        let last = smoothing.iter().rposition(|&s| s);
        let mut sets: Sets = vec![None; t + 1];
        let (mut uvs, mut faces) = (corners, self.base.faces.clone());
        for j in 0..t {
            if last.is_none_or(|l| j > l) {
                uvs = linear(&uvs, j, &self.base.faces);
                continue;
            }
            let old = self.all[j].numbering.vertex_count() as u32;
            let step = Step::new(&faces, old).ok_or_else(|| bad("a level is too big"))?;
            uvs = if smoothing[j] {
                let (next, coarse) = self.smoothed(j, &faces, &step, uvs);
                sets[j] = Some(coarse);
                next
            } else {
                linear(&uvs, j, &self.base.faces)
            };
            if Some(j) != last {
                faces = finer(&faces, &step);
            }
        }
        Ok((uvs, sets))
    }

    /// Step `j` over `faces` (level `j`'s, canonical) smoothing `uvs`, level `j`'s UVs, as
    /// dividing does: the level above's UVs, and `uvs` back.
    fn smoothed(
        &self,
        j: usize,
        faces: &[[u32; 4]],
        step: &Step,
        uvs: Vec<Corners>,
    ) -> (Vec<Corners>, Vec<Corners>) {
        let level = &self.all[j];
        let Some(sk) = &level.skeleton else {
            unreachable!("only levels below the top keep UVs");
        };
        let num = &level.numbering;
        let topo = Topo {
            cfg: self.top.cfg,
            nodes: &sk.topo,
            num,
        };
        let rule = Rule {
            topo,
            sharp: &level.sharp,
            smooth: true,
            pin_corners: self.base.pin_corners,
        };
        let (src, fcanon) = (step.sources(num.vertex_count() as u32), num.face_canon());
        let over = Over {
            num,
            faces,
            step,
            src: &src,
            fcanon: &fcanon,
        };
        let s = Smooth::over(&over, &rule, uvs);
        let kids: Vec<[Corners; 4]> = gs_par::map(faces.len(), |fi| {
            let n = arity(faces[fi]);
            std::array::from_fn(|k| (k < n).then(|| s.child_of(faces, step, fi, k)))
                .map(Option::unwrap_or_default)
        });
        let next = (kids.iter().zip(faces))
            .flat_map(|(c, &f)| c[..arity(f)].iter().copied())
            .collect();
        (next, s.coarse)
    }

    /// With a lower level active, registers on the top and on the gathered state every column
    /// the active store registered since it was gathered, as carrying up does, so each column
    /// registered next takes the same place in all three.
    fn align(&mut self) -> Result<(), MeshError> {
        let Some(live) = &mut self.live else {
            return Ok(());
        };
        for store in [&mut self.top, &mut live.gathered] {
            for def in live.store.defs.iter().skip(store.defs.len()) {
                store.add_attr(def.clone())?;
            }
        }
        Ok(())
    }

    /// Writes `uvs`, the top's UVs by canonical face, into its UV columns `defs`.
    fn put_top(&mut self, defs: &UvDefs, uvs: &[Corners]) -> Result<(), MeshError> {
        let num = Arc::clone(&self.all[self.top_level()].numbering);
        let (canon, starts) = (num.face_canon(), num.face_starts());
        for (k, pair) in defs.iter().enumerate() {
            for (g, def) in pair.iter().enumerate() {
                let col = gs_par::map(self.top.node_count(), |n| {
                    let faces = &canon[starts[n] as usize..starts[n + 1] as usize];
                    let values = faces.iter().map(|&f| uvs[f as usize][k][g]).collect();
                    Some(Arc::new(AttrVec::F32(values)))
                });
                put(&mut self.top, def, col)?;
            }
        }
        Ok(())
    }

    /// Gives each level the UVs `sets` keeps for it.
    fn keep(&mut self, sets: Sets) {
        for (level, set) in self.all.iter_mut().zip(sets) {
            if level.uvs.is_some() || set.is_some() {
                let uvs = set.map(Arc::new);
                *level = Arc::new(Level {
                    uvs,
                    ..(**level).clone()
                });
            }
        }
    }

    /// Shows the written UVs on the active level when it is below the top: its UV columns
    /// gathered again, put in its store and in the state it was gathered in alike, so they are
    /// no edit to carry up and edits pending on it stay pending.
    fn regather(&mut self, defs: &UvDefs) -> Result<(), MeshError> {
        if self.live.is_none() {
            return Ok(());
        }
        let fresh = self.gather(self.active).store;
        let Some(live) = &mut self.live else {
            return Ok(());
        };
        for def in defs.iter().flatten() {
            let Some(id) = fresh.attr_id(&def.name) else {
                continue;
            };
            let col = &fresh.cols.attrs[id.index()];
            put(&mut live.store, def, col.clone())?;
            put(&mut live.gathered, def, col.clone())?;
        }
        Ok(())
    }
}
