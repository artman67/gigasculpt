//! Face groups (decision 0026): the id rule, the colors, and the whole-part edits behind the
//! `group.*` commands.
//!
//! A face's group is the `group` column ([`AttrDef::face_group`], one `u32` per face, 0 for no
//! group). A fresh id is one above the largest id any live face holds, hidden faces included,
//! counted by [`fresh_after`] alone, which refuses to pass [`MAX_GROUP`]. Each node keeps its
//! largest id in its derived state (`crate::visibility`), so a fresh id is one read per node,
//! counted in [`crate::Work::group_reads`].
//!
//! Makers flood over edges, visiting faces in ascending store face id and growing each region
//! first in, first out, and number regions in order of their lowest face, so the same command on
//! the same part gives the same ids every time. The flood fill follows Blender's
//! `init_flood_fill` (`editors/sculpt_paint/mesh/sculpt_face_set.cc`).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::bits::Bits;
use crate::columns::{AttrDef, AttrVec, NodeTopo};
use crate::config::{FaceId, MeshError, NodeId};
use crate::faces::edge_faces;
use crate::mask::Scope;
use crate::math::{NO, Vec3, face_cross};
use crate::rng::CommandRng;
use crate::select::{FaceRule, FaceSet, Selector};
use crate::store::MeshStore;
use crate::stroke::View;
use crate::view::{PosRead, TopoRead, TopoReader};
use crate::visibility::{Cover, Visible};
use crate::write::NewValues;

/// The largest id a face may hold (decision 0026, 2; review 0007, m1): the largest Blender's
/// face sets, a signed 32-bit attribute, hold, so a part's groups can always leave for it. A
/// part takes one id per group made, so it never comes near; files and commands that give a
/// larger id are refused.
pub const MAX_GROUP: u32 = i32::MAX as u32;

/// The first of `count` fresh ids after `largest` (at least one): `largest + 1`, or `None` when
/// the last of them would pass [`MAX_GROUP`]. Every fresh id is counted here, so none can wrap
/// to 0, which is no group.
pub fn fresh_after(largest: u32, count: u32) -> Option<u32> {
    let last = largest.checked_add(count.max(1))?;
    (last <= MAX_GROUP).then_some(largest + 1)
}

/// No fresh id is left for a new group: the part's largest id is [`MAX_GROUP`] or too near it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoFreshId {
    /// The part's largest id.
    pub largest: u32,
    /// How many fresh ids were asked for.
    pub wanted: u32,
}

impl fmt::Display for NoFreshId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let groups = if self.wanted == 1 {
            "a new group".to_string()
        } else {
            format!("{} new groups", self.wanted)
        };
        write!(
            f,
            "no id is left for {groups}: the largest group id is {}, and no id may pass {MAX_GROUP}",
            self.largest
        )
    }
}

impl std::error::Error for NoFreshId {}

/// Refuses a group column holding an id above [`MAX_GROUP`], as a file or command gives it.
pub(crate) fn check_ids(ids: &[u32]) -> Result<(), MeshError> {
    match ids.iter().position(|&g| g > MAX_GROUP) {
        Some(i) => Err(MeshError::Input(format!(
            "face {i} has group id {}, above the largest a face may hold, {MAX_GROUP}",
            ids[i]
        ))),
        None => Ok(()),
    }
}

/// The largest group id among a node's live faces (0 when none), from its topology and its
/// group column.
pub(crate) fn largest_in(topo: &NodeTopo, col: Option<&AttrVec>) -> u32 {
    let Some(AttrVec::U32(ids)) = col else {
        return 0;
    };
    (topo.faces.iter().zip(ids))
        .filter(|(f, _)| f[0] != NO)
        .map(|(_, &g)| g)
        .max()
        .unwrap_or(0)
}

/// A group's color as RGBA bytes, from its id and a color seed alone (decision 0026, 5):
/// Blender's face set color (`BKE_paint_face_set_overlay_color_get`,
/// `blenkernel/intern/paint.cc`, GPL-2.0-or-later), so a group looks the same in both programs
/// (decision 0032, 6). A golden-ratio hue per id, saturation and value from Blender's integer
/// hash, then its `hsv_to_rgb` and byte rounding, in `f32` with no fused multiply-add, as
/// Blender's x86-64 build works them out; no platform functions, so every machine gets the
/// same bytes. Group 0 is white.
pub fn group_color(id: u32, seed: u32) -> [u8; 4] {
    if id == 0 {
        return [255; 4];
    }
    // Blender's 0.618033988749895, as the nearest `f32` holds it.
    let hue = 0.618_034_f32 * id.wrapping_add(seed % 10) as f32;
    let hue = hue - hue.floor();
    // `BLI_hash_int_01`: the hash over 2^32 - 1, which is 2^32 in `f32`.
    let unit = |k: u32| blender_hash(k) as f32 * (1.0 / 4_294_967_295.0_f32);
    let key = id.wrapping_add(seed);
    let s = 0.6 + unit(key.wrapping_add(1)) * 0.25;
    let v = 1.0 - unit(key.wrapping_add(2)) * 0.35;
    let six = hue * 6.0;
    let channel = |n: f32| ((n.clamp(0.0, 1.0) - 1.0) * s + 1.0) * v;
    let rgb = [
        channel((six - 3.0).abs() - 1.0),
        channel(2.0 - (six - 2.0).abs()),
        channel(2.0 - (six - 4.0).abs()),
    ];
    let byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), 255]
}

/// Blender's `BLI_hash_int(k)`: Bob Jenkins' `final` mix of `BLI_hash_int_2d(k, 0)`.
fn blender_hash(k: u32) -> u32 {
    let start = 0xdead_beef_u32.wrapping_add((2 << 2) + 13);
    let (mut a, mut b, mut c) = (start.wrapping_add(k), start, start);
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(14));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(11));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(25));
    c ^= b;
    c = c.wrapping_sub(b.rotate_left(16));
    a ^= c;
    a = a.wrapping_sub(c.rotate_left(4));
    b ^= a;
    b = b.wrapping_sub(a.rotate_left(14));
    c ^= b;
    c.wrapping_sub(b.rotate_left(24))
}

/// How `group.make` cuts the part into groups (decision 0026, 3). Hidden faces never seed or
/// join a region and keep their ids.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupMaker {
    /// One group per edge-connected piece.
    Shells,
    /// Pieces split wherever neighbouring faces' normals differ by more than `max_angle`
    /// degrees.
    Normals { max_angle: f32 },
    /// The visible faces, or with `coverage` below 1 a patchy part of them: each face draws a
    /// number from the stream seeded with `seed`, `round(16 * cluster)` passes smooth the
    /// numbers over neighbours, and the lowest `round(coverage * count)` form the group.
    Visible {
        coverage: f32,
        cluster: f32,
        seed: u64,
    },
    /// The visible masked faces (every visible face when nothing is masked); `clear_mask` then
    /// clears the mask.
    Masked { clear_mask: bool },
    /// The visible faces turned toward `view` within `angle` degrees; `additive` joins them to
    /// the part's largest id instead of a fresh one.
    Front {
        view: View,
        angle: f32,
        additive: bool,
    },
}

/// What [`MeshStore::make_groups`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Made {
    /// The nodes written.
    pub nodes: Vec<NodeId>,
    /// The first and last id the maker gave faces (it gave every id between them too), or
    /// `None` when no face changed group.
    pub ids: Option<[u32; 2]>,
}

impl Made {
    /// Ids `first` to `last` given, if `nodes` were written.
    fn of(nodes: Vec<NodeId>, first: u32, last: u32) -> Made {
        let ids = (!nodes.is_empty()).then_some([first, last]);
        Made { nodes, ids }
    }
}

/// One group's faces, as `group.list` reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCount {
    pub id: u32,
    pub visible: u64,
    pub hidden: u64,
}

/// Whether live face `f` is visible.
fn open(visible: Visible<'_>, t: &TopoReader<'_>, f: FaceId) -> bool {
    t.face(f)[0] != NO && visible.face(f)
}

/// A node's group ids as a [`MeshStore::put_attr`] write: released when they are all zero,
/// unchanged, or new values.
fn written(old: Option<&[u32]>, new: Vec<u32>) -> NewValues {
    if new.iter().all(|&g| g == 0) {
        return if old.is_some() {
            NewValues::Zeros
        } else {
            NewValues::Same
        };
    }
    match old {
        Some(o) if o == new.as_slice() => NewValues::Same,
        _ => NewValues::Values(AttrVec::U32(new)),
    }
}

impl MeshStore {
    /// The largest group id any live face holds, hidden faces included (0 when none): one read
    /// of each node's summary.
    pub fn largest_group(&self) -> u32 {
        self.der.vis.groups.iter().copied().max().unwrap_or(0)
    }

    /// A fresh group id: one above [`MeshStore::largest_group`], by [`fresh_after`]. Counts the
    /// summaries read in [`crate::Work::group_reads`].
    pub fn fresh_group(&mut self) -> Result<u32, NoFreshId> {
        self.fresh_groups(1)
    }

    /// The first of `count` fresh group ids, as [`MeshStore::fresh_group`].
    fn fresh_groups(&mut self, count: u32) -> Result<u32, NoFreshId> {
        let largest = self.read_largest();
        fresh_after(largest, count).ok_or(NoFreshId {
            largest,
            wanted: count,
        })
    }

    /// [`MeshStore::largest_group`], counting the summaries read.
    fn read_largest(&mut self) -> u32 {
        self.work.group_reads += self.der.vis.groups.len() as u64;
        self.largest_group()
    }

    /// Every group id held by a live face, ascending, with its visible and hidden face counts.
    pub fn group_counts(&self) -> Vec<GroupCount> {
        let g = self.attr_id(&AttrDef::face_group().name);
        let visible = self.visible();
        let per_node = gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            let ids = match g.and_then(|g| node.attr(g)) {
                Some(AttrVec::U32(v)) => Some(&v[..]),
                _ => None,
            };
            let mut counts = BTreeMap::new();
            for (j, fc) in node.faces.iter().enumerate() {
                if fc[0] == NO {
                    continue;
                }
                let id = ids.map_or(0, |v| v[j]);
                let c: &mut (u64, u64) = counts.entry(id).or_default();
                if visible.face(node.first_face + j as u32) {
                    c.0 += 1;
                } else {
                    c.1 += 1;
                }
            }
            counts
        });
        let mut all: BTreeMap<u32, (u64, u64)> = BTreeMap::new();
        for counts in per_node {
            for (id, (v, h)) in counts {
                let c = all.entry(id).or_default();
                c.0 += v;
                c.1 += h;
            }
        }
        (all.into_iter())
            .map(|(id, (visible, hidden))| GroupCount {
                id,
                visible,
                hidden,
            })
            .collect()
    }

    /// Give the faces of `set` group `id`. Exactly those faces change. Returns the nodes
    /// written.
    pub fn assign_group(&mut self, set: &FaceSet, id: u32) -> Vec<NodeId> {
        self.relabel(|n, j, old| {
            if set.node(n).is_some_and(|b| b.get(j)) {
                id
            } else {
                old
            }
        })
    }

    /// Move every face of the groups in `sources`, hidden faces included, to group `target`.
    /// Returns the nodes written.
    pub fn merge_groups(&mut self, sources: &[u32], target: u32) -> Vec<NodeId> {
        self.relabel(|_, _, old| if sources.contains(&old) { target } else { old })
    }

    /// Make groups with `maker` (decision 0026, 3). Fails, writing nothing, when the groups it
    /// would make have no fresh ids left.
    pub fn make_groups(&mut self, maker: GroupMaker) -> Result<Made, NoFreshId> {
        match maker {
            GroupMaker::Shells => self.flood_groups(|_, _| true),
            GroupMaker::Normals { max_angle } => {
                let min_cos = libm::cos(f64::from(max_angle).to_radians()) as f32;
                let normals = self.unit_normals();
                let cfg = self.cfg;
                let n = |f: FaceId| normals[cfg.face_node(f) as usize][cfg.face_local(f) as usize];
                self.flood_groups(|f, g| n(f).dot(n(g)) >= min_cos)
            }
            GroupMaker::Visible {
                coverage,
                cluster,
                seed,
            } => {
                let set = if coverage >= 1.0 {
                    self.select(&Selector::Visible)
                } else {
                    self.patches(coverage, cluster, seed)
                };
                self.group_of(&set, false)
            }
            GroupMaker::Masked { clear_mask } => {
                let none = (0..self.node_count() as NodeId)
                    .all(|n| self.node_state(n).mask == Cover::None);
                let set = if none {
                    self.select(&Selector::Visible)
                } else {
                    let masked = self.select(&Selector::Masked {
                        threshold: 0.5,
                        rule: FaceRule::Mean,
                    });
                    masked.and(&self.select(&Selector::Visible))
                };
                let mut made = self.group_of(&set, false)?;
                if clear_mask {
                    let cleared = self.mask_set(0.0, Scope::All);
                    made.nodes = crate::par::merge(&made.nodes, &cleared);
                }
                Ok(made)
            }
            GroupMaker::Front {
                view,
                angle,
                additive,
            } => {
                let facing = self.select(&Selector::Facing { view, angle });
                let set = facing.and(&self.select(&Selector::Visible));
                self.group_of(&set, additive)
            }
        }
    }

    /// Give `set` one fresh id, or with `join` the part's largest id (a fresh one if there is
    /// no group yet). Writes nothing for an empty set.
    fn group_of(&mut self, set: &FaceSet, join: bool) -> Result<Made, NoFreshId> {
        if set.count() == 0 {
            return Ok(Made::default());
        }
        let id = match self.read_largest() {
            largest if join && largest > 0 => largest,
            largest => fresh_after(largest, 1).ok_or(NoFreshId { largest, wanted: 1 })?,
        };
        Ok(Made::of(self.assign_group(set, id), id, id))
    }

    /// Each live face's unit normal, per node and slot (zero for a dead or degenerate face).
    fn unit_normals(&self) -> Vec<Vec<Vec3>> {
        let pos = self.pos();
        gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            (node.faces.iter())
                .map(|&fc| {
                    if fc[0] == NO {
                        Vec3::ZERO
                    } else {
                        face_cross(fc, |v| pos.get(v)).normalized()
                    }
                })
                .collect()
        })
    }

    /// Flood the visible faces into regions across edges where `join(f, g)` holds, and number
    /// them from a fresh id in order of their lowest face. Fails before writing when the regions
    /// would pass [`MAX_GROUP`].
    fn flood_groups(&mut self, join: impl Fn(FaceId, FaceId) -> bool) -> Result<Made, NoFreshId> {
        let (regions, count) = {
            let (t, visible, cfg) = (self.topo(), self.visible(), self.cfg);
            let mut region: Vec<Vec<u32>> = (0..self.node_count())
                .map(|k| vec![0; self.node(k as NodeId).faces.len()])
                .collect();
            let at = |f: FaceId| (cfg.face_node(f) as usize, cfg.face_local(f) as usize);
            let (mut queue, mut nb, mut next) = (VecDeque::new(), Vec::new(), 0);
            for k in 0..self.node_count() {
                let node = self.node(k as NodeId);
                for j in 0..node.faces.len() {
                    let seed = node.first_face + j as u32;
                    if region[k][j] != 0 || !open(visible, &t, seed) {
                        continue;
                    }
                    next += 1;
                    region[k][j] = next;
                    queue.push_back(seed);
                    while let Some(f) = queue.pop_front() {
                        edge_faces(&t, |g| open(visible, &t, g), f, &mut nb);
                        for &g in &nb {
                            let (n, l) = at(g);
                            if region[n][l] == 0 && join(f, g) {
                                region[n][l] = next;
                                queue.push_back(g);
                            }
                        }
                    }
                }
            }
            (region, next)
        };
        if count == 0 {
            return Ok(Made::default());
        }
        let base = self.fresh_groups(count)? - 1;
        let nodes = self.relabel(|n, j, old| match regions[n as usize][j] {
            0 => old,
            r => base + r,
        });
        Ok(Made::of(nodes, base + 1, base + count))
    }

    /// Group Visible's patchy subset (decision 0026, 3): random numbers smoothed over visible
    /// neighbours, the lowest `round(coverage * count)` taken, ties by face id.
    fn patches(&self, coverage: f32, cluster: f32, seed: u64) -> FaceSet {
        let (t, visible, cfg) = (self.topo(), self.visible(), self.cfg);
        let rng = CommandRng::from_seed(seed);
        let first = |k: usize| self.node(k as NodeId).first_face;
        let mut value: Vec<Vec<f64>> = gs_par::map(self.node_count(), |k| {
            let node = self.node(k as NodeId);
            (0..node.faces.len())
                .map(|j| {
                    let f = node.first_face + j as u32;
                    (rng.u64_at(u64::from(f)) >> 40) as f64 / 16_777_216.0
                })
                .collect()
        });
        let passes = (16.0 * cluster.clamp(0.0, 1.0)).round() as usize;
        for _ in 0..passes {
            let old = &value;
            value = gs_par::map(self.node_count(), |k| {
                let mut nb = Vec::new();
                (0..old[k].len())
                    .map(|j| {
                        let f = first(k) + j as u32;
                        if !open(visible, &t, f) {
                            return old[k][j];
                        }
                        edge_faces(&t, |g| open(visible, &t, g), f, &mut nb);
                        let at =
                            |g: FaceId| old[cfg.face_node(g) as usize][cfg.face_local(g) as usize];
                        let sum = nb.iter().fold(old[k][j], |s, &g| s + at(g));
                        sum / (nb.len() + 1) as f64
                    })
                    .collect()
            });
        }
        let mut order: Vec<(f64, FaceId)> = (0..self.node_count())
            .flat_map(|k| (0..value[k].len()).map(move |j| (k, j)))
            .map(|(k, j)| (value[k][j], first(k) + j as u32))
            .filter(|&(_, f)| open(visible, &t, f))
            .collect();
        let take = (f64::from(coverage.clamp(0.0, 1.0)) * order.len() as f64).round() as usize;
        order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut picked: Vec<Bits> = (0..self.node_count())
            .map(|k| Bits::zeros(self.node(k as NodeId).faces.len()))
            .collect();
        for &(_, f) in &order[..take] {
            picked[cfg.face_node(f) as usize].set(cfg.face_local(f) as usize, true);
        }
        FaceSet::from_nodes(picked)
    }

    /// Rewrite every live face's group to `new_of(node, slot, old)`, as one write. A part with no
    /// group column gets one with its first non-zero id ([`MeshStore::put_column`]). Returns the
    /// nodes written.
    fn relabel(&mut self, new_of: impl Fn(NodeId, usize, u32) -> u32 + Sync) -> Vec<NodeId> {
        let g = self.attr_fits(&AttrDef::face_group()).ok().flatten();
        let nodes: Vec<NodeId> = (0..self.node_count() as NodeId).collect();
        let fresh = gs_par::map(nodes.len(), |i| {
            let node = self.node(nodes[i]);
            let old = match g.and_then(|g| node.attr(g)) {
                Some(AttrVec::U32(v)) => Some(&v[..]),
                _ => None,
            };
            let new = (node.faces.iter().enumerate())
                .map(|(j, fc)| {
                    let o = old.map_or(0, |v| v[j]);
                    if fc[0] == NO {
                        o
                    } else {
                        new_of(nodes[i], j, o)
                    }
                })
                .collect();
            written(old, new)
        });
        self.put_column(AttrDef::face_group(), &nodes, fresh)
    }
}
