//! One level below the base (decision 0014, section 12). The coarse level comes from
//! [`super::unsubdivide`]; every level is then renumbered by maps from each new level to the
//! old level it equals, and the stack is rebuilt as a file's is ([`Levels::from_file`]), which
//! checks the result against decision 0006's rule once more. The top store keeps every point
//! and face; only its corner columns (UVs) turn where a face's canonical corners turned, by
//! [`MeshStore::store_corners`].

use std::collections::BTreeMap;

use gs_protocol::{GROUP, PlainMesh};

use super::file::finer;
use super::fill::{face_slot, majority, value};
use super::unsubdivide::{self, Below};
use super::{Levels, StoredLevel, StoredLevels, bad};
use crate::columns::AttrValue;
use crate::config::{MeshError, NodeId};
use crate::plain::Placement;
use crate::store::MeshStore;
use crate::subdivide::Step;
use crate::view::PosRead;

/// From one new level to the old level it equals: each new point's old index, and each new
/// face's old face with the turn of its corners (new corner `i` is old corner `(i + turn) % 4`).
struct Map {
    points: Vec<u32>,
    faces: Vec<(u32, u8)>,
}

impl Map {
    /// The same map one level up, from both levels' steps: a new edge point is the old edge
    /// point of the matching corner, a centre the old centre, and child `k` of a new face the
    /// old face's child `(k + turn) % 4`, turned alike. Every face is a quad from level 0 up.
    fn up(&self, new: &Step, old: &Step) -> Map {
        let had = self.points.len();
        let src = new.sources(had as u32);
        let points = gs_par::map(new.count as usize, |v| {
            if v < had {
                return self.points[v];
            }
            let s = src[v - had] as usize;
            let (g, r) = self.faces[s / 4];
            if new.center[s / 4] == v as u32 {
                old.center[g as usize]
            } else {
                old.edge[g as usize * 4 + (s % 4 + r as usize) % 4]
            }
        });
        let faces = gs_par::map(self.faces.len() * 4, |c| {
            let (g, r) = self.faces[c / 4];
            (g * 4 + (c as u32 % 4 + u32::from(r)) % 4, r)
        });
        Map { points, faces }
    }

    /// `old`, a numbering of the old level, as a numbering of the new one.
    fn renumber(&self, old: &Placement) -> Placement {
        Placement {
            vertices: gs_par::map(self.points.len(), |v| old.vertices[self.points[v] as usize]),
            faces: gs_par::map(self.faces.len(), |f| old.faces[self.faces[f].0 as usize]),
            first_corner: gs_par::map(self.faces.len(), |f| {
                let (g, r) = self.faces[f];
                (old.first_corner[g as usize] + 4 - r) % 4
            }),
        }
    }

    /// An old level's own UVs (by old face) as the new level's, turned as its faces turned.
    fn turned(&self, uvs: &[[[f32; 2]; 4]]) -> Vec<[[f32; 2]; 4]> {
        if uvs.is_empty() {
            return Vec::new();
        }
        gs_par::map(self.faces.len(), |f| {
            let (g, r) = self.faces[f];
            std::array::from_fn(|i| uvs[g as usize][(i + r as usize) % 4])
        })
    }

    /// Where the top's corner columns turn, for [`MeshStore::store_corners`]: each old face's
    /// store slot (from `old`, the top's old numbering) with the turn of the new face made from
    /// it, since new corner `i` holds what old corner `(i + turn) % 4` held.
    fn turns(&self, old: &Placement) -> Placement {
        Placement {
            vertices: Vec::new(),
            faces: (self.faces.iter())
                .map(|&(g, _)| old.faces[g as usize])
                .collect(),
            first_corner: self.faces.iter().map(|&(_, r)| r).collect(),
        }
    }
}

impl Levels {
    /// The stack with one more level below the base (decision 0014, section 12): the old base
    /// becomes level 1 and every level renumbers, the active one included, so the same mesh
    /// stays active. Pending edits are carried up first. Fails, leaving `self` as it was, when
    /// level 0 is not one division of a coarser mesh, a face-group border runs through a coarse
    /// face, or the base has creases.
    pub fn reconstruct(&self) -> Result<Levels, MeshError> {
        let mut l = self.share();
        let active = l.active;
        l.set(l.top_level())?;
        let creased = l.all[0].sharp.len();
        if creased > 0 {
            return Err(bad(&format!(
                "level 0 has {creased} creased edges; reconstruct does not keep creases yet"
            )));
        }
        let t = l.top_level();
        let groups = l.level0_groups();
        let n0 = l.all[0].numbering.vertex_count();
        let mut found = unsubdivide::below(&l.base.faces, n0, groups.as_deref())?;
        let stored = l.stored();
        let coarse = l.coarse_level(&found)?;
        let map = Map {
            points: std::mem::take(&mut found.points),
            faces: std::mem::take(&mut found.faces_up),
        };
        let (mut lower, map) = l.renumbered(&found, map, &stored)?;
        lower.insert(0, coarse);
        let old = l.all[t].numbering.placement();
        let top_placed = map.renumber(&old);
        let mut top = l.top.share();
        top.store_corners(&l.base.corners, &map.turns(&old));
        let mut smooth = vec![true, true];
        smooth.extend_from_slice(&stored.smooth[1..]);
        let rebuilt = StoredLevels {
            active: t + 1,
            pin_corners: stored.pin_corners,
            corners: stored.corners,
            smooth,
            base_faces: found.faces,
            creases: Vec::new(),
            lower,
            pending: None,
        };
        let mut out = Levels::from_file(top, top_placed, rebuilt)?;
        out.everything = self.everything;
        out.set(active + 1)?;
        Ok(out)
    }

    /// The face group each level-0 face shows (the majority of its top descendants, as a
    /// gathered level shows it), if the part has a group column.
    fn level0_groups(&self) -> Option<Vec<u32>> {
        let a = self.top.attr_id(GROUP)?.index();
        let t = self.top_level();
        let num = &self.all[t].numbering;
        let bits = |f: u32| {
            let (n, slot) = face_slot(&self.top, num.face(f).0);
            match value(&self.top, a, n, slot) {
                AttrValue::U32(u) => u,
                AttrValue::F32(x) => x.to_bits(),
            }
        };
        Some(gs_par::map(self.base.faces.len(), |p| {
            majority(self.base.descendants(0, t, p as u32).map(bits).collect())
        }))
    }

    /// The new base as a file stores a lower level: its node table and numbering, from a store
    /// built of its corners where the top holds them.
    fn coarse_level(&self, below: &Below) -> Result<StoredLevel, MeshError> {
        let (pos, num) = (self.top.pos(), &self.all[self.top_level()].numbering);
        let plain = PlainMesh {
            positions: (below.corners.iter())
                .map(|&c| pos.get(num.id(c)).to_array())
                .collect(),
            faces: below.faces.clone(),
            attributes: BTreeMap::new(),
        };
        let (store, placed) = MeshStore::import(plain, self.top.cfg)?;
        let nodes = (0..store.node_count() as NodeId)
            .map(|n| {
                let node = store.node(n);
                [node.pos.len() as u32, node.faces.len() as u32]
            })
            .collect();
        Ok(StoredLevel {
            nodes,
            numbering: placed,
            uvs: Vec::new(),
        })
    }

    /// Every old level below the top renumbered as the new level it equals, and the map from
    /// the new top to the old, starting from `map` (new level 1 to old level 0).
    fn renumbered(
        &self,
        below: &Below,
        mut map: Map,
        stored: &StoredLevels,
    ) -> Result<(Vec<StoredLevel>, Map), MeshError> {
        let too_big = || bad("a level is too big");
        let coarse = Step::new(&below.faces, below.corners.len() as u32).ok_or_else(too_big)?;
        let mut new_faces = finer(&below.faces, &coarse);
        let mut old_faces = self.base.faces.clone();
        let t = self.top_level();
        let mut lower = Vec::with_capacity(t + 1);
        for (j, level) in stored.lower.iter().enumerate() {
            lower.push(StoredLevel {
                nodes: level.nodes.clone(),
                numbering: map.renumber(&level.numbering),
                uvs: map.turned(&level.uvs),
            });
            let new = Step::new(&new_faces, map.points.len() as u32).ok_or_else(too_big)?;
            let count = self.all[j].numbering.vertex_count() as u32;
            let old = Step::new(&old_faces, count).ok_or_else(too_big)?;
            map = map.up(&new, &old);
            if j + 1 < t {
                new_faces = finer(&new_faces, &new);
                old_faces = finer(&old_faces, &old);
            }
        }
        Ok((lower, map))
    }
}
