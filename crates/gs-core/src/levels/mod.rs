//! Detail levels: the top is the truth (decision 0014).
//!
//! A [`Levels`] stack holds the top level as an ordinary [`MeshStore`], every level's
//! numbering (decision 0006's rule, so a lower level's points are the top's first points) and,
//! below the top, its node skeleton: the topology columns and seam lists, immutable and shared
//! by reference count. Only the active level below the top lives as a store, the live store,
//! gathered from the top onto its skeleton. Edits made there are carried up to the top at the
//! next switch ([`carry`]). Levels are built by Catmull-Clark steps ([`rules`]) cut node by node
//! from the level below, without a global sort ([`cut`]); a step smooths the UVs too when asked
//! ([`uvs`]), and the level it was made from keeps the UVs it had.

mod carry;
mod cut;
mod file;
mod fill;
mod load;
pub mod project;
mod reconstruct;
mod region;
mod rules;
mod unsubdivide;
mod uvs;

use std::ops::Range;
use std::sync::Arc;

use crate::columns::{ColumnSet, Columns, NodeTopo};
use crate::config::{MeshError, NodeId};
use crate::history::Allocation;
use crate::math::{NO, Vec3, arity};
use crate::numbering::Numbering;
use crate::store::{Lists, MeshStore};
use crate::subdivide::{Step, corner_place, ends};
use crate::view::PosRead;

pub use carry::Carried;
pub use file::{Pending, StoredLevel, StoredLevels};
use rules::{Rule, Sharp, Topo, child_sharpness, key};
use uvs::Corners;

/// How a part's levels are made, chosen when the first level is added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Crease sharpness of base edges, `(a, b, sharpness)` with `a` and `b` the edge's points
    /// in the base's canonical numbering: 1 to 254 levels, 255 always sharp, 0 smooth.
    pub creases: Vec<(u32, u32, u8)>,
    /// Boundary corners (points with one face) stay put, instead of following the boundary
    /// curve.
    pub pin_corners: bool,
    /// Face columns that hold one value per face corner, in groups of four column names,
    /// corner 0 to corner 3 (UVs: `uv0.u` .. `uv3.u`, and the `v` group).
    pub corners: Vec<[String; 4]>,
}

/// What every level is built on: the base faces and the settings.
pub(crate) struct Base {
    /// Level-0 faces, canonical.
    pub faces: Vec<[u32; 4]>,
    /// Where each base face's children start among level 1's faces (one more at the end).
    pub cstart: Vec<u32>,
    pub pin_corners: bool,
    pub corners: Vec<[String; 4]>,
}

impl Base {
    fn new(faces: Vec<[u32; 4]>, pin_corners: bool, corners: Vec<[String; 4]>) -> Base {
        let mut cstart = Vec::with_capacity(faces.len() + 1);
        let mut at = 0u32;
        for &f in &faces {
            cstart.push(at);
            at += arity(f) as u32;
        }
        cstart.push(at);
        Base {
            faces,
            cstart,
            pin_corners,
            corners,
        }
    }

    pub fn is_corner_column(&self, name: &str) -> bool {
        self.corners.iter().any(|g| g.iter().any(|c| c == name))
    }

    /// The faces of level `t` descending from face `p` of level `j`, by canonical index.
    pub fn descendants(&self, j: usize, t: usize, p: u32) -> Range<u32> {
        if j == t {
            return p..p + 1;
        }
        if j == 0 {
            let s = 4u32.pow((t - 1) as u32);
            return self.cstart[p as usize] * s..self.cstart[p as usize + 1] * s;
        }
        let s = 4u32.pow((t - j) as u32);
        p * s..(p + 1) * s
    }

    /// The face of level `t` holding corner `k` of face `p` of level `j`, and the corner's
    /// place in it: each step keeps a corner in child `k`, at place `k` (or 3 for a base
    /// triangle's last corner, whose unused fourth corner reads as that one).
    pub fn corner_descendant(&self, j: usize, t: usize, p: u32, k: usize) -> (u32, usize) {
        if j == t {
            return (p, k);
        }
        let (mut at, place, from) = if j == 0 {
            // A triangle's unused fourth corner reads as its last.
            let n = arity(self.faces[p as usize]);
            let k = k.min(n - 1);
            (self.cstart[p as usize] + k as u32, corner_place(k, n), 1)
        } else {
            (p * 4 + k as u32, k, j + 1)
        };
        for _ in from..t {
            at = at * 4 + place as u32;
        }
        (at, place)
    }
}

/// A level's node topology, kept below the top so a switch never sorts or builds rings.
#[derive(Clone)]
pub(crate) struct Skeleton {
    pub topo: Vec<Arc<NodeTopo>>,
    /// Its nodes' seam and shared lists, shared by every copy of the level, so a level made
    /// again (a UV write, deleting the levels above) copies none.
    pub lists: Arc<Lists>,
}

impl Skeleton {
    fn of(store: &MeshStore) -> Skeleton {
        Skeleton {
            topo: store.cols.topo.clone(),
            lists: Arc::new(store.lists()),
        }
    }
}

/// One level, immutable once made.
#[derive(Clone)]
pub(crate) struct Level {
    pub numbering: Arc<Numbering>,
    /// `None` for the top, whose store holds its topology.
    pub skeleton: Option<Skeleton>,
    /// Made from the level below by Catmull-Clark (else flat). Unused for level 0.
    pub smooth: bool,
    /// Its sharp edges, sorted.
    pub sharp: Vec<Sharp>,
    /// Its UVs by canonical face, when the step above it smoothed them and so no level above
    /// holds them: what it shows, and what the levels below it read through it.
    pub uvs: Option<Arc<Vec<Corners>>>,
}

/// The active level's store while it is below the top, and its state when gathered.
struct Live {
    store: MeshStore,
    gathered: MeshStore,
}

/// One level's sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LevelInfo {
    pub vertices: usize,
    pub faces: usize,
    /// Made from the level below by Catmull-Clark rather than a flat step.
    pub smooth: bool,
    /// Keeps its own UVs: the step above it smoothed them, so no level above holds them.
    pub own_uvs: bool,
}

/// A part's detail levels. See the module docs.
pub struct Levels {
    base: Arc<Base>,
    /// Every level, the base first.
    all: Vec<Arc<Level>>,
    top: MeshStore,
    live: Option<Live>,
    active: usize,
    /// Carry over every point of each level instead of the marked region: the oracle the
    /// region is checked against (review 0004, T1). Only tests set it.
    everything: bool,
}

/// True if both stores hold the very same position and attribute columns: no edit was made on
/// one since it was the other (normals follow positions, and may be computed apart).
fn same_edits(a: &MeshStore, b: &MeshStore) -> bool {
    let (x, y) = (&a.cols, &b.cols);
    let attrs = (x.attrs.iter().zip(&y.attrs)).all(|(p, q)| {
        p.iter().zip(q).all(|pair| match pair {
            (Some(u), Some(v)) => Arc::ptr_eq(u, v),
            (u, v) => u.is_none() && v.is_none(),
        })
    });
    x.pos.len() == y.pos.len()
        && x.attrs.len() == y.attrs.len()
        && x.pos.iter().zip(&y.pos).all(|(p, q)| Arc::ptr_eq(p, q))
        && attrs
}

fn bad(s: &str) -> MeshError {
    MeshError::Input(s.to_string())
}

/// Drops the top's normals while a lower level is active (decision 0014, section 13): nothing
/// reads them until the top is active again, and they are a fifth of the top's memory.
/// Carrying up still writes the top's positions; the store skips normals it does not hold.
fn drop_normals(top: &mut MeshStore) {
    let none = Arc::new(Vec::new());
    top.cols.nrm.iter_mut().for_each(|c| *c = Arc::clone(&none));
}

/// Computes every normal of the top again if [`drop_normals`] dropped them, the same bits a
/// top that kept them holds.
fn restore_normals(top: &mut MeshStore) {
    let pos = &top.cols.pos;
    if (top.cols.nrm.iter().zip(pos)).all(|(n, p)| n.len() == p.len()) {
        return;
    }
    top.cols.nrm = gs_par::map(pos.len(), |k| Arc::new(vec![Vec3::ZERO; pos[k].len()]));
    let all: Vec<NodeId> = (0..top.node_count() as NodeId).collect();
    top.normals(&all, &[]);
    top.mark(&all, ColumnSet::NORMAL);
}

/// Sharp edges normalized: `a < b`, sorted, sharpness 0 dropped, the highest kept for an edge
/// listed twice. Every edge must be an edge of `store` under `num`.
fn sharp_edges(
    store: &MeshStore,
    num: &Numbering,
    creases: &[(u32, u32, u8)],
) -> Result<Vec<Sharp>, MeshError> {
    let topo = Topo {
        cfg: store.cfg,
        nodes: &store.cols.topo,
        num,
    };
    let mut out: Vec<Sharp> = Vec::new();
    for &(a, b, s) in creases {
        let n = num.vertex_count() as u32;
        if a >= n || b >= n || a == b {
            return Err(MeshError::Input(format!(
                "crease {a}-{b} names a point that does not exist"
            )));
        }
        let edge = topo.ring(a).iter().any(|&f| {
            let fc = topo.face(f);
            let k = arity(fc);
            (0..k).any(|i| {
                (fc[i] == a && fc[(i + 1) % k] == b) || (fc[i] == b && fc[(i + 1) % k] == a)
            })
        });
        if !edge {
            return Err(MeshError::Input(format!("crease {a}-{b} is not an edge")));
        }
        if s > 0 {
            out.push((a.min(b), a.max(b), s));
        }
    }
    out.sort_unstable_by(|x, y| (x.0, x.1, y.2).cmp(&(y.0, y.1, x.2)));
    out.dedup_by_key(|x| (x.0, x.1));
    Ok(out)
}

impl Levels {
    /// One level, `store`, numbered by `numbering` (which must cover every point and face of a
    /// store without dead slots), with `settings` for the levels to come.
    pub fn new(
        store: MeshStore,
        numbering: Arc<Numbering>,
        settings: &Settings,
    ) -> Result<Levels, MeshError> {
        if numbering.vertex_count() != store.vertex_count()
            || numbering.face_count() != store.face_count()
        {
            return Err(bad("the numbering does not cover the mesh"));
        }
        let slots: usize = (0..store.node_count() as u32)
            .map(|n| store.node(n).pos.len())
            .sum();
        if slots != store.vertex_count() {
            return Err(bad("the mesh has dead slots; re-sort it first"));
        }
        let sharp = sharp_edges(&store, &numbering, &settings.creases)?;
        let base = Base::new(
            numbering.canonical_faces(&store),
            settings.pin_corners,
            settings.corners.clone(),
        );
        Ok(Levels {
            base: Arc::new(base),
            all: vec![Arc::new(Level {
                numbering,
                skeleton: None,
                smooth: true,
                sharp,
                uvs: None,
            })],
            top: store,
            live: None,
            active: 0,
            everything: false,
        })
    }

    /// A copy sharing every column, skeleton and numbering (decision 0010), with no history.
    #[must_use]
    pub fn share(&self) -> Levels {
        Levels {
            base: Arc::clone(&self.base),
            all: self.all.clone(),
            top: self.top.share(),
            live: self.live.as_ref().map(|l| Live {
                store: l.store.share(),
                gathered: l.gathered.share(),
            }),
            active: self.active,
            everything: self.everything,
        }
    }

    /// Carries over every point of each level instead of the marked region, when `on`: slow,
    /// and the same bits if the region is right. For tests only (review 0004, T1).
    #[doc(hidden)]
    pub fn carry_everything(&mut self, on: bool) {
        self.everything = on;
    }

    /// True if both hold the very same columns, levels and base: a share neither side changed.
    pub fn same(&self, other: &Levels) -> bool {
        self.same_besides_active(other) && self.store().same_columns(other.store())
    }

    /// True if both are the same stack apart from the active store's columns: the base, every
    /// level, the active level, and the inactive top and gathered state. A job that edited the
    /// active store of a share in place lands only on a stack this holds for (decision 0010).
    pub fn same_besides_active(&self, other: &Levels) -> bool {
        let inactive = match (&self.live, &other.live) {
            (Some(a), Some(b)) => {
                a.gathered.same_columns(&b.gathered) && self.top.same_columns(&other.top)
            }
            (a, b) => a.is_none() && b.is_none(),
        };
        Arc::ptr_eq(&self.base, &other.base)
            && self.all.len() == other.all.len()
            && (self.all.iter().zip(&other.all)).all(|(a, b)| Arc::ptr_eq(a, b))
            && self.active == other.active
            && inactive
    }

    /// How many levels, the base included.
    pub fn count(&self) -> usize {
        self.all.len()
    }
    /// The active level, 0 for the base.
    pub fn active(&self) -> usize {
        self.active
    }
    pub fn top_level(&self) -> usize {
        self.all.len() - 1
    }
    /// The active level's store: the live store, or the top.
    pub fn store(&self) -> &MeshStore {
        self.live.as_ref().map_or(&self.top, |l| &l.store)
    }
    pub fn store_mut(&mut self) -> &mut MeshStore {
        match &mut self.live {
            Some(l) => &mut l.store,
            None => &mut self.top,
        }
    }
    /// The top level's store, the truth for every level.
    pub fn top(&self) -> &MeshStore {
        &self.top
    }
    /// The active level's numbering.
    pub fn numbering(&self) -> &Arc<Numbering> {
        &self.all[self.active].numbering
    }
    /// Level `j`'s numbering.
    pub fn level_numbering(&self, j: usize) -> &Arc<Numbering> {
        &self.all[j].numbering
    }
    /// The base's creases, as [`Settings::creases`] gave them, normalized: `a < b`, sorted,
    /// none at sharpness 0.
    pub fn base_creases(&self) -> &[(u32, u32, u8)] {
        &self.all[0].sharp
    }
    /// Every level's sizes, the base first.
    pub fn info(&self) -> Vec<LevelInfo> {
        (self.all.iter())
            .map(|l| LevelInfo {
                vertices: l.numbering.vertex_count(),
                faces: l.numbering.face_count(),
                smooth: l.smooth,
                own_uvs: l.uvs.is_some(),
            })
            .collect()
    }
    /// True if the live store holds edits not carried up yet.
    pub fn pending(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| !same_edits(&l.store, &l.gathered))
    }
    /// The settings the levels were made with, for a part made again from these levels.
    pub fn pin_corners(&self) -> bool {
        self.base.pin_corners
    }

    /// Every allocation the stack holds besides the active store's own columns and steps: the
    /// inactive top's columns, the gathered state, the skeletons and the numberings.
    pub fn other_allocations(&self, mut f: impl FnMut(Allocation)) {
        if self.live.is_some() {
            self.top.column_allocations(&mut f);
        }
        if let Some(l) = &self.live {
            l.gathered.column_allocations(&mut f);
        }
        for level in &self.all {
            if let Some(s) = &level.skeleton {
                s.topo
                    .iter()
                    .for_each(|t| f(crate::history::topo_allocation(t)));
                let (seam, shared) = &*s.lists;
                let entries: usize = seam.iter().chain(shared).map(Vec::len).sum();
                f(Allocation {
                    at: Arc::as_ptr(&s.lists).addr(),
                    bytes: entries * size_of::<u32>(),
                });
            }
            f(Allocation {
                at: Arc::as_ptr(&level.numbering).addr(),
                bytes: level.numbering.bytes(),
            });
            if let Some(uvs) = &level.uvs {
                let bytes = uvs.len() * size_of::<Corners>();
                f(Allocation {
                    at: Arc::as_ptr(uvs).addr(),
                    bytes,
                });
            }
        }
    }

    /// The active store and its numbering, giving up the levels: for a stack of one level.
    pub fn into_single(self) -> (MeshStore, Arc<Numbering>) {
        let numbering = Arc::clone(&self.all[self.active].numbering);
        let store = match self.live {
            Some(l) => l.store,
            None => self.top,
        };
        (store, numbering)
    }

    /// Fails if the active store no longer holds its level's very topology columns (a topology
    /// tool edited it), which would make every other level wrong.
    fn check(&self) -> Result<(), MeshError> {
        let held = match (&self.live, &self.all[self.active].skeleton) {
            (Some(l), Some(s)) => {
                (l.store.cols.topo.iter().zip(&s.topo)).all(|(a, b)| Arc::ptr_eq(a, b))
            }
            (None, None) => true,
            _ => false,
        };
        let sizes = self.all[self.active].numbering.vertex_count() == self.store().vertex_count();
        if held && sizes {
            Ok(())
        } else {
            Err(bad(
                "the active level's topology changed; a part with levels takes no topology edits",
            ))
        }
    }

    /// Carries the live store's edits, if any, up to the top, and drops the live store.
    fn carry_up(&mut self) -> Carried {
        let Some(live) = self.live.take() else {
            return Carried::default();
        };
        if same_edits(&live.store, &live.gathered) {
            return Carried::default();
        }
        let stack = carry::Stack {
            levels: &self.all,
            base: &self.base,
            live: &live.store,
            gathered: &live.gathered,
            k: self.active,
            everything: self.everything,
        };
        carry::carry(&stack, &mut self.top)
    }

    /// Level `j` gathered from the top onto its skeleton.
    fn gather(&self, j: usize) -> Live {
        let level = &self.all[j];
        let t = self.top_level();
        let Some(sk) = &level.skeleton else {
            unreachable!("only levels below the top are gathered");
        };
        let top_num = &self.all[t].numbering;
        let num = &level.numbering;
        let p = self.top.pos();
        let pos = gs_par::map(sk.topo.len(), |n| {
            let slots = sk.topo[n].ring.len();
            let col: Vec<Vec3> = (0..slots as u32)
                .map(|l| p.get(top_num.id(num.index(n as u32, l))))
                .collect();
            Arc::new(col)
        });
        let slots: Vec<usize> = sk.topo.iter().map(|t| t.ring.len()).collect();
        let faces: Vec<usize> = sk.topo.iter().map(|t| t.faces.len()).collect();
        let own = (j..t).find_map(|m| self.all[m].uvs.as_ref().map(|u| (m, &u[..])));
        let g = fill::Gather {
            top: &self.top,
            top_num,
            base: &self.base,
            num,
            j,
            t,
            own,
        };
        let cols = Columns {
            nrm: slots
                .iter()
                .map(|&s| Arc::new(vec![Vec3::ZERO; s]))
                .collect(),
            pos,
            topo: sk.topo.clone(),
            attrs: g.columns(&slots, &faces),
        };
        let lists = (*sk.lists).clone();
        let store = MeshStore::from_columns(self.top.cfg, self.top.defs.clone(), cols, Some(lists));
        Live {
            gathered: store.share(),
            store,
        }
    }

    /// Makes level `j` active: carries the live store's edits up to the top first, then
    /// gathers level `j` unless it is the top. Returns what carrying up wrote. The top holds
    /// normals only while it is active.
    pub fn set(&mut self, j: usize) -> Result<Carried, MeshError> {
        if j >= self.count() {
            return Err(MeshError::Input(format!("there is no level {j}")));
        }
        if j == self.active {
            return Ok(Carried::default());
        }
        self.check()?;
        let carried = self.carry_up();
        self.active = self.top_level();
        if j < self.top_level() {
            self.live = Some(self.gather(j));
            self.active = j;
            drop_normals(&mut self.top);
        } else {
            restore_normals(&mut self.top);
        }
        Ok(carried)
    }

    /// Adds a level on top and makes it active: carries pending edits up first, then one
    /// Catmull-Clark step (or a flat one) from the top, cut node by node without a global sort.
    pub fn divide(&mut self, smooth: bool) -> Result<Carried, MeshError> {
        self.divide_uvs(smooth, false)
    }

    /// [`Levels::divide`], with the UVs smoothed by the same Catmull-Clark step when
    /// `smooth_uvs` and `smooth` are both on and the part holds the two UV corner groups
    /// ([`uvs`]); the level below then keeps the UVs it had.
    pub fn divide_uvs(&mut self, smooth: bool, smooth_uvs: bool) -> Result<Carried, MeshError> {
        let carried = self.set(self.top_level())?;
        self.add_level(smooth, smooth_uvs, None)?;
        Ok(carried)
    }

    /// Adds a level on top of the top, which must be active, and makes it active: its points
    /// placed by the rule, or at `given` (the new level's positions by canonical index, which a
    /// file stores); given positions leave the new level's attribute columns empty.
    fn add_level(
        &mut self,
        smooth: bool,
        smooth_uvs: bool,
        given: Option<&[[f32; 3]]>,
    ) -> Result<(), MeshError> {
        let t = self.top_level();
        let level = Arc::clone(&self.all[t]);
        let num = &level.numbering;
        let faces = num.canonical_faces(&self.top);
        let old = num.vertex_count() as u32;
        let fcanon = num.face_canon();
        let first = cut::first_uses(&self.top, num, &faces, &fcanon);
        let step = Step::numbered(&faces, old, &first)
            .ok_or_else(|| bad("the subdivided mesh would be too big"))?;
        drop(first);
        if given.is_some_and(|g| g.len() < step.count as usize) {
            return Err(bad("fewer stored points than the level has"));
        }
        let src = step.sources(old);
        let coarse = cut::Coarse {
            store: &self.top,
            num,
            faces: &faces,
            step: &step,
            src: &src,
            fcanon: &fcanon,
        };
        let rule = Rule {
            topo: Topo {
                cfg: self.top.cfg,
                nodes: &self.top.cols.topo,
                num,
            },
            sharp: &level.sharp,
            smooth,
            pin_corners: self.base.pin_corners,
        };
        let pt = self.top.pos();
        let p = |c: u32| pt.get(num.id(c));
        let place = |v: u32| {
            if let Some(g) = given {
                return Vec3::from_array(g[v as usize]);
            }
            if v < old {
                return rule.vertex(v, &p);
            }
            let s = src[(v - old) as usize] as usize;
            let f = faces[s / 4];
            if step.center[s / 4] == v {
                rules::face_point(key(f), &p)
            } else {
                let (a, b) = ends(f, s % 4);
                rule.edge(a, b, &p)
            }
        };
        let (mut cols, numbering, layout) = cut::cut(&coarse, &place)?;
        let uvs = (smooth && smooth_uvs && given.is_none())
            .then(|| uvs::groups(&self.base, &self.top.defs, &self.top))
            .flatten()
            .map(|groups| uvs::Smooth::of(&coarse, &rule, &groups));
        cols.attrs = if given.is_some() {
            vec![vec![None; cols.len()]; self.top.defs.len()]
        } else {
            fill::built(&coarse, &layout, &self.base, uvs.as_ref())
        };
        let sharp = child_edges(&level.sharp, &faces, &step);
        let store = MeshStore::from_columns(self.top.cfg, self.top.defs.clone(), cols, None);
        let below = Level {
            skeleton: Some(Skeleton::of(&self.top)),
            uvs: uvs.map(|s| Arc::new(s.coarse)),
            ..(*level).clone()
        };
        self.all[t] = Arc::new(below);
        self.all.push(Arc::new(Level {
            numbering: Arc::new(numbering),
            skeleton: None,
            smooth,
            sharp,
            uvs: None,
        }));
        self.top = store;
        self.active = t + 1;
        Ok(())
    }

    /// Makes the active level the top, dropping every level above it; its pending edits stay,
    /// since nothing above is left to update. False if the active level is the top already.
    pub fn delete_higher(&mut self) -> bool {
        let Some(live) = self.live.take() else {
            return false;
        };
        let k = self.active;
        self.all.truncate(k + 1);
        let top = Level {
            skeleton: None,
            uvs: None,
            ..(*self.all[k]).clone()
        };
        self.all[k] = Arc::new(top);
        self.top = live.store;
        true
    }

    /// Makes the active level the base, dropping every level below it: its faces become the
    /// base faces and its sharp edges the base creases; the levels above keep their numbering.
    /// False if the active level is the base already.
    pub fn delete_lower(&mut self) -> bool {
        let k = self.active;
        if k == 0 {
            return false;
        }
        let faces = self.all[k].numbering.canonical_faces(self.store());
        self.base = Arc::new(Base::new(
            faces,
            self.base.pin_corners,
            self.base.corners.clone(),
        ));
        self.all.drain(..k);
        self.active = 0;
        true
    }
}

/// The sharp edges of the level above: each sharp edge's two halves, one level less sharp.
fn child_edges(sharp: &[Sharp], faces: &[[u32; 4]], step: &Step) -> Vec<Sharp> {
    if sharp.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (fi, &f) in faces.iter().enumerate() {
        for i in 0..arity(f) {
            let (a, b) = ends(f, i);
            let Ok(at) = sharp.binary_search_by(|e| (e.0, e.1).cmp(&(a, b))) else {
                continue;
            };
            let s = child_sharpness(sharp[at].2);
            let e = step.edge[fi * 4 + i];
            if s > 0 && e != NO {
                out.push((a.min(e), a.max(e), s));
                out.push((b.min(e), b.max(e), s));
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}
