//! Live Detail (decision 0021): triangles added under the brush where the surface needs them
//! and removed where it has too many, while it sculpts.
//!
//! [`refine`] is the one home of the algorithm. A refining stroke calls it once per dab, after
//! the dab's frames are built for every symmetry pass and before the kernel's node query, so the
//! kernel moves the new points in the same dab. It edges toward a target length:
//!
//! 1. targets: `max` from the mode and the dab radius, `min = merge * max`;
//! 2. region: the visible faces whose closest point lies inside a pass sphere;
//! 3. nodes owning a corner of the region and over three quarters full are cut first, so no
//!    node is cut and no id changes during the rest of the pass;
//! 4. short edges of the region's faces collapse, shortest first; 5. long ones split, longest
//!    first, each after any longer side of its triangles (longest-edge bisection, which may
//!    reach past the sphere but only adds points there); 6. edges around what changed flip
//!    toward valence 6; 7. changed points relax along the surface. Steps 4, 6 and 7 stay
//!    inside the spheres grown by `max`, so no point farther out moves (and, over a region a
//!    selector picks, on its faces' corners and the points the pass makes);
//! 8. slots freed by collapses are released only at the end, so none is reused within a pass.
//!
//! Every split, collapse, flip and quad split is one operation, and a pass stops cleanly at its
//! budget; the rest waits for the next dab. There is no time budget anywhere, so the same
//! stroke gives the same mesh on any machine and at any thread count.

mod pass;
mod pick;
mod whole;

pub use pick::{TYPICAL_EDGE, typical};
pub use whole::refine_all;

use crate::brush::{BrushPreset, Column, KernelSpec, StrokeSpec};
use crate::config::{FaceId, NodeId, VertId};
use crate::math::{Aabb, NO, Vec3, arity, closest_on_triangle, face_triangles};
use crate::par::merge;
use crate::store::MeshStore;
use crate::view::{PosRead, TopoRead};
use pass::Pass;
use serde::Serialize;

/// How the longest edge a dab keeps is worked out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// This many times the dab radius (brush mode; screen mode resolves to it too).
    Relative(f32),
    /// A length in scene units.
    Constant(f32),
}

/// The settings a refining stroke uses, resolved (decision 0021, section 4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveDetail {
    pub target: Target,
    /// `min / max`: edges shorter than `merge` times the target merge. At most
    /// [`MOST_MERGE`] is used, so both halves of a split edge stay above `min`.
    pub merge: f32,
    /// Collapse short edges; off only adds triangles.
    pub combined: bool,
    /// Operations per dab.
    pub budget: u32,
    /// Put a split's new point on the smooth curve through both ends (the Hermite midpoint);
    /// off, on the straight edge.
    pub smooth_split: bool,
    /// How far changed points slide toward the middle of their neighbours, 0 to 1.
    pub relax: f32,
}

/// The largest `merge` used: with it, both halves of a split long edge are still not short.
pub const MOST_MERGE: f32 = 0.45;

/// The shortest target a dab keeps, as a fraction of its radius, whatever the mode asks: at it,
/// one brush already holds 290,000 triangles, and a script asking for far less by accident
/// would ask for millions (review 0006, S2). Blender's
/// brush-relative detail stops at the same half percent (`detail_percent`, 0.5 to 100, in
/// `source/blender/makesrna/intern/rna_sculpt_paint.cc`).
pub const LEAST_SIZE: f32 = 0.005;

impl Default for LiveDetail {
    fn default() -> LiveDetail {
        LiveDetail {
            target: Target::Relative(0.25),
            merge: 0.4,
            combined: true,
            budget: 4096,
            smooth_split: true,
            relax: 0.5,
        }
    }
}

impl LiveDetail {
    /// Refuse settings no stroke can use, naming the first.
    pub fn check(&self) -> Result<(), String> {
        let size = match self.target {
            Target::Relative(k) => k,
            Target::Constant(l) => l,
        };
        if !(size > 0.0 && size.is_finite()) {
            return Err(format!(
                "the live detail size must be a positive number, got {size}"
            ));
        }
        if !(0.0..=1.0).contains(&self.merge) {
            return Err(format!(
                "live detail merge must be from 0 to 1, got {}",
                self.merge
            ));
        }
        if !(0.0..=1.0).contains(&self.relax) {
            return Err(format!(
                "live detail relax must be from 0 to 1, got {}",
                self.relax
            ));
        }
        if self.budget == 0 {
            return Err("the live detail budget must be at least one operation".into());
        }
        Ok(())
    }

    /// The longest and shortest edge a dab of radius `r` keeps: the longest at least
    /// [`LEAST_SIZE`] times `r`.
    pub fn lengths(&self, r: f32) -> (f32, f32) {
        let max = match self.target {
            Target::Relative(k) => k * r,
            Target::Constant(l) => l,
        }
        .max(LEAST_SIZE * r);
        (max, max * self.merge.min(MOST_MERGE))
    }
}

/// What refinement passes did, summed over a stroke.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveReport {
    /// Passes run (one per dab).
    pub passes: u32,
    pub splits: u32,
    pub collapses: u32,
    pub flips: u32,
    pub quad_splits: u32,
    /// Points added and removed.
    pub added: u32,
    pub removed: u32,
    /// Passes that stopped at the operation budget.
    pub budget_hits: u32,
    /// Splits left for a later dab: no room in their nodes, or a longer edge beside them that
    /// could not split first.
    pub waits: u32,
    /// Nodes cut in two before a pass.
    pub node_cuts: u32,
    /// The most operations one pass ran.
    pub most_ops: u32,
    /// A pass changed the topology without keeping a mirror map, so it released the map
    /// (decision 0021, section 10).
    pub mirror_cleared: bool,
    /// A pass mirrored across a plane with no map there and changed the topology: its halves
    /// match only as far as floating point allows.
    pub mirror_loose: bool,
}

impl LiveReport {
    /// Operations run in all.
    pub fn ops(&self) -> u32 {
        self.splits + self.collapses + self.flips + self.quad_splits
    }
    /// Whether any pass changed the topology.
    pub fn changed(&self) -> bool {
        self.ops() > 0 || self.node_cuts > 0
    }
    fn add(&mut self, o: &LiveReport) {
        self.passes += o.passes;
        self.splits += o.splits;
        self.collapses += o.collapses;
        self.flips += o.flips;
        self.quad_splits += o.quad_splits;
        self.added += o.added;
        self.removed += o.removed;
        self.budget_hits += o.budget_hits;
        self.waits += o.waits;
        self.node_cuts += o.node_cuts;
        self.most_ops = self.most_ops.max(o.most_ops);
        self.mirror_cleared |= o.mirror_cleared;
        self.mirror_loose |= o.mirror_loose;
    }
}

/// Why a stroke does not refine, from its preset alone (decision 0021, section 3): the one list
/// of these reasons, each worded by [`NotRefined::why`] and of the kind [`NotRefined::blocked`]
/// gives. The engine adds the reasons that depend on the part or the call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotRefined {
    /// Drag, anchored, curve and gesture strokes never refine.
    Class,
    /// The Move kernel never refines.
    Kernel,
    /// A brush that paints a column (the Mask Pen's mask, hiding, color, face groups) moves no
    /// point, so it never refines (research 24, wrong result 8).
    Paint(Column),
    /// An automask that limits where the brush reaches: the pass would rebuild the surface it
    /// spares (amended 2026-10-12).
    Automask(Automasked),
    /// Wrap Mode above 1: each tile would refine its own way under one budget (decision 0054,
    /// section 6).
    Wrap,
}

impl NotRefined {
    /// The reason in a sentence about `preset`, with the setting that turns it off when there
    /// is one: what a stroke's answer and the window's cursor show.
    pub fn why(self, preset: &BrushPreset) -> String {
        let name = &preset.name;
        match self {
            Self::Class => {
                format!(
                    "{name} drags, anchors or follows a curve; only brushes that lay dabs refine"
                )
            }
            Self::Kernel => format!("{name} moves what it grabs, which never refines"),
            Self::Paint(column) => {
                let what = match column {
                    Column::Mask => "the mask",
                    Column::Hide => "hidden faces",
                    Column::Color => "color",
                    Column::Group => "face groups",
                    Column::UvHint => "UV hints",
                    Column::UvDensity => "UV density",
                };
                format!("{name} paints {what} and moves no point, which never changes the topology")
            }
            Self::Automask(which) => {
                let (field, spares) = match which {
                    Automasked::Backface => ("backface", "what faces away"),
                    Automasked::FaceGroup => ("face_group", "the other groups"),
                    Automasked::Topology => ("topology", "what is not joined to the stroke start"),
                };
                format!(
                    "{name}'s {} automask spares {spares}, which Live Detail would rebuild all the \
                     same; settings {{\"automask\": {{\"{field}\": 0}}}} turns it off",
                    field.replace('_', " ")
                )
            }
            Self::Wrap => format!(
                "{name} repeats its stroke over the part's tiles (Wrap Mode {}), and each tile would \
                 refine its own way; settings {{\"tip\": {{\"wrap\": 1}}}} turns it off",
                preset.tip.wrap
            ),
        }
    }

    /// The kind of block the reason is.
    pub fn blocked(self) -> Blocked {
        match self {
            Self::Class | Self::Kernel | Self::Paint(_) | Self::Wrap => Blocked::Brush,
            Self::Automask(Automasked::Backface) => Blocked::Backface,
            Self::Automask(Automasked::FaceGroup) => Blocked::FaceGroup,
            Self::Automask(Automasked::Topology) => Blocked::Topology,
        }
    }
}

/// What keeps Live Detail from refining a part under a brush, by kind (decision 0021, section
/// 3, amended 2026-10-12): the one list of kinds. The engine names a kind in `session.state`'s
/// `live_detail.blocked` and the window words it after "No Live Detail: " ([`Blocked::label`]).
/// Each [`NotRefined`] reason has its kind; the engine adds the part's two and the opt-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    /// The part has detail levels, frozen or not, which keep their topology.
    Levels,
    /// The part has UV columns, which refining would tear.
    Uvs,
    /// The brush never refines: it drags, moves what it grabs, paints a column or repeats over
    /// tiles.
    Brush,
    /// The brush's backface automask spares what faces away, which refining would rebuild all
    /// the same (decision 0021, section 3); so do the next two for what they spare.
    Backface,
    /// The brush's face group automask spares the other face groups.
    FaceGroup,
    /// The brush's topology automask spares what is not joined to the stroke start.
    Topology,
    /// The brush could refine, but its own settings turn Live Detail off.
    OptOut,
}

impl Blocked {
    /// Every kind, in the order decision 0021 (section 8) lists the reasons.
    pub const ALL: [Blocked; 7] = [
        Self::Levels,
        Self::Uvs,
        Self::Brush,
        Self::Backface,
        Self::FaceGroup,
        Self::Topology,
        Self::OptOut,
    ];

    /// The kind in a few words, as the window shows it after "No Live Detail: " beside the pen
    /// and, for an automask, on the status line. `live_detail.why_not` says it at length.
    pub fn label(self) -> &'static str {
        self.row().0
    }

    /// Whether the brush's own automask stands in the way, which the window dims its Live
    /// Detail switch for (decision 0021, section 3).
    pub fn automask(self) -> bool {
        self.row().1
    }

    /// What the window makes of the kind, one row each: its words ([`Blocked::label`]) and
    /// whether it dims the switch ([`Blocked::automask`]).
    fn row(self) -> (&'static str, bool) {
        match self {
            Self::Levels => ("detail levels", false),
            Self::Uvs => ("UV columns", false),
            Self::Brush => ("this brush", false),
            Self::Backface => ("backface automask", true),
            Self::FaceGroup => ("face group automask", true),
            Self::Topology => ("topology automask", true),
            Self::OptOut => ("off for this brush", false),
        }
    }
}

/// The automasks that keep a brush from refining, in the order `refuses` names the first that
/// is on. The cavity automask is not one: it limits no region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Automasked {
    /// Spares what faces away from the view.
    Backface,
    /// Spares the face groups other than the one under the stroke start.
    FaceGroup,
    /// Spares what is not joined to the stroke start.
    Topology,
}

/// Whether a stroke with `preset` can refine, and if not, why.
pub fn refuses(preset: &BrushPreset) -> Option<NotRefined> {
    if !matches!(preset.stroke, StrokeSpec::Dab { .. }) {
        return Some(NotRefined::Class);
    }
    if !preset.sculpts() {
        // Mode off: the brush only paints, by its `[paint]` table (decision 0039, section 8).
        return Some(NotRefined::Paint(Column::Color));
    }
    match &preset.kernel {
        KernelSpec::Move(_) => return Some(NotRefined::Kernel),
        KernelSpec::Attribute(a) => return Some(NotRefined::Paint(a.column)),
        _ => {}
    }
    let am = &preset.automask;
    let on = [
        (am.backface, Automasked::Backface),
        (am.face_group, Automasked::FaceGroup),
        (am.topology, Automasked::Topology),
    ];
    let first = on.into_iter().find(|&(strength, _)| strength != 0.0);
    let wrap = (preset.tip.wrap > 1).then_some(NotRefined::Wrap);
    first.map(|(_, which)| NotRefined::Automask(which)).or(wrap)
}

/// One refinement pass over the pass spheres (`center`, `radius`), one per symmetry pass, all
/// of the dab's radius. See the module docs. `mirrored` names the planes the passes mirror
/// across (none for a radial stroke): a mirror map across the first of them that has one is
/// kept, every edit made in pairs; any other map is released once the pass changes the
/// topology. Returns the nodes whose own columns it wrote, ascending.
pub fn refine(
    store: &mut MeshStore,
    spheres: &[(Vec3, f32)],
    ld: &LiveDetail,
    mirrored: [bool; 3],
    report: &mut LiveReport,
) -> Vec<NodeId> {
    let Some(&(_, r)) = spheres.first() else {
        return Vec::new();
    };
    let (max, min) = ld.lengths(r);
    if !(max > 0.0 && max.is_finite()) {
        return Vec::new();
    }
    run_pass(store, spheres, (max, min), ld, mirrored, report, |s| {
        region(s, spheres, max)
    })
}

/// The pass of [`refine`] over the region `find` gives, found again after nodes are cut (which
/// renumbers faces), with the targets `(max, min)` already worked out.
fn run_pass(
    store: &mut MeshStore,
    spheres: &[(Vec3, f32)],
    (max, min): (f32, f32),
    ld: &LiveDetail,
    mirrored: [bool; 3],
    report: &mut LiveReport,
    find: impl Fn(&MeshStore) -> Region,
) -> Vec<NodeId> {
    let mut out = LiveReport {
        passes: 1,
        ..LiveReport::default()
    };
    let mut reg = find(store);
    let full = full_nodes(store, &reg.nodes);
    let mut written = Vec::new();
    if !full.is_empty() {
        let mut ed = store.edit_topology();
        for &n in &full {
            ed.split_node(n);
        }
        written = ed.written();
        drop(ed);
        out.node_cuts = full.len() as u32;
        reg = find(store);
    }
    let maps = store.mirror_maps();
    let kept = maps.iter().copied().find(|m| mirrored[m.0]);
    let mut ed = store.edit_topology();
    ed.hold();
    let only = reg.points.as_deref();
    Pass::new(&mut ed, spheres, max, min, ld, kept, only).run(&reg.faces, reg.whole, &mut out);
    written = merge(&written, &ed.written());
    drop(ed);
    if out.ops() > 0 {
        for &(axis, _) in maps.iter().filter(|&&m| Some(m) != kept) {
            written = merge(&written, &store.clear_mirror(axis));
            out.mirror_cleared = true;
        }
        out.mirror_loose = (0..3).any(|a| mirrored[a] && kept.is_none_or(|m| m.0 != a));
    }
    report.add(&out);
    written
}

/// The faces a pass may change and the nodes owning their corners.
struct Region {
    /// Visible faces whose closest point lies inside a pass sphere (or that a selector picks),
    /// ascending.
    faces: Vec<FaceId>,
    /// Nodes holding one of them or owning a corner of one, ascending: those over three
    /// quarters full are cut before the pass so its splits find room.
    nodes: Vec<NodeId>,
    /// For a picked region, its faces' corners, ascending: of the points the pass did not make,
    /// the only ones it moves, removes or turns edges around. `None` for a dab's region, which
    /// the spheres bound.
    points: Option<Vec<VertId>>,
    /// A whole-part region ([`refine_all`]): every quad no split needed is split as well, so
    /// the faces come out triangles. A dab's region keeps them.
    whole: bool,
}

/// Faces one job of the region scan tests.
const RUN: usize = 2048;

fn region(store: &MeshStore, spheres: &[(Vec3, f32)], max: f32) -> Region {
    let grown: Vec<(Vec3, f32)> = spheres.iter().map(|&(c, r)| (c, r + max)).collect();
    let q = store.query_all(&grown);
    let nodes = merge(&q.brushed, &q.shell);
    let (vis, pos) = (store.visible(), store.pos());
    // Each node's points placed against the spheres' box once: most faces of a node lie far
    // from the brush, and their corners' places pass them over without a distance test.
    let reach = Reach::of(spheres);
    let places: Vec<Vec<u8>> = gs_par::map(nodes.len(), |k| {
        let pts = store.node(nodes[k]).pos;
        pts.iter().map(|&p| reach.place(p)).collect()
    });
    // A node's faces in runs of `RUN`, so one big node spreads over the threads; the runs are
    // joined in order, so the list does not depend on the thread count.
    let runs: Vec<(usize, usize)> = (0..nodes.len())
        .flat_map(|k| {
            (0..store.node(nodes[k]).faces.len())
                .step_by(RUN)
                .map(move |at| (k, at))
        })
        .collect();
    let per = gs_par::map(runs.len(), |i| {
        let (k, at) = runs[i];
        let (n, own) = (nodes[k], &places[k]);
        let node = store.node(n);
        // A vertex id is its node's base plus its slot, so `v ^ base` is the slot of a vertex
        // the node owns and past every slot for any other node's.
        let place = |v: u32| match (v ^ node.base) as usize {
            l if l < own.len() => own[l],
            _ => far(&reach, pos.get(v)),
        };
        let run = &node.faces[at..node.faces.len().min(at + RUN)];
        let mut out = Vec::new();
        for (l, &fc) in run.iter().enumerate() {
            if fc[0] == NO {
                continue;
            }
            let last = if fc[3] == NO { fc[2] } else { fc[3] };
            let beyond = place(fc[0]) & place(fc[1]) & place(fc[2]) & place(last);
            let f = node.first_face + (at + l) as u32;
            if beyond == 0 && vis.face(f) && touches(spheres, fc, |v| pos.get(v)) {
                out.push(f);
            }
        }
        out
    });
    Region::of(store, per.concat())
}

impl Region {
    /// The region of `faces` (visible, ascending), with the nodes owning them and their corners.
    fn of(store: &MeshStore, faces: Vec<FaceId>) -> Region {
        let (cfg, topo) = (store.config(), store.topo());
        let mut owners: Vec<NodeId> = (faces.iter())
            .flat_map(|&f| {
                let fc = topo.face(f);
                let corners = (0..arity(fc)).map(move |k| cfg.node_of(fc[k]));
                corners.chain([cfg.face_node(f)])
            })
            .collect();
        owners.sort_unstable();
        owners.dedup();
        Region {
            faces,
            nodes: owners,
            points: None,
            whole: false,
        }
    }

    /// The region with its faces' corners as the only points it may move: for a picked region,
    /// which no sphere bounds.
    fn held(mut self, store: &MeshStore) -> Region {
        let topo = store.topo();
        let mut points: Vec<VertId> = (self.faces.iter())
            .flat_map(|&f| {
                let fc = topo.face(f);
                (0..arity(fc)).map(move |k| fc[k])
            })
            .collect();
        points.sort_unstable();
        points.dedup();
        self.points = Some(points);
        self
    }
}

/// The box around the pass spheres, padded far past float rounding at the scene's scale. A
/// face whose corners all lie beyond one side of it is farther than the pad from every sphere,
/// so [`touches`] would refuse it: only its rounding on a nearly flat triangle could say
/// otherwise.
struct Reach {
    lo: Vec3,
    hi: Vec3,
}

impl Reach {
    fn of(spheres: &[(Vec3, f32)]) -> Reach {
        let (mut b, mut scale) = (Aabb::EMPTY, 0.0f32);
        for &(c, r) in spheres {
            b.grow(c - Vec3::new(r, r, r));
            b.grow(c + Vec3::new(r, r, r));
            scale = scale.max(c.x.abs()).max(c.y.abs()).max(c.z.abs()).max(r);
        }
        // 2^-12 of the scale: thousands of times the rounding of a distance at that scale.
        let pad = scale / 4096.0;
        let d = Vec3::new(pad, pad, pad);
        Reach {
            lo: b.lo - d,
            hi: b.hi + d,
        }
    }

    /// The sides of the box `p` lies beyond, one bit each: below and above in x, y, then z.
    /// No bits for a dead slot.
    #[inline]
    fn place(&self, p: Vec3) -> u8 {
        u8::from(p.x < self.lo.x)
            | u8::from(p.x > self.hi.x) << 1
            | u8::from(p.y < self.lo.y) << 2
            | u8::from(p.y > self.hi.y) << 3
            | u8::from(p.z < self.lo.z) << 4
            | u8::from(p.z > self.hi.z) << 5
    }
}

/// [`Reach::place`] for a corner another node owns: out of line, as few faces have one.
#[cold]
#[inline(never)]
fn far(reach: &Reach, p: Vec3) -> u8 {
    reach.place(p)
}

/// Nodes among `nodes` with more than three quarters of their point or face slots in use.
fn full_nodes(store: &MeshStore, nodes: &[NodeId]) -> Vec<NodeId> {
    let cfg = store.config();
    let (vcap, fcap) = (cfg.capacity() as usize, cfg.face_capacity() as usize);
    (nodes.iter().copied())
        .filter(|&n| {
            let t = &store.cols.topo[n as usize];
            let (v, f) = (t.ring.len() - t.vfree.len(), t.faces.len() - t.ffree.len());
            4 * v > 3 * vcap || 4 * f > 3 * fcap
        })
        .collect()
}

/// Whether face `fc` (a quad as the two triangles it is drawn as) has its closest point inside
/// a pass sphere: the region's faces, every edge of which is a candidate of the pass (decision
/// 0021, section 2, step 2).
fn touches(spheres: &[(Vec3, f32)], fc: [u32; 4], pos: impl Fn(u32) -> Vec3) -> bool {
    let p = fc.map(|v| if v == NO { Vec3::ZERO } else { pos(v) });
    let tris = face_triangles(fc, |k| p[k].to_array());
    // Squared distance to a triangle: its closest point by gs-core's one search.
    let dist2 = |c: Vec3, t: [Vec3; 3]| (closest_on_triangle(c, &t).0 - c).len2();
    spheres
        .iter()
        .any(|&(c, r)| (tris.iter()).any(|t| dist2(c, t.map(|k| p[k])) <= r * r))
}
