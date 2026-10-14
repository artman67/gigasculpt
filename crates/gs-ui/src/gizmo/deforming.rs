//! The open deformer's handles in the 3D view (decision 0041, amended 2026-10-13 for `DF7c`): what
//! the window last heard of it ([`Deforming`]), a press on one of its handles until the pen
//! lifts ([`Hold`]), and the pen rectangle on the model that selects a cage's or a curve's dots
//! ([`Rectangle`]). A drag writes its argument into the open's calls as they were at the press,
//! each sample one `update` that replaces the one still waiting (`Shell::queue`), so a frame
//! sends one; Accept, Cancel and the lift are the deformer's own (`deformer`).

use std::collections::BTreeSet;

use gs_core::camera::Camera;
use gs_core::deform::Deform;
use gs_core::gizmo::{
    Deformer, DeformerHandle, DeformerHandles, Drive, Kind, MOVE_SNAP, Moved, Style, along_ray,
    deformer_handles, on_plane, pick_deformer, snap_move, sweep, toward_viewer,
};
use gs_core::transform::{Op, Similarity, Transform};
use gs_core::{Aabb, NodeId, Quat};
use gs_engine::{PartReport, SessionState};
use gs_protocol::Step;
use serde_json::{Value, json};

use super::deformer::update;
use super::{PenPress, Seen, Took, Tool};
use crate::pieces::deform::num;
use crate::pieces::spring::session;
use crate::record::with_args;

/// The open deformer as the window last heard it, and the dots selected on it.
pub(super) struct Deforming {
    /// Its first call's transform, which its handles drag.
    transform: Transform,
    /// Its part's live points' box in its own space and where the part is, which an operation
    /// that takes no box is measured on.
    bounds: Aabb,
    placed: Similarity,
    /// Its calls, as `session.state` reports them.
    calls: Vec<Value>,
    /// What keeps the selection across calls: its part, its operation and its first frame.
    key: String,
    /// The selected dots, by name; none with each new deformer.
    pub(super) selected: BTreeSet<String>,
}

impl Deforming {
    /// Its handles seen through `camera` in a view `size` pixels wide and high.
    pub(super) fn shown(&self, camera: &Camera, size: [f32; 2], style: &Style) -> DeformerHandles {
        let d = Deformer::of(&self.transform, self.bounds, &self.placed);
        deformer_handles(&self.transform.frame, &d, (camera, size, style))
    }

    /// What its handle `h` drags, for the cursor over it (`deformer::words`), a turn snapping
    /// to `steps` of one; a length's ends measured on its part's box where the rows keep none.
    pub(super) fn words(&self, ctx: &egui::Context, h: &DeformerHandle, steps: u32) -> Vec<String> {
        let diagonal = self
            .placed
            .length((self.bounds.hi - self.bounds.lo).len2().sqrt());
        super::deformer::words(ctx, &self.calls, h, (diagonal, steps))
    }

    /// Its handles are dots: a press on the model draws the pen rectangle.
    fn dotted(&self) -> bool {
        let dots = |d: &Deform| matches!(d, Deform::Lattice(_) | Deform::Curve(_));
        matches!(&self.transform.op, Op::Deform(d) if dots(d))
    }
}

/// A deformer handle the pen holds, from the press until the pen lifts.
pub(super) struct Hold {
    /// The handle as the press found it.
    pub(super) handle: DeformerHandle,
    /// For a dot: every dot as the press found it, and whether the drag moves it.
    dots: Vec<(DeformerHandle, bool)>,
    /// The frame's axes, which a dot's step is written along.
    axes: Quat,
    camera: Camera,
    size: [f32; 2],
    /// Where the press began, and the last sample the ring measured from, in the view's pixels.
    start: [f32; 2],
    last: [f32; 2],
    /// The angle the pen swept around the ring's centre so far, in radians, counterclockwise.
    swept: f32,
    /// The open's calls at the press: each update is them with the drag written in.
    calls: Vec<Value>,
    /// The last value the drag sent, while one went out.
    sent: Option<Value>,
    /// What a lift with nothing sent does to the selection.
    tap: Tap,
}

/// A tap on a dot (section 4): a press on an unselected one selected it already.
enum Tap {
    Keep,
    Alone(String),
    Remove(String),
}

/// The pen rectangle, in the view's pixels.
pub(super) struct Rectangle {
    pub(super) start: [f32; 2],
    pub(super) end: [f32; 2],
}

impl Tool {
    /// Takes the open deformer from `state` after a call, on its part among `parts`; the
    /// selection stays while the same deformer stays open.
    pub fn heard(&mut self, state: &SessionState, parts: &[PartReport]) {
        let kept = self.deforming.take();
        self.deforming = state.deformer.as_ref().and_then(|d| {
            let transform = d.transform()?;
            let p = parts.iter().find(|p| p.name == d.part)?;
            let nodes = 0..p.store.node_count() as NodeId;
            let bounds = (nodes.map(|n| p.store.bounds(n))).fold(Aabb::EMPTY, Aabb::union);
            let calls = serde_json::to_value(&d.transforms)
                .ok()?
                .as_array()?
                .clone();
            let key = format!("{} {} {}", d.part, d.kind, calls.first()?["frame"]);
            let kept = kept.filter(|k| k.key == key);
            Some(Deforming {
                transform,
                bounds,
                placed: p.transform,
                calls,
                key,
                selected: kept.map(|k| k.selected).unwrap_or_default(),
            })
        });
    }

    /// The deformer handle in use: the one the pen holds, or else the one a press at `hover`
    /// (the view's pixels) would grab among `shown`.
    pub(super) fn hot_deformer<'a>(
        &'a self,
        shown: &'a DeformerHandles,
        style: &Style,
        hover: Option<[f32; 2]>,
    ) -> Option<&'a DeformerHandle> {
        if let PenPress::Hold(h) = &self.press {
            return Some(&h.handle);
        }
        let radius = style.pick_pt * style.px_per_pt;
        let i = pick_deformer(&shown.handles, hover?, radius)?;
        Some(&shown.handles[i])
    }

    /// A press at `at` (the view's pixels) on the open deformer's handles: it holds the one it
    /// lands on. On a dot, Ctrl removes it from the selection; else an unselected dot is
    /// selected first, added with Shift (section 4). `None` off every handle.
    pub(super) fn press_deformer(&mut self, at: [f32; 2], seen: &Seen) -> Option<Took> {
        let d = self.deforming.as_mut()?;
        let shown = d.shown(seen.camera, seen.size, &seen.style);
        let radius = seen.style.pick_pt * seen.style.px_per_pt;
        let handle = shown.handles[pick_deformer(&shown.handles, at, radius)?].clone();
        let took = Took {
            gizmo: true,
            ..Took::default()
        };
        let name = handle.name();
        let was = d.selected.contains(&name);
        let tap = match (handle.kind, was, seen.shift) {
            (Kind::Dot, _, _) if seen.ctrl => {
                d.selected.remove(&name);
                self.press = PenPress::Done;
                return Some(took);
            }
            (Kind::Dot, true, true) => Tap::Remove(name),
            (Kind::Dot, true, false) => Tap::Alone(name),
            (Kind::Dot, false, shift) => {
                if !shift {
                    d.selected.clear();
                }
                d.selected.insert(name);
                Tap::Keep
            }
            _ => Tap::Keep,
        };
        let dots = (shown.handles.iter())
            .filter(|h| handle.kind == Kind::Dot && h.kind == Kind::Dot)
            .map(|h| (h.clone(), d.selected.contains(&h.name())))
            .collect();
        self.press = PenPress::Hold(Box::new(Hold {
            handle,
            dots,
            axes: d.transform.frame.axes,
            camera: *seen.camera,
            size: seen.size,
            start: at,
            last: at,
            swept: 0.0,
            calls: d.calls.clone(),
            sent: None,
            tap,
        }));
        Some(took)
    }

    /// Whether a press on the model draws the pen rectangle: a cage or a curve is open.
    pub(super) fn rectangles(&self) -> bool {
        self.deforming.as_ref().is_some_and(Deforming::dotted)
    }

    /// One pen sample at `at` on held handle `h`: a drag's update while `touching`, the press's
    /// values back on the barrel button, the selection's tap at the lift.
    pub(super) fn hold(
        &mut self,
        mut h: Hold,
        at: [f32; 2],
        (touching, barrel): (bool, bool),
        seen: &Seen,
    ) -> Vec<Step> {
        if barrel {
            self.press = PenPress::Done;
            return h.cancel().into_iter().collect();
        }
        if touching {
            let step = h.follow(at, seen.shift.then_some(seen.prefs.snap));
            self.press = PenPress::Hold(Box::new(h));
            return step.into_iter().collect();
        }
        if let (Some(d), None) = (self.deforming.as_mut(), &h.sent) {
            match h.tap {
                Tap::Keep => {}
                Tap::Alone(name) => d.selected = BTreeSet::from([name]),
                Tap::Remove(name) => {
                    d.selected.remove(&name);
                }
            }
        }
        Vec::new()
    }

    /// One pen sample at `at` on the pen rectangle `r`: it follows the pen while `touching`;
    /// at the lift it selects the dots inside (Shift adds, Ctrl removes), and a tap clears the
    /// selection. The barrel button lets it go.
    pub(super) fn rectangle(
        &mut self,
        mut r: Rectangle,
        at: [f32; 2],
        (touching, barrel): (bool, bool),
        seen: &Seen,
    ) {
        if barrel {
            self.press = PenPress::Done;
            return;
        }
        if touching {
            r.end = at;
            self.press = PenPress::Rect(Box::new(r));
            return;
        }
        let Some(d) = self.deforming.as_mut() else {
            return;
        };
        let radius = seen.style.pick_pt * seen.style.px_per_pt;
        let (dx, dy) = (r.end[0] - r.start[0], r.end[1] - r.start[1]);
        if libm::hypotf(dx, dy) <= radius {
            d.selected.clear();
            return;
        }
        let within = |p: [f32; 2], i: usize| {
            let (a, b) = (r.start[i], r.end[i]);
            a.min(b) <= p[i] && p[i] <= a.max(b)
        };
        let shown = d.shown(seen.camera, seen.size, &seen.style);
        let inside = (shown.handles.iter()).filter(|h| {
            let p = seen.camera.project(h.at.to_array(), seen.size);
            h.kind == Kind::Dot && p.is_some_and(|p| within(p, 0) && within(p, 1))
        });
        if !seen.shift && !seen.ctrl {
            d.selected.clear();
        }
        for h in inside {
            if seen.ctrl {
                d.selected.remove(&h.name());
            } else {
                d.selected.insert(h.name());
            }
        }
    }
}

impl Hold {
    /// The pen at `at`: the update its drag makes, when it changed. While `snap` is given
    /// (Shift), a length snaps to [`MOVE_SNAP`] and an angle to that many steps of a turn.
    fn follow(&mut self, at: [f32; 2], snap: Option<u32>) -> Option<Step> {
        let h = &self.handle;
        let (by, value) = match h.kind {
            Kind::Cone => {
                let line = (h.at, h.along);
                let to = along_ray(&self.camera, self.size, line, at)?;
                let by = to - along_ray(&self.camera, self.size, line, self.start)?;
                (by, written(h.moved(by, snap)?))
            }
            Kind::Ring => {
                let centre = self.camera.project(h.at.to_array(), self.size)?;
                if let Some(turned) = sweep(centre, self.last, at) {
                    self.swept += turned;
                    self.last = at;
                }
                // Counterclockwise looking down the axis, which may point away.
                let away = h.along.dot(toward_viewer(&self.camera, h.at)) < 0.0;
                let by = self.swept.to_degrees() * if away { -1.0 } else { 1.0 };
                (by, written(h.moved(by, snap)?))
            }
            Kind::Dot => self.dragged(at, snap.is_some())?,
        };
        if (by == 0.0 && self.sent.is_none()) || self.sent.as_ref() == Some(&value) {
            return None;
        }
        self.sent = Some(value.clone());
        Some(update(&self.calls, h.drive.at(), &value))
    }

    /// A dot drag to `at`: every selected dot moved by the same step in the screen plane
    /// through the pressed one (with `snap`, to [`MOVE_SNAP`] along the frame's axes), and the
    /// cage's moved points or the curve's points written whole. The step's length comes first.
    fn dragged(&self, at: [f32; 2], snap: bool) -> Option<(f32, Value)> {
        let plane = (self.handle.at, self.camera.axes().2);
        let to = on_plane(&self.camera, self.size, plane, at)?;
        let mut by = to - on_plane(&self.camera, self.size, plane, self.start)?;
        if snap {
            by = snap_move(by, self.axes, MOVE_SNAP);
        }
        let step = self.axes.inverse().rotate(by).to_array();
        let moved = |p: [f32; 3], moves: bool| {
            let p: [f32; 3] = std::array::from_fn(|a| if moves { p[a] + step[a] } else { p[a] });
            p.map(num)
        };
        let value = if let Drive::Cage { .. } = self.handle.drive {
            let offsets = &self.calls[0]["op"]["lattice"]["offsets"];
            let mut offsets = offsets.as_array().cloned().unwrap_or_default();
            for (h, moves) in &self.dots {
                if let (Drive::Cage { at, by }, true) = (h.drive, *moves) {
                    offsets.retain(|o| o["at"] != json!(at));
                    offsets.push(json!({ "at": at, "by": moved(by, true) }));
                }
            }
            json!(offsets)
        } else {
            let place = |(h, moves): &(DeformerHandle, bool)| match h.drive {
                Drive::Curve { place, .. } => Some(moved(place, *moves)),
                _ => None,
            };
            json!(self.dots.iter().filter_map(place).collect::<Vec<_>>())
        };
        Some((by.len2().sqrt(), value))
    }

    /// Letting go without keeping the drag: an update with the press's values, if one went out.
    pub(super) fn cancel(&self) -> Option<Step> {
        self.sent.as_ref()?;
        let calls = self.calls.iter();
        let back = calls.map(|c| with_args("transform.apply", c.clone()));
        Some(session("update", back.collect()))
    }
}

/// A moved argument as its call writes it.
fn written(moved: Moved) -> Value {
    match moved {
        Moved::Number(v) => json!(num(v)),
        Moved::Vector(v) => json!(v.to_array().map(num)),
    }
}
