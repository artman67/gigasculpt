//! The window's pen stroke. It begins when the pen touches the 3D view, takes each pen sample
//! as it comes, through the same [`Driver`] `brush.stroke` uses, so the part changes under the
//! pen, and ends when the pen lifts. The engine then commits it as the `brush.stroke` call that
//! replays it: `main`'s camera in the exact form, the viewport size, the session's patch for the
//! brush, the pinned preset and alpha hashes and every sample, so the log line gives back the
//! same mesh bit for bit.
//!
//! The part is the active one (decision 0033): the samples' rays are tested against it alone.
//! Once the part is known the samples are fed in order, as a replay feeds them. A press that
//! misses it, when the window asks for the classic navigation, is no stroke:
//! the window turns the view with it instead (decision 0017). A gesture needs no surface under
//! its press, so it starts wherever the pen presses (decision 0017 as amended 2026-10-12, item
//! 17).
//!
//! Alt can change mid-stroke: [`PenStroke::Alt`] sets the sign of the samples after it, and the
//! log line keeps a sample's `invert` only where it differs from the stroke's (decision 0036).
//! A stroke that runs through points takes Alt taps as points and Space as moves, and Drag
//! Rect takes Space as moves, which the log line marks on its samples (decision 0055; 0062,
//! row 782).

use gs_core::NodeId;
use gs_core::brush::StrokeSpec;
use gs_core::camera::Camera;
use gs_protocol::{CommandError, PenSample};

use super::BrushSettings;
use super::aim::Driver;
use super::detail::Refining;
use super::stroke::{BrushStroke, PathPoint, StrokePoint, Stroked, outcome};
use crate::Origin;
use crate::document::{Document, Part};
use crate::view::{ExactCamera, ViewSpec};

/// How a pen stroke starts, as the window saw it. It is drawn through `main`'s camera.
#[derive(Clone, Debug)]
pub struct PenDown {
    /// The 3D view's area: left, top, width and height in window pixels.
    pub viewport: [u32; 4],
    /// A press on empty canvas turns the view instead of sculpting (the classic
    /// navigation): the window asks for it unless it is switched off or `main` is locked.
    pub classic: bool,
    /// A brush to use instead of the session's (a held modifier picks Smooth or Mask Pen).
    pub brush: Option<String>,
    /// Alt is held: work the other way.
    pub invert: bool,
}

/// What the window's pen did to the 3D view, in order.
#[derive(Clone, Debug)]
pub enum PenStroke {
    /// The pen touched the view.
    Down(PenDown),
    /// A sample while it touches, in window pixels.
    Sample(PenSample),
    /// It lifted, or left range.
    Up,
    /// Alt was pressed (`true`) or released while the pen touches: the samples after it work
    /// that way (decision 0036).
    Alt(bool),
    /// An Alt tap during a stroke that runs through points (Clay Line, Clip Curve): the latest
    /// sample is a soft point of its path (decision 0055).
    Point,
    /// A quick second Alt tap: the latest point is a corner instead.
    Corner,
    /// Space was pressed (`true`) or released during a stroke that runs through points: the
    /// samples after it move the whole path instead of extending it.
    Space(bool),
}

/// A pen stroke in progress.
pub(crate) struct Live {
    pub origin: Origin,
    /// The call it will be: settings, camera, viewport, and the samples so far.
    args: BrushStroke,
    /// The view's top-left corner, in window pixels.
    corner: [f64; 2],
    camera: (Camera, [f32; 2]),
    /// The part and the stroke on it, once a sample has hit a part: its name, the driver, the
    /// brush's name and whether it refines.
    run: Option<(String, Driver, String, Refining)>,
    /// Why the stroke cannot go on, if something refused it.
    pub problem: Option<CommandError>,
    /// What a press that misses every part is.
    miss: Miss,
    /// The press was on empty canvas (`miss`): no stroke, the window navigates.
    pub on_canvas: bool,
    /// The title of the job holding the scene as the pen went down: a press on a part sculpts
    /// nothing, and one on empty canvas navigates (decision 0010 as amended 2026-10-13).
    pub held: Option<&'static str>,
    /// Alt now, which each sample takes.
    alt: bool,
    /// Space now, while each sample moves the path or Drag Rect's rectangle; `None` when the
    /// brush's stroke has nothing Space moves ([`StrokeSpec::moves`]).
    space: Option<bool>,
    /// The brush's stroke runs through points, so takes Alt taps as points and corners.
    points: bool,
    /// The sample the latest point sits on, which a corner marks.
    last_point: Option<usize>,
}

/// What a press that misses every part is (decision 0017).
#[derive(Clone, Copy)]
enum Miss {
    /// A stroke all the same (classic navigation off): it takes the only part there is.
    Stroke,
    /// A press on empty canvas, which the window navigates with (classic navigation on).
    Canvas,
    /// A gesture's press with classic navigation on: a gesture needs no surface under its
    /// press, so it is a press on empty canvas only in a scene with no part to take (decision
    /// 0017 as amended 2026-10-12, items 17 and 18).
    Gesture,
}

/// A finished pen stroke, ready to commit as a `brush.stroke` call.
pub(crate) struct Ended {
    pub origin: Origin,
    pub part: String,
    pub args: BrushStroke,
    pub output: Stroked,
    pub why: String,
    pub written: Vec<NodeId>,
    /// Where the stroke first touched, if it changed anything.
    pub first: Option<[f32; 3]>,
}

impl Live {
    /// A stroke as the window began it through `camera` (`main`'s), with the session's
    /// settings and, for a brush that paints color, the session's colors and `seed` (the stream
    /// of the command it will log as).
    pub fn begin(
        origin: &Origin,
        down: PenDown,
        settings: &BrushSettings,
        camera: Camera,
        seed: u64,
    ) -> Result<Self, CommandError> {
        let state = settings.state();
        let brush = down.brush.unwrap_or(state.brush);
        let shipped = super::presets::find(&brush)?;
        let name = &shipped.preset.name;
        let [x, y, w, h] = down.viewport;
        let mut args = BrushStroke::new(name, Vec::new());
        args.camera = Some(ViewSpec::Exact(ExactCamera::from(camera)));
        args.viewport = Some([w, h]);
        args.size = state.size;
        args.size_px = state.size_px;
        // What the session changed for the brush; the preset's hash pins the rest.
        let patch = settings.patch_of(name);
        args.settings = (!patch.is_empty()).then_some(patch);
        args.invert = down.invert;
        args.symmetry = Some(state.symmetry);
        args.live_detail = settings.detail.pen();
        args.pin_brush(settings)?;
        args.pin_paint(seed, &settings.colors)?;
        let stroke = args.resolve().ok().map(|p| p.stroke);
        let space = stroke.is_some_and(|s| s.moves()).then_some(false);
        let miss = match (down.classic, stroke) {
            (false, _) => Miss::Stroke,
            (true, Some(StrokeSpec::Gesture { .. })) => Miss::Gesture,
            (true, _) => Miss::Canvas,
        };
        Ok(Self {
            origin: origin.clone(),
            args,
            corner: [f64::from(x), f64::from(y)],
            camera: (camera, [w.max(1) as f32, h.max(1) as f32]),
            run: None,
            problem: None,
            miss,
            on_canvas: false,
            held: None,
            alt: down.invert,
            space,
            points: stroke.is_some_and(|s| s.takes_points()),
            last_point: None,
        })
    }

    /// A key during the stroke: Alt sets the sign of the samples from now on; for a stroke that
    /// runs through points, a tap marks a point on the latest sample, a quick second tap makes
    /// that point a corner, and Space makes the samples from now on move the path (or Drag
    /// Rect's rectangle). A stroke that takes no points drops the taps, and one with nothing to
    /// move drops Space.
    pub fn key(&mut self, key: &PenStroke) {
        match *key {
            PenStroke::Alt(held) => self.alt = held,
            PenStroke::Space(held) => self.space = self.space.map(|_| held),
            PenStroke::Point if self.points => self.mark(false),
            PenStroke::Corner if self.points => self.mark(true),
            _ => {}
        }
    }

    /// Marks the latest sample a soft point, or (`corner`) the latest point's sample sharp, in
    /// the log line and in the stroke, which a replay does at that sample in the same order.
    fn mark(&mut self, corner: bool) {
        let at = if corner {
            self.last_point
        } else {
            self.args.samples.len().checked_sub(1)
        };
        let Some(sample) = at.and_then(|i| self.args.samples.get_mut(i)) else {
            return;
        };
        if corner {
            sample.point = Some(PathPoint::Sharp);
        } else {
            sample.point.get_or_insert(PathPoint::Soft);
            self.last_point = at;
        }
        if let Some((_, driver, ..)) = &mut self.run {
            if corner {
                driver.corner();
            } else {
                driver.point();
            }
        }
    }

    /// Takes one sample.
    pub fn feed(&mut self, doc: &mut Document, s: &PenSample) {
        if self.problem.is_some() || self.on_canvas {
            return;
        }
        let point = StrokePoint {
            position: vec![(s.x - self.corner[0]) as f32, (s.y - self.corner[1]) as f32],
            pressure: Some(s.pressure),
            tilt: (s.tilt_x != 0.0 || s.tilt_y != 0.0).then_some([s.tilt_x, s.tilt_y]),
            t_ns: Some(s.t_ns),
            // Written only where Alt differs from the stroke's start, so a stroke where it
            // never changed logs as it always has.
            invert: (self.alt != self.args.invert).then_some(self.alt),
            point: None,
            moves: self.space == Some(true),
        };
        self.args.samples.push(point);
        if self.run.is_none() {
            self.choose(doc);
            return;
        }
        let input = self.args.samples[self.args.samples.len() - 1].input();
        if let Some((part, driver, ..)) = &mut self.run
            && let Some(store) = doc.get_mut(part).map(Part::store_mut)
            && let Err(e) = driver.feed(store, input)
        {
            self.problem = Some(fed(e));
        }
    }

    /// Picks the part the stroke sculpts, opens its step, and feeds it every sample so far. It is
    /// the active part (decision 0033, section 4): the newest sample's ray is tested against it
    /// alone, so a part in front of it neither takes the stroke nor stops it, and a press that
    /// misses it is a press on empty canvas, except for a gesture, which takes it all the same.
    /// With no active part, the part that shows nearest along the ray (or the only one).
    fn choose(&mut self, doc: &mut Document) {
        let Some(last) = self.args.samples.last() else {
            return;
        };
        let (camera, size) = self.camera;
        let at = last.input().at;
        let (o, d) = camera.ray(at.x, at.y, size);
        let names: Vec<String> = match doc.active() {
            Some(active) => vec![active.to_string()],
            None => doc.shown().map(|(name, _)| name.clone()).collect(),
        };
        let hit = (names.iter())
            .filter_map(|n| {
                let t = doc.get(n)?.hit(o, d)?.0;
                Some((t, n.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let press = self.args.samples.len() == 1;
        let canvas = match self.miss {
            Miss::Stroke => false,
            Miss::Canvas => true,
            Miss::Gesture => names.is_empty(),
        };
        let part = match (hit, names.as_slice()) {
            (Some((_, part)), _) => part,
            (None, _) if press && canvas => {
                self.on_canvas = true;
                return;
            }
            (None, [only]) => only.clone(),
            (None, _) => return,
        };
        if let Some(job) = self.held {
            self.problem = Some(crate::job::busy(job));
            return;
        }
        let levels = doc
            .get(&part)
            .is_some_and(|p| p.levels().is_some() || p.frozen().is_some());
        let Some((args, camera)) = doc
            .get(&part)
            .map(|p| self.args.in_part(p, Some(self.camera)))
        else {
            return;
        };
        let Some(store) = doc.get_mut(&part).map(Part::store_mut) else {
            return;
        };
        store.begin_step();
        match args.begin(store, camera, levels) {
            Ok((mut driver, brush, refining)) => {
                for s in &self.args.samples {
                    if let Err(e) = driver.feed(store, s.input()) {
                        self.problem = Some(fed(e));
                        break;
                    }
                }
                self.args.part = Some(gs_protocol::PartRef(part.clone()));
                self.run = Some((part, driver, brush, refining));
            }
            Err(e) => {
                store.cancel_step();
                self.problem = Some(e);
            }
        }
    }

    /// Ends the stroke. `None` if it never reached a part or something refused it (then the
    /// part is back as it was).
    pub fn end(self, doc: &mut Document) -> Option<Ended> {
        let (part, driver, brush, refining) = self.run?;
        let t = doc.get(&part)?.transform();
        let store = doc.get_mut(&part)?.store_mut();
        if self.problem.is_some() {
            store.cancel_step();
            return None;
        }
        let done = driver.finish(store).in_scene(&t);
        let (output, why) = outcome(&part, brush, &done, &refining);
        let touched = !done.report.nodes.is_empty();
        Some(Ended {
            origin: self.origin,
            part,
            args: self.args,
            output,
            why,
            written: done.report.nodes,
            first: done.first.filter(|_| touched).map(gs_core::Vec3::to_array),
        })
    }
}

/// A sample the stroke could not take, refused as `brush.stroke` refuses it (`run_samples`).
fn fed(e: gs_core::StrokeError) -> CommandError {
    crate::errors::args_invalid("/samples", e.message)
}
