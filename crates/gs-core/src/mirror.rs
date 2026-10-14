//! The mirror map (decision 0021, section 10): on a part symmetric across a plane through the
//! origin, each point's partner across it, so refinement changes both halves alike and the
//! topology stays exactly symmetric.
//!
//! The map is an optional point column `mirror.x`, `mirror.y` or `mirror.z` of kind `Label`,
//! holding each point's partner id. It is valid when it is an involution and every face's
//! mirror image (its corners' partners, winding reversed) is a face. The points naming
//! themselves are put exactly on the plane when the map is found and when a pass makes them,
//! and a mirrored stroke's plane clamp keeps them there; a stroke without the mirror may move
//! them off, which leaves the topology symmetric, so the map holds. A map is never half true:
//!
//! - the topology editor rewrites partner ids wherever it moves a point ([`TopoEditor`]'s
//!   `mirror_moved` and `mirror_renumbered`);
//! - a refinement pass mirrored across the map's plane splits, collapses and flips in pairs;
//!   every other refinement releases the column ([`MeshStore::clear_mirror`]);
//! - a plain mesh holds partners as point indices, and a store built from one keeps a map only
//!   if it is valid; detail levels never hold one.

use crate::columns::{AttrDef, AttrId, AttrKind, AttrValue, AttrVec, ColumnSet, Domain};
use crate::config::{FaceId, NodeId, VertId};
use crate::math::{Bins, NO, Vec3, arity, same_cycle};
use crate::par::merge;
use crate::store::MeshStore;
use crate::topo::TopoEditor;
use crate::write::NewValues;

const NAMES: [&str; 3] = ["mirror.x", "mirror.y", "mirror.z"];

/// The map column across the plane through the origin square to `axis` (0 = X).
pub fn def(axis: usize) -> AttrDef {
    AttrDef::new(NAMES[axis], Domain::Vertex, AttrKind::Label)
}

/// The axis whose map column `d` is, if it is one.
pub(crate) fn axis_of(d: &AttrDef) -> Option<usize> {
    let axis = NAMES.iter().position(|&n| n == d.name)?;
    (*d == def(axis)).then_some(axis)
}

/// Whether a point attribute named `name` is a map: its values are point ids in a store and
/// point indices in a plain mesh.
pub fn is_map(name: &str) -> bool {
    NAMES.contains(&name)
}

/// `p` mirrored across the plane square to `axis`.
pub fn reflect(p: Vec3, axis: usize) -> Vec3 {
    let mut a = p.to_array();
    a[axis] = -a[axis];
    Vec3::from_array(a)
}

/// `p` moved onto the plane square to `axis`.
pub(crate) fn onto(p: Vec3, axis: usize) -> Vec3 {
    let mut a = p.to_array();
    a[axis] = 0.0;
    Vec3::from_array(a)
}

/// Why a map is not valid or could not be found: the first point or face at fault, in store
/// order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// No point is this point's partner: none lies within the tolerance of its mirror image,
    /// or the one that does pairs with another point.
    NoPartner(VertId),
    /// No face is this face's mirror image with its winding reversed.
    NoMirrorFace(FaceId),
    /// Another column already has the map's name.
    NameTaken,
}

impl MeshStore {
    /// The map columns some node holds, as (axis, attribute index), by axis.
    pub(crate) fn mirror_maps(&self) -> Vec<(usize, usize)> {
        let mut v: Vec<(usize, usize)> = (self.defs.iter().enumerate())
            .filter_map(|(a, d)| Some((axis_of(d)?, a)))
            .filter(|&(_, a)| self.cols.attrs[a].iter().any(Option::is_some))
            .collect();
        v.sort_unstable();
        v
    }

    /// Whether the store holds a mirror map across the plane square to `axis`.
    pub fn has_mirror(&self, axis: usize) -> bool {
        self.mirror_maps().iter().any(|m| m.0 == axis)
    }

    fn live_point(&self, v: VertId) -> bool {
        let (n, l) = (self.cfg.node_of(v) as usize, self.cfg.local_of(v) as usize);
        (self.cols.pos.get(n)).is_some_and(|p| p.get(l).is_some_and(|q| !q.is_dead()))
    }

    /// `v`'s value in map column `a`, `None` when that is no live point.
    fn partner_in(&self, a: usize, v: VertId) -> Option<VertId> {
        let (n, l) = (self.cfg.node_of(v) as usize, self.cfg.local_of(v) as usize);
        let col = self.cols.attrs[a].get(n)?.as_ref();
        let p = col.map_or(0, |c| if l < c.len() { c.bits(l) } else { 0 });
        self.live_point(p).then_some(p)
    }

    /// The face whose corners are `fc`'s partners in reverse order, if there is one.
    fn mirror_face(
        &self,
        fc: [u32; 4],
        partner: impl Fn(VertId) -> Option<VertId>,
    ) -> Option<FaceId> {
        let k = arity(fc);
        let mut want = [NO; 4];
        for i in 0..k {
            want[k - 1 - i] = partner(fc[i])?;
        }
        let cfg = self.cfg;
        let (n, l) = (
            cfg.node_of(want[0]) as usize,
            cfg.local_of(want[0]) as usize,
        );
        (self.cols.topo[n].ring_of(l).iter().copied()).find(|&g| {
            let t = &self.cols.topo[cfg.face_node(g) as usize];
            same_cycle(t.faces[cfg.face_local(g) as usize], want)
        })
    }

    /// The first face of node `n` without a mirror image.
    fn faceless(
        &self,
        n: NodeId,
        partner: &(impl Fn(VertId) -> Option<VertId> + Sync),
    ) -> Option<FaceId> {
        let t = &self.cols.topo[n as usize];
        (t.faces.iter().enumerate())
            .filter(|(_, fc)| fc[0] != NO)
            .find(|(_, fc)| self.mirror_face(**fc, partner).is_none())
            .map(|(j, _)| self.cfg.face(n, j as u32))
    }

    /// Check the map across `axis` (see the module docs); a store without one passes.
    pub fn check_mirror(&self, axis: usize) -> Result<(), Fault> {
        let Some(&(_, a)) = self.mirror_maps().iter().find(|m| m.0 == axis) else {
            return Ok(());
        };
        let cfg = self.cfg;
        let partner = |v: VertId| self.partner_in(a, v);
        let faults = gs_par::map(self.node_count(), |k| {
            let n = k as NodeId;
            for (l, q) in self.cols.pos[k].iter().enumerate() {
                let v = cfg.vert(n, l as u32);
                if !q.is_dead() && partner(v).is_none_or(|w| partner(w) != Some(v)) {
                    return Some(Fault::NoPartner(v));
                }
            }
            self.faceless(n, &partner).map(Fault::NoMirrorFace)
        });
        faults.into_iter().flatten().next().map_or(Ok(()), Err)
    }

    /// Release every map that is not valid: after a rebuild, which carries a map's values but
    /// cannot know whether they still hold.
    pub(crate) fn drop_broken_mirrors(&mut self) {
        for (axis, a) in self.mirror_maps() {
            if self.check_mirror(axis).is_err() {
                self.cols.attrs[a].fill(None);
            }
        }
    }

    /// Find the map across `axis` of a mesh that is already symmetric and store it. Each
    /// point's partner is the point nearest its mirror image within `tolerance`, the lowest id
    /// on a tie; points that are their own partner move exactly onto the plane. Refuses,
    /// changing nothing, naming the first point in store order without a partner or the first
    /// face without a mirror image. Returns the nodes written and how many points are their
    /// own partner.
    pub fn map_mirror(
        &mut self,
        axis: usize,
        tolerance: f32,
    ) -> Result<(Vec<NodeId>, usize), Fault> {
        let cfg = self.cfg;
        let pts: Vec<(VertId, Vec3)> = (0..self.node_count())
            .flat_map(|k| {
                let n = k as NodeId;
                (self.cols.pos[k].iter().enumerate())
                    .filter(|(_, p)| !p.is_dead())
                    .map(move |(l, &p)| (cfg.vert(n, l as u32), p))
            })
            .collect();
        let found = nearest_mirrors(&pts, axis, tolerance);
        for (i, f) in found.iter().enumerate() {
            if f.is_none_or(|j| found[j] != Some(i)) {
                return Err(Fault::NoPartner(pts[i].0));
            }
        }
        let ids: Vec<VertId> = pts.iter().map(|x| x.0).collect();
        let partner = |v: VertId| {
            let i = ids.binary_search(&v).ok()?;
            found[i].map(|j| ids[j])
        };
        let faceless = gs_par::map(self.node_count(), |k| self.faceless(k as NodeId, &partner));
        if let Some(f) = faceless.into_iter().flatten().next() {
            return Err(Fault::NoMirrorFace(f));
        }
        let id = self.add_attr(def(axis)).map_err(|_| Fault::NameTaken)?;
        let mut values: Vec<Vec<u32>> = (self.cols.pos.iter()).map(|p| vec![0; p.len()]).collect();
        let mut snap: Vec<NodeId> = Vec::new();
        for (i, &(v, p)) in pts.iter().enumerate() {
            let (n, l) = (cfg.node_of(v), cfg.local_of(v) as usize);
            let w = found[i].map_or(v, |j| ids[j]);
            values[n as usize][l] = w;
            if w == v && p.to_array()[axis] != 0.0 && snap.last() != Some(&n) {
                snap.push(n);
            }
        }
        self.write(&snap, ColumnSet::POSITION).each(|m| {
            let n = m.id();
            let ps = m.positions();
            for (l, &w) in values[n as usize].iter().enumerate() {
                if w == cfg.vert(n, l as u32) && !ps[l].is_dead() {
                    ps[l] = onto(ps[l], axis);
                }
            }
        });
        let all: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let fresh = (values.into_iter())
            .map(|v| {
                if v.is_empty() {
                    NewValues::Same
                } else {
                    NewValues::Values(AttrVec::U32(v))
                }
            })
            .collect();
        let written = self.put_attr(id, &all, fresh);
        let on_plane = (found.iter().enumerate())
            .filter(|&(i, f)| *f == Some(i))
            .count();
        Ok((merge(&snap, &written), on_plane))
    }

    /// Release the map across `axis`. Returns the nodes written: none without a map.
    pub fn clear_mirror(&mut self, axis: usize) -> Vec<NodeId> {
        let Some(&(_, a)) = self.mirror_maps().iter().find(|m| m.0 == axis) else {
            return Vec::new();
        };
        let held: Vec<NodeId> = (self.cols.attrs[a].iter().enumerate())
            .filter(|(_, c)| c.is_some())
            .map(|(k, _)| k as NodeId)
            .collect();
        let zeros = vec![NewValues::Zeros; held.len()];
        self.put_attr(AttrId(a as u8), &held, zeros)
    }
}

/// For each point of `pts` (ids ascending), the index of the point nearest its mirror image
/// across `axis` within `tol`, the lowest id on a tie. Points are binned in cubes of side
/// `2 tol`, so a ball of radius `tol` meets at most two along each axis; but never under a
/// millionth of the farthest coordinate, so a tolerance of 0 cannot overflow the cube numbers
/// (and put every point in one cube).
fn nearest_mirrors(pts: &[(VertId, Vec3)], axis: usize, tol: f32) -> Vec<Option<usize>> {
    let reach = (pts.iter().flat_map(|p| p.1.to_array())).fold(0.0_f32, |m, x| m.max(x.abs()));
    let side = f64::from((2.0 * tol).max(reach * 1e-6).max(f32::MIN_POSITIVE));
    let key = |p: Vec3| p.to_array().map(|x| (f64::from(x) / side).floor() as i64);
    let bins = Bins::new(pts.len(), |i| key(pts[i].1));
    let r = Vec3::new(tol, tol, tol);
    gs_par::map(pts.len(), |i| {
        let q = reflect(pts[i].1, axis);
        let (lo, hi) = (key(q - r), key(q + r));
        (bins.within(lo, hi))
            .map(|j| ((pts[j as usize].1 - q).len2(), j as usize))
            .filter(|&(d2, _)| d2 <= tol * tol)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|b| b.1)
    })
}

impl TopoEditor<'_> {
    /// `v`'s partner in map column `a`.
    pub(crate) fn partner(&self, a: usize, v: VertId) -> VertId {
        let cfg = self.s.cfg;
        match self.attr(a, cfg.node_of(v) as usize, cfg.local_of(v) as usize) {
            AttrValue::U32(p) => p,
            AttrValue::F32(_) => v,
        }
    }

    pub(crate) fn set_partner(&mut self, a: usize, v: VertId, p: VertId) {
        let cfg = self.s.cfg;
        let (n, l) = (cfg.node_of(v) as usize, cfg.local_of(v) as usize);
        self.set_attr(a, n, l, AttrValue::U32(p));
    }

    /// Point `v` was copied into the slot `w`, partner included: name `w` where `v` was named.
    pub(crate) fn mirror_moved(&mut self, v: VertId, w: VertId) {
        for i in 0..self.maps.len() {
            let a = self.maps[i];
            let p = self.partner(a, v);
            if p == v {
                self.set_partner(a, w, w);
            } else if self.is_live(p) {
                self.set_partner(a, p, w);
            }
        }
    }

    /// Node `n`'s points moved to new slots, `new[old] = new`, partners gathered with them:
    /// rewrite the partners naming them.
    pub(crate) fn mirror_renumbered(&mut self, n: NodeId, new: &[u32]) {
        if self.maps.is_empty() {
            return;
        }
        let cfg = self.s.cfg;
        let here: Vec<VertId> = (0..new.len() as u32)
            .map(|l| cfg.vert(n, l))
            .filter(|&v| self.is_live(v))
            .collect();
        for i in 0..self.maps.len() {
            let a = self.maps[i];
            for &v in &here {
                let p = self.partner(a, v);
                if cfg.node_of(p) == n && (cfg.local_of(p) as usize) < new.len() {
                    self.set_partner(a, v, cfg.vert(n, new[cfg.local_of(p) as usize]));
                }
            }
            for &v in &here {
                let p = self.partner(a, v);
                if cfg.node_of(p) != n && self.is_live(p) {
                    self.set_partner(a, p, v);
                }
            }
        }
    }
}
