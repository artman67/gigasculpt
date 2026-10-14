//! Levels as a scene file stores them (decision 0016, section 15). The top level is stored as
//! any store is; beside it a file holds the base (its faces, sharp edges and settings), each
//! lower level's node table and numbering (and its own UVs, when a step smoothed the UVs above
//! it), every level's smooth flag, the active level, and
//! the active level's edits not carried up yet, as its store holds them.
//!
//! [`Levels::stored`] gives that form, sharing the edited columns rather than copying them.
//! [`Levels::from_file`] checks it and rebuilds everything else: each level's canonical faces
//! and step from the base (decision 0006), its numbering, its skeleton's faces, rings, seam and
//! shared lists, its sharp edges, and the live store gathered from the top with the stored
//! edits put back into exactly the nodes they were in, so the reopened stack shares every
//! other column with its gathered state as before and the next switch carries the same bits.

use std::sync::Arc;

use super::{Base, Level, Levels, Live, Skeleton, bad, child_edges};
use crate::build::topology;
use crate::columns::{AttrDef, AttrId, AttrKind, AttrVec, ColumnSet, Domain, NodeTopo};
use crate::config::{MeshError, NodeConfig, NodeId};
use crate::derived::lists_of;
use crate::exact::fits;
use crate::math::{NO, Vec3, arity};
use crate::numbering::Numbering;
use crate::plain::Placement;
use crate::store::MeshStore;
use crate::subdivide::{Step, children, level_vertex_counts};

/// One level below the top as a file stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLevel {
    /// Per node, its vertex slots and its face slots.
    pub nodes: Vec<[u32; 2]>,
    /// Its numbering's arrays (see [`Numbering::placement`]).
    pub numbering: Placement,
    /// Its own UVs by canonical face, corner 0 to 3, each `[u, v]`, when the step above it
    /// smoothed them (see [`super::Levels::divide_uvs`]); else empty.
    pub uvs: Vec<[[f32; 2]; 4]>,
}

/// The active level's edits not carried up: every column of its store that is no longer the
/// very column gathered from the top (decision 0014's pending edits, by allocation).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pending {
    /// The active store's attribute definitions: the top's, then any it added.
    pub defs: Vec<AttrDef>,
    /// The nodes whose positions were edited, ascending, with their positions.
    pub positions: Vec<(NodeId, Arc<Vec<Vec3>>)>,
    /// Per attribute of `defs`, the nodes whose column was edited or added, ascending, with
    /// the column.
    pub columns: Vec<Vec<(NodeId, Arc<AttrVec>)>>,
    /// Per attribute of `defs`, the nodes whose gathered column was dropped, ascending.
    pub dropped: Vec<Vec<NodeId>>,
}

/// A part's detail levels as a scene file stores them beside the top level's store and
/// numbering. See the module docs.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLevels {
    /// The active level, 0 for the base.
    pub active: usize,
    pub pin_corners: bool,
    /// The face corner column groups (see [`super::Settings::corners`]).
    pub corners: Vec<[String; 4]>,
    /// Every level's flag, the base first: made from the level below by Catmull-Clark rather
    /// than a flat step.
    pub smooth: Vec<bool>,
    /// Level 0's faces, canonical.
    pub base_faces: Vec<[u32; 4]>,
    /// Level 0's sharp edges, `(a, b, sharpness)` with `a < b`, sorted, sharpness above 0.
    /// Every level above takes its own from the level below.
    pub creases: Vec<(u32, u32, u8)>,
    /// Every level below the top, the base first.
    pub lower: Vec<StoredLevel>,
    /// The active level's edits, when it is below the top.
    pub pending: Option<Pending>,
}

impl Levels {
    /// The stack as a file stores it beside the top (see the module docs).
    pub fn stored(&self) -> StoredLevels {
        let t = self.top_level();
        let lower = (self.all[..t].iter())
            .map(|l| {
                let topo = l.skeleton.as_ref().map_or(&[][..], |s| &s.topo[..]);
                StoredLevel {
                    nodes: topo
                        .iter()
                        .map(|n| [n.ring.len() as u32, n.faces.len() as u32])
                        .collect(),
                    numbering: l.numbering.placement(),
                    uvs: l.uvs.as_ref().map_or_else(Vec::new, |u| u.to_vec()),
                }
            })
            .collect();
        StoredLevels {
            active: self.active,
            pin_corners: self.base.pin_corners,
            corners: self.base.corners.clone(),
            smooth: self.all.iter().map(|l| l.smooth).collect(),
            base_faces: self.base.faces.clone(),
            creases: self.all[0].sharp.clone(),
            lower,
            pending: self.live.as_ref().map(pending),
        }
    }

    /// The stack a file stores: `top` (the top level's store, already checked as a store) and
    /// its numbering `placed`, with `stored`. Checks everything a stack relies on (decision
    /// 0016, section 15), so nothing read from a file can make a later switch, carry or gather
    /// fail or panic, then rebuilds the rest (see the module docs).
    pub fn from_file(
        top: MeshStore,
        placed: Placement,
        stored: StoredLevels,
    ) -> Result<Levels, MeshError> {
        let StoredLevels {
            active,
            pin_corners,
            corners,
            smooth,
            base_faces,
            creases,
            lower,
            pending,
        } = stored;
        let t = lower.len();
        if t == 0 || smooth.len() != t + 1 || active > t || pending.is_some() != (active < t) {
            return Err(bad(
                "the stored levels disagree on their count or the active level",
            ));
        }
        let cfg = top.cfg;
        check_corners(&corners, &top.defs)?;
        let v0 = u32::try_from(lower[0].numbering.vertices.len())
            .map_err(|_| bad("the base has too many points"))?;
        check_base(&base_faces, v0)?;
        let mut sharp = check_creases(&creases, &base_faces)?;
        let counts = level_vertex_counts(&base_faces, v0, t as u32)
            .ok_or_else(|| bad("a base face uses a point that does not exist"))?;
        let mut faces = base_faces.clone();
        let mut all = Vec::with_capacity(t + 1);
        let mut below = None;
        for (j, mut level) in lower.into_iter().enumerate() {
            let vc = u32::try_from(counts[j]).map_err(|_| bad("a level is too big"))?;
            let step = Step::new(&faces, vc).ok_or_else(|| bad("a level is too big"))?;
            let uvs = std::mem::take(&mut level.uvs);
            if !uvs.is_empty() && (uvs.len() != faces.len() || corners.len() != 2) {
                return Err(bad(
                    "a level's own UVs do not fit its faces or the part's two UV groups",
                ));
            }
            let (numbering, skeleton) = rebuild(cfg, level, &faces, counts[j])?;
            let next = child_edges(&sharp, &faces, &step);
            all.push(Arc::new(Level {
                numbering: Arc::new(numbering),
                skeleton: Some(skeleton),
                smooth: smooth[j],
                sharp: std::mem::replace(&mut sharp, next),
                uvs: (!uvs.is_empty()).then(|| Arc::new(uvs)),
            }));
            if j + 1 < t {
                faces = finer(&faces, &step);
            } else {
                below = Some(step);
            }
        }
        let Some(step) = below else {
            return Err(bad("the stored levels have no level below the top"));
        };
        let numbering = top_numbering(&top, placed, counts[t], &faces, &step)?;
        all.push(Arc::new(Level {
            numbering: Arc::new(numbering),
            skeleton: None,
            smooth: smooth[t],
            sharp,
            uvs: None,
        }));
        let mut levels = Levels {
            base: Arc::new(Base::new(base_faces, pin_corners, corners)),
            all,
            top,
            live: None,
            active: t,
            everything: false,
        };
        if let Some(p) = pending {
            let mut live = levels.gather(active);
            put_back(&mut live.store, &levels.base, p)?;
            levels.live = Some(live);
            levels.active = active;
            super::drop_normals(&mut levels.top);
        }
        Ok(levels)
    }
}

/// The live store's columns that are no longer the gathered ones.
fn pending(live: &Live) -> Pending {
    let (s, g) = (&live.store.cols, &live.gathered.cols);
    let positions = (s.pos.iter().zip(&g.pos).enumerate())
        .filter(|(_, (a, b))| !Arc::ptr_eq(a, b))
        .map(|(n, (a, _))| (n as NodeId, Arc::clone(a)))
        .collect();
    let mut columns = Vec::with_capacity(s.attrs.len());
    let mut dropped = Vec::with_capacity(s.attrs.len());
    for (a, col) in s.attrs.iter().enumerate() {
        let gathered = g.attrs.get(a);
        let (mut edited, mut gone) = (Vec::new(), Vec::new());
        for (n, c) in col.iter().enumerate() {
            match (c, gathered.and_then(|x| x[n].as_ref())) {
                (Some(u), Some(v)) if Arc::ptr_eq(u, v) => {}
                (Some(u), _) => edited.push((n as NodeId, Arc::clone(u))),
                (None, Some(_)) => gone.push(n as NodeId),
                (None, None) => {}
            }
        }
        columns.push(edited);
        dropped.push(gone);
    }
    Pending {
        defs: live.store.defs.clone(),
        positions,
        columns,
        dropped,
    }
}

/// A column named in a corner group must be a face column of scalars, as UVs are: the levels
/// read such columns four at a time per face.
fn check_corners(corners: &[[String; 4]], defs: &[AttrDef]) -> Result<(), MeshError> {
    let fine = |d: &AttrDef| {
        !corners.iter().flatten().any(|c| *c == d.name)
            || (d.domain == Domain::Face && d.kind == AttrKind::Scalar)
    };
    if defs.iter().all(fine) {
        Ok(())
    } else {
        Err(bad(
            "a corner column group names a column that is not face scalars",
        ))
    }
}

/// Every base face has three or four distinct points below `count`.
fn check_base(faces: &[[u32; 4]], count: u32) -> Result<(), MeshError> {
    let sound = |f: &[u32; 4]| {
        let c = &f[..arity(*f)];
        c.iter().all(|&v| v < count) && (1..c.len()).all(|i| !c[..i].contains(&c[i]))
    };
    if faces.iter().all(sound) {
        Ok(())
    } else {
        Err(bad(
            "a base face names a point twice or one that does not exist",
        ))
    }
}

/// The base's sharp edges, checked: normalized as a stack keeps them, each an edge of a face.
fn check_creases(
    creases: &[(u32, u32, u8)],
    faces: &[[u32; 4]],
) -> Result<Vec<(u32, u32, u8)>, MeshError> {
    let mut edges: Vec<(u32, u32)> = (faces.iter())
        .flat_map(|&f| (0..arity(f)).map(move |i| crate::subdivide::ends(f, i)))
        .collect();
    edges.sort_unstable();
    let normal = creases
        .windows(2)
        .all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1));
    let edge = |&(a, b, s): &(u32, u32, u8)| s > 0 && a < b && edges.binary_search(&(a, b)).is_ok();
    if normal && creases.iter().all(edge) {
        Ok(creases.to_vec())
    } else {
        Err(bad(
            "a crease is not an edge of the base, or the list is not sorted",
        ))
    }
}

/// True if `v` holds every index below `n` once.
fn permutation(v: &[u32], n: usize) -> bool {
    let mut seen = vec![false; n];
    v.len() == n
        && (v.iter()).all(|&i| (i as usize) < n && !std::mem::replace(&mut seen[i as usize], true))
}

/// A numbering's arrays fit `vertices` points and `faces` faces, face `c` with `corners(c)`
/// corners: permutations, and a first corner each face has.
fn check_placement(
    p: &Placement,
    vertices: u64,
    faces: usize,
    corners: impl Fn(usize) -> usize,
) -> Result<(), MeshError> {
    let fits = |(c, &k): (usize, &u8)| (k as usize) < corners(c);
    let ok = u64::try_from(p.vertices.len()) == Ok(vertices)
        && permutation(&p.vertices, p.vertices.len())
        && permutation(&p.faces, faces)
        && p.first_corner.len() == faces
        && p.first_corner.iter().enumerate().all(fits);
    if ok {
        Ok(())
    } else {
        Err(bad(
            "a level's numbering is not a numbering of its points and faces",
        ))
    }
}

/// A lower level's numbering and skeleton from its node table, its numbering's arrays and its
/// canonical faces, checked: canonical face `c` sits in the slot the numbering gives it, its
/// corners the store ids of its canonical corners, turned by its first corner.
fn rebuild(
    cfg: NodeConfig,
    level: StoredLevel,
    faces: &[[u32; 4]],
    vertices: u64,
) -> Result<(Numbering, Skeleton), MeshError> {
    let StoredLevel {
        nodes, numbering, ..
    } = level;
    let within = |r: &[u32; 2]| r[0] <= cfg.capacity() && r[1] <= cfg.face_capacity();
    if nodes.len() > cfg.max_nodes() || !nodes.iter().all(within) {
        return Err(bad("a level's node table holds more than nodes can"));
    }
    let sum = |i: usize| nodes.iter().map(|r| u64::from(r[i])).sum::<u64>();
    if sum(0) != vertices || sum(1) != faces.len() as u64 {
        return Err(bad(
            "a level's node table does not hold its points and faces",
        ));
    }
    check_placement(&numbering, vertices, faces.len(), |c| arity(faces[c]))?;
    let num = Numbering::from_table(cfg, &nodes, numbering);
    let fcanon = num.face_canon();
    let fstart = num.face_starts();
    let stored = gs_par::map(nodes.len(), |n| {
        (fstart[n]..fstart[n + 1])
            .map(|d| {
                let c = fcanon[d as usize];
                let (_, first) = num.face(c);
                let f = faces[c as usize];
                let k = arity(f);
                let mut out = [NO; 4];
                for (j, slot) in out.iter_mut().enumerate().take(k) {
                    *slot = num.id(f[(first as usize + j) % k]);
                }
                out
            })
            .collect()
    });
    let slots: Vec<usize> = nodes.iter().map(|r| r[0] as usize).collect();
    let topo: Vec<Arc<NodeTopo>> = topology(cfg, &slots, stored)?;
    let lists = Arc::new(lists_of(cfg, &topo));
    Ok((num, Skeleton { topo, lists }))
}

/// The faces of the level above `faces`, from its step: decision 0006's children, in order.
pub(super) fn finer(faces: &[[u32; 4]], step: &Step) -> Vec<[u32; 4]> {
    let mut out = Vec::with_capacity(faces.len() * 4);
    for (fi, &f) in faces.iter().enumerate() {
        let (kids, n) = children(f, step.edges(fi), step.center[fi]);
        out.extend_from_slice(&kids[..n]);
    }
    out
}

/// The top level's numbering, checked: the top has no dead slots, `placed` numbers its
/// `vertices` points and its faces, and its faces in that numbering are the subdivision of
/// the level below (`faces` with `step`), compared face by face in parallel.
fn top_numbering(
    top: &MeshStore,
    placed: Placement,
    vertices: u64,
    faces: &[[u32; 4]],
    step: &Step,
) -> Result<Numbering, MeshError> {
    let nodes = || (0..top.node_count() as NodeId).map(|n| top.node(n));
    let slots: usize = nodes().map(|n| n.pos.len()).sum();
    let face_slots: usize = nodes().map(|n| n.faces.len()).sum();
    if slots != top.vertex_count() || face_slots != top.face_count() {
        return Err(bad("the top level has dead slots"));
    }
    let corners: usize = faces.iter().map(|&f| arity(f)).sum();
    if face_slots != corners || slots as u64 != vertices {
        return Err(bad(
            "the top level does not have the points and faces its levels give",
        ));
    }
    check_placement(&placed, vertices, corners, |_| 4)?;
    let num = Numbering::from_placement(top, placed);
    let mut start = Vec::with_capacity(faces.len());
    let mut at = 0u32;
    for &f in faces {
        start.push(at);
        at += arity(f) as u32;
    }
    let same = gs_par::reduce(
        faces.len(),
        || true,
        |ok, p| {
            let (kids, n) = children(faces[p], step.edges(p), step.center[p]);
            ok && (0..n).all(|k| num.canonical_face(top, start[p] + k as u32) == kids[k])
        },
        |a, b| a && b,
    );
    if same {
        Ok(num)
    } else {
        Err(bad(
            "the top level's faces do not follow the subdivision rule",
        ))
    }
}

/// Ascending, distinct and below `n`.
fn ascending(nodes: impl IntoIterator<Item = NodeId>, n: usize) -> bool {
    let mut last = None;
    nodes.into_iter().all(|k| {
        let ok = (k as usize) < n && last.is_none_or(|l| l < k);
        last = Some(k);
        ok
    })
}

/// Puts the stored edits `p` into `store`, the active level just gathered: its added
/// definitions, then each edited column into its node, then refreshes what depends on them.
fn put_back(store: &mut MeshStore, base: &Base, p: Pending) -> Result<(), MeshError> {
    let Pending {
        defs,
        positions,
        columns,
        dropped,
    } = p;
    let had = store.defs.len();
    if defs.len() < had || defs[..had] != store.defs[..] {
        return Err(bad(
            "the active level's columns do not start with the top's",
        ));
    }
    if columns.len() != defs.len() || dropped.len() != defs.len() {
        return Err(bad(
            "the active level's edits name columns it does not have",
        ));
    }
    check_corners(&base.corners, &defs)?;
    for d in &defs[had..] {
        if store.attr_id(&d.name).is_some() {
            return Err(bad("the active level names a column twice"));
        }
        store.add_attr(d.clone())?;
    }
    let nn = store.node_count();
    let finite = |q: &Vec3| q.x.is_finite() && q.y.is_finite() && q.z.is_finite();
    let fit = |(n, col): &(NodeId, Arc<Vec<Vec3>>)| {
        store.cols.pos[*n as usize].len() == col.len() && col.iter().all(finite)
    };
    if !ascending(positions.iter().map(|x| x.0), nn) || !positions.iter().all(fit) {
        return Err(bad("an edited position column does not fit its node"));
    }
    let moved: Vec<NodeId> = positions.iter().map(|x| x.0).collect();
    for (n, col) in positions {
        store.cols.pos[n as usize] = col;
    }
    store.after_moves(&moved);
    for (a, (edited, gone)) in columns.into_iter().zip(dropped).enumerate() {
        let def = &store.defs[a];
        // A column's length: the node's vertex slots (one ring each) or its face slots.
        let len = |n: NodeId| {
            let t = &store.cols.topo[n as usize];
            if def.domain == Domain::Face {
                t.faces.len()
            } else {
                t.ring.len()
            }
        };
        let fit = |(n, col): &(NodeId, Arc<AttrVec>)| col.len() == len(*n) && fits(def.kind, col);
        let held = |&n: &NodeId| a < had && store.cols.attrs[a][n as usize].is_some();
        let mut nodes: Vec<NodeId> = edited
            .iter()
            .map(|x| x.0)
            .chain(gone.iter().copied())
            .collect();
        nodes.sort_unstable();
        let sound = ascending(edited.iter().map(|x| x.0), nn)
            && ascending(gone.iter().copied(), nn)
            && ascending(nodes.iter().copied(), nn)
            && edited.iter().all(fit)
            && gone.iter().all(held);
        if !sound {
            return Err(bad(&format!(
                "an edited column of {} does not fit its node",
                def.name
            )));
        }
        let column = &mut store.cols.attrs[a];
        for (n, col) in edited {
            column[n as usize] = Some(col);
        }
        for n in gone {
            column[n as usize] = None;
        }
        let set = ColumnSet::attr(AttrId(a as u8));
        store.mark(&nodes, set);
        store.refresh_state(&nodes, set);
    }
    Ok(())
}
