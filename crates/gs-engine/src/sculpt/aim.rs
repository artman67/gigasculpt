//! From samples to the stroke pipeline: world samples go in as given; screen samples are aimed
//! through the camera, one at a time, at the mesh as the stroke has left it so far. `brush.stroke`
//! and the window's pen both feed a [`Driver`] sample by sample, so the same samples give the
//! same mesh whichever path they take.
//!
//! How a screen sample becomes a point depends on the stroke class:
//! - dab and curve strokes hit the surface under the sample; a sample whose ray misses the
//!   part is left out;
//! - drag strokes, Drag Rect (anchored) and hook brushes grab the surface at the first sample
//!   that hits it; later samples land on the plane through the grab that faces the camera, so
//!   the point never chases the surface it moves. A hook's plane turns toward the one that holds
//!   the start normal as its direction and Brush Modifier say (decision 0049, section 7);
//! - a Drag Dot stroke (anchored, `follow`) takes its stamp back before each sample and hits
//!   the surface under it as the stroke found it, so the stamp follows the pointer over the
//!   surface and commits what one press there makes; a sample that misses lays the last stamp
//!   again (decision 0051, section 5);
//! - gestures land on the plane facing the camera through the point it orbits. A gesture
//!   sweeps along the view rays, so which plane makes no difference to what it does.
//!
//! A size in pixels becomes a radius at the depth of each point, so the brush keeps its size
//! on screen.

use gs_core::brush::{KernelSpec, StrokeSpec};
use gs_core::camera::Camera;
use gs_core::{BrushPreset, MeshStore, Stroke, StrokeError, StrokeParams, StrokeReport};
use gs_core::{StrokeSample, Vec3};

/// The most missed samples kept for saying how far a stroke that never hit passed.
const KEPT_MISSES: usize = 256;

/// One sample: a world point, or a point on the screen (only `x` and `y` used).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Input {
    pub at: Vec3,
    pub pressure: f32,
    pub tilt: [f32; 2],
    /// Alt held at this sample; `None`, as at the stroke's start.
    pub invert: Option<bool>,
    /// A point of the path on this sample, and whether it is sharp (decision 0055).
    pub point: Option<bool>,
    /// Space held: the sample moves the path.
    pub moves: bool,
}

/// The brush radius of screen samples.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Radius {
    /// On screen, at the depth of each point.
    Pixels(f32),
    /// In scene units.
    Units(f32),
}

/// How samples become points.
pub(crate) enum Aim {
    /// World points, with this radius.
    World(f32),
    /// Points on the screen.
    Screen(Screen),
}

/// Aiming screen samples.
pub(crate) struct Screen {
    camera: Camera,
    /// The image the samples are in, in pixels.
    size: [f32; 2],
    radius: Radius,
    mode: Mode,
    /// Where the stroke grabbed the surface, and the radius there.
    grab: Option<(Vec3, f32)>,
    /// A hook brush's preset, which says the plane it pulls in.
    hook: Option<Box<BrushPreset>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Surface,
    Grab,
    /// Drag Dot: the surface, with the stroke's stamp taken back first.
    Follow,
    Plane,
}

impl Screen {
    /// Aiming through `camera` at an image of `size` pixels, for `preset`.
    pub fn new(camera: Camera, size: [f32; 2], radius: Radius, preset: &BrushPreset) -> Self {
        let mode = match (&preset.stroke, &preset.kernel) {
            (StrokeSpec::Gesture { .. }, _) => Mode::Plane,
            (StrokeSpec::Anchored { follow: true, .. }, _) => Mode::Follow,
            (StrokeSpec::Drag { .. } | StrokeSpec::Anchored { .. }, _)
            | (_, KernelSpec::Hook(_)) => Mode::Grab,
            _ => Mode::Surface,
        };
        Self {
            camera,
            size,
            radius,
            mode,
            grab: None,
            hook: matches!(preset.kernel, KernelSpec::Hook(_)).then(|| Box::new(preset.clone())),
        }
    }

    /// The radius at a point.
    fn radius_at(&self, p: Vec3) -> f32 {
        match self.radius {
            Radius::Pixels(px) => px * self.camera.pixel_size(self.camera.depth(p), self.size),
            Radius::Units(r) => r,
        }
    }

    /// Where the ray through `x`, `y` meets the plane through `on` that faces the camera. A
    /// camera ray always runs forward, so it always meets that plane.
    fn on_plane(&self, x: f32, y: f32, on: Vec3) -> Vec3 {
        let (o, d) = self.camera.ray(x, y, self.size);
        let f = self.camera.view().dir;
        o + d * ((on - o).dot(f) / d.dot(f))
    }

    /// Where the ray through `x`, `y` meets the plane through the grab `g` that a hook pulls in
    /// at `pressure`, from the stroke's start normal `start` ([`BrushPreset::pull_plane`]); the
    /// plane facing the camera where there is no other, or the ray does not meet it ahead.
    fn pulled(&self, x: f32, y: f32, g: Vec3, pressure: f32, start: Option<Vec3>) -> Vec3 {
        let view = self.camera.view().dir;
        let plane = (self.hook.as_ref()).and_then(|h| h.pull_plane(view, pressure, start));
        let Some(n) = plane else {
            return self.on_plane(x, y, g);
        };
        let (o, d) = self.camera.ray(x, y, self.size);
        let t = (g - o).dot(n) / d.dot(n);
        if t.is_finite() && t > 0.0 {
            o + d * t
        } else {
            self.on_plane(x, y, g)
        }
    }

    /// The point and radius for a sample at `pressure`, or the ray that missed. `start` is the
    /// stroke's start normal, once its first dab ran.
    fn point(
        &mut self,
        store: &MeshStore,
        (x, y): (f32, f32),
        pressure: f32,
        start: Option<Vec3>,
    ) -> Result<(Vec3, f32), (Vec3, Vec3)> {
        match (self.mode, self.grab) {
            (Mode::Plane, _) => {
                let p = self.on_plane(x, y, Vec3::from_array(self.camera.target));
                Ok((p, self.radius_at(p)))
            }
            (Mode::Grab, Some((g, r))) => Ok((self.pulled(x, y, g, pressure, start), r)),
            (mode, _) => {
                let (o, d) = self.camera.ray(x, y, self.size);
                let hit = store.ray_cast(o, d).hit.ok_or((o, d))?;
                let r = self.radius_at(hit.pos);
                if mode == Mode::Grab {
                    self.grab = Some((hit.pos, r));
                }
                Ok((hit.pos, r))
            }
        }
    }
}

/// A stroke on one part, fed one sample at a time.
pub(crate) struct Driver {
    stroke: Stroke,
    aim: Aim,
    used: u64,
    missed: u64,
    /// Rays of missed screen samples, or world samples, kept to say how far off they were.
    misses: Vec<(Vec3, Option<Vec3>)>,
    /// The radius of the first sample used.
    radius: Option<f32>,
    /// Where the first sample used landed, for a stroke whose samples touch the surface.
    first: Option<Vec3>,
    /// A gesture's samples outline a region (on the camera plane, for screen samples); they
    /// do not touch the surface, so they leave no point (review 0003 m1).
    outline: bool,
    /// Alt at the stroke's start, which a sample without its own takes.
    invert: bool,
}

/// What a finished stroke did.
pub(crate) struct Finished {
    pub report: StrokeReport,
    pub used: u64,
    pub missed: u64,
    pub radius: Option<f32>,
    /// Where the first sample used landed: where the stroke first touched. None for a
    /// gesture, which outlines a region instead.
    pub first: Option<Vec3>,
    /// When the stroke wrote nothing: how far its nearest sample was from the surface.
    pub gap: Option<f32>,
}

impl Driver {
    /// Starts a stroke on `store` (inside its open step).
    pub fn begin(
        store: &mut MeshStore,
        preset: &BrushPreset,
        params: StrokeParams,
        aim: Aim,
    ) -> Result<Self, StrokeError> {
        let invert = params.invert;
        Ok(Self {
            stroke: Stroke::begin(store, preset, params)?,
            invert,
            aim,
            used: 0,
            missed: 0,
            misses: Vec::new(),
            radius: None,
            first: None,
            outline: matches!(preset.stroke, StrokeSpec::Gesture { .. }),
        })
    }

    /// Aims one sample and feeds it to the stroke, moving its path with Space, then marks the
    /// point on it, if any (decision 0055). A sample that misses still marks the point, on the
    /// latest sample held, as a replay does.
    pub fn feed(&mut self, store: &mut MeshStore, input: Input) -> Result<(), StrokeError> {
        self.stroke.set_moving(input.moves);
        self.aim_and_add(store, input)?;
        if let Some(sharp) = input.point {
            self.point();
            if sharp {
                self.corner();
            }
        }
        Ok(())
    }

    /// An Alt tap: the latest sample held is a point of the path.
    pub fn point(&mut self) {
        self.stroke.point();
    }

    /// A quick second tap: the latest point is a corner.
    pub fn corner(&mut self) {
        self.stroke.corner();
    }

    fn aim_and_add(&mut self, store: &mut MeshStore, input: Input) -> Result<(), StrokeError> {
        let follow = matches!(&self.aim, Aim::Screen(s) if s.mode == Mode::Follow);
        let start = self.stroke.start_normal();
        let point = match &mut self.aim {
            Aim::World(r) => {
                if self.misses.len() != KEPT_MISSES {
                    self.misses.push((input.at, None));
                }
                Ok((input.at, *r))
            }
            Aim::Screen(screen) => {
                if follow {
                    self.stroke.rewind(store);
                }
                screen.point(store, (input.at.x, input.at.y), input.pressure, start)
            }
        };
        let (pos, radius) = match point {
            Ok(point) => point,
            Err((o, d)) => {
                self.missed += 1;
                if self.misses.len() != KEPT_MISSES {
                    self.misses.push((o, Some(d)));
                }
                if follow {
                    self.stroke.relay(store);
                }
                return Ok(());
            }
        };
        self.used += 1;
        self.radius.get_or_insert(radius);
        if !self.outline {
            self.first.get_or_insert(pos);
        }
        let sample = StrokeSample {
            pos,
            pressure: input.pressure,
            tilt: input.tilt,
            radius,
        };
        self.stroke.set_invert(input.invert.unwrap_or(self.invert));
        self.stroke.add(store, &[sample])
    }

    /// Ends the stroke.
    pub fn finish(self, store: &mut MeshStore) -> Finished {
        let report = self.stroke.finish(store);
        let gap = report
            .nodes
            .is_empty()
            .then(|| nearest(store, &self.misses))
            .flatten();
        Finished {
            report,
            used: self.used,
            missed: self.missed,
            radius: self.radius,
            first: self.first,
            gap,
        }
    }
}

/// How far the nearest of `samples` (world points, or rays) passed from the surface.
fn nearest(store: &MeshStore, samples: &[(Vec3, Option<Vec3>)]) -> Option<f32> {
    (samples.iter())
        .filter_map(|&(p, ray)| {
            let near = match ray {
                Some(d) => store.nearest_to_ray(p, d),
                None => store.nearest_vertex(p),
            };
            near.found.map(|f| f.1)
        })
        .min_by(f32::total_cmp)
}
