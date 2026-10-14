//! Starting a stroke: the samples checked, the stroke aimed at the part (through a camera, or
//! at world points with a view direction) and the driver begun.

use gs_core::brush::StrokeSpec;
use gs_core::camera::Camera;
use gs_core::{MeshStore, PosRead, StrokeParams, View};
use gs_protocol::{CommandError, Length, Vec3};

use super::{BrushStroke, DEFAULT_SIZE, StrokePoint};
use crate::document::Document;
use crate::errors;
use crate::mesh::{resolve_length, store_diagonal};
use crate::sculpt::aim::{Aim, Driver, Radius, Screen};
use crate::sculpt::detail::{self, Refining};
use crate::view::{ViewSpec, Views};

/// The size of a screen stroke whose size is left out, in pixels.
const DEFAULT_PX: f32 = 64.0;
/// The image screen samples are in when the viewport is left out: a `view.snapshot`'s.
const DEFAULT_VIEWPORT: [u32; 2] = [1024, 1024];

impl BrushStroke {
    /// The camera of a screen stroke, resolved for `doc`; `None` for a world stroke.
    pub(crate) fn camera(
        &self,
        doc: &Document,
        views: &Views,
    ) -> Result<Option<(Camera, [f32; 2])>, CommandError> {
        self.camera
            .as_ref()
            .map(|spec| aim_camera(spec, self.viewport, doc, views))
            .transpose()
    }

    /// Starts the stroke on `store`, inside its open step, aimed through `camera` (see
    /// [`BrushStroke::camera`]), or at world samples without one. `levels` says whether the
    /// part has detail levels, which never refine. Returns the driver, the brush's name and
    /// whether the stroke refines.
    pub(crate) fn begin(
        &self,
        store: &mut MeshStore,
        camera: Option<(Camera, [f32; 2])>,
        levels: bool,
    ) -> Result<(Driver, String, Refining), CommandError> {
        let preset = self.resolve()?;
        check_marks(&self.samples, &self.brush, preset.stroke)?;
        let alpha = (preset.tip.alpha.as_ref())
            .map(crate::alpha::pyramid)
            .transpose()?;
        // The brush's width on screen, for a stroke sized in pixels (see `screen`).
        let pixels =
            (camera.is_some() && self.size.is_none()).then(|| self.size_px.unwrap_or(DEFAULT_PX));
        // A screen's up squares Select Rect's rectangle to it (decision 0051, section 4).
        let up = camera.as_ref().map(|(c, _)| c.axes().1);
        let (view, aim) = match camera {
            Some((camera, size)) => self.screen(store, camera, size, &preset)?,
            None => self.world(store)?,
        };
        let refining = match self.live_detail {
            None => Refining::NotAsked,
            Some(_) => {
                detail::refusal(&preset, levels, store).map_or(Refining::Runs, Refining::Refused)
            }
        };
        let live_detail = match (&refining, self.live_detail) {
            (Refining::Runs, Some(ld)) => Some(ld.resolve(pixels)?),
            _ => None,
        };
        let colors = (self.colors.as_ref())
            .map(|c| c.core("/colors"))
            .transpose()?
            .unwrap_or_default();
        let params = StrokeParams {
            alpha,
            invert: self.invert,
            symmetry: self.symmetry.unwrap_or_default().checked()?,
            lock: self.lock.unwrap_or_default().array(),
            live_detail,
            seed: self.seed.unwrap_or_default(),
            colors,
            pen: (self.pen.map(|p| p.core("/pen")).transpose()?).unwrap_or_default(),
            up,
            ..StrokeParams::new(view)
        };
        let driver = Driver::begin(store, &preset, params, aim)
            .map_err(|e| errors::args_invalid("", e.message))?;
        Ok((driver, preset.name, refining))
    }

    /// The view and aim of a stroke through `camera` at an image of `size` pixels.
    fn screen(
        &self,
        store: &MeshStore,
        camera: Camera,
        size: [f32; 2],
        preset: &gs_core::BrushPreset,
    ) -> Result<(View, Aim), CommandError> {
        if self.view_dir.is_some() {
            let message = "a stroke with a camera is seen from the camera";
            return Err(errors::args_invalid("/view_dir", message));
        }
        let radius = match (self.size, self.size_px) {
            (Some(_), Some(_)) => {
                let message = "give size or size_px, not both";
                return Err(errors::args_invalid("/size", message));
            }
            (Some(size), None) => Radius::Units(radius_of(size, store_diagonal(store))?),
            (None, px) => Radius::Pixels(px.unwrap_or(DEFAULT_PX)),
        };
        let screen = Screen::new(camera, size, radius, preset);
        Ok((camera.view(), Aim::Screen(screen)))
    }

    /// The view and aim of a stroke of world samples.
    fn world(&self, store: &MeshStore) -> Result<(View, Aim), CommandError> {
        if self.size_px.is_some() {
            let message = "a size in pixels needs a camera; give size instead";
            return Err(errors::args_invalid("/size_px", message));
        }
        let first = (self.samples.first()).map_or(gs_core::Vec3::ZERO, |s| s.input().at);
        let view = world_view(store, self.view_dir, first)?;
        let radius = radius_of(self.size.unwrap_or(DEFAULT_SIZE), store_diagonal(store))?;
        Ok((view, Aim::World(radius)))
    }
}

/// The exact camera `spec` means for an image of `viewport` pixels (a snapshot's when left
/// out), resolved for `doc` and the session's views, and the image size.
pub(crate) fn aim_camera(
    spec: &ViewSpec,
    viewport: Option<[u32; 2]>,
    doc: &Document,
    views: &Views,
) -> Result<(Camera, [f32; 2]), CommandError> {
    let [w, h] = viewport.unwrap_or(DEFAULT_VIEWPORT);
    if !(1..=16384).contains(&w) || !(1..=16384).contains(&h) {
        let message = "the viewport must be 1 to 16384 pixels each way";
        return Err(errors::args_invalid("/viewport", message));
    }
    let size = [w as f32, h as f32];
    Ok((spec.resolve(doc, views, size[0] / size[1])?, size))
}

/// `E_ARGS_INVALID` at sample `i`'s `field`.
fn bad_sample(i: usize, field: &str, message: impl Into<String>) -> CommandError {
    errors::args_invalid(&format!("/samples/{i}/{field}"), message)
}

/// Every sample has `dims` finite coordinates and finite tilt.
pub(super) fn check_samples(samples: &[StrokePoint], dims: usize) -> Result<(), CommandError> {
    for (i, s) in samples.iter().enumerate() {
        if s.position.len() != dims {
            let what = if dims == 3 {
                "a world point [x, y, z]"
            } else {
                "a point [x, y] on the camera's image"
            };
            return Err(bad_sample(i, "position", format!("needs {what}")));
        }
        if !s.position.iter().all(|x| x.abs() <= 1e18) {
            return Err(bad_sample(i, "position", "coordinates must be finite"));
        }
        if !s.tilt.unwrap_or_default().iter().all(|x| x.is_finite()) {
            return Err(bad_sample(i, "tilt", "tilt must be finite"));
        }
    }
    Ok(())
}

/// No sample of a stroke that runs through no points carries a `point`, and none of a stroke
/// with nothing for Space to move (decision 0062, row 782) carries `moves`, so a log never
/// holds a mark that does nothing (decision 0055, section 4).
fn check_marks(
    samples: &[StrokePoint],
    brush: &str,
    stroke: StrokeSpec,
) -> Result<(), CommandError> {
    let marked = |s: &StrokePoint| {
        let point = (s.point.is_some() && !stroke.takes_points()).then_some("point");
        point.or((s.moves && !stroke.moves()).then_some("moves"))
    };
    match (samples.iter().enumerate()).find_map(|(i, s)| marked(s).map(|f| (i, f))) {
        Some((i, "point")) => {
            let message = format!("{brush}'s stroke runs through no points, so takes no point");
            Err(bad_sample(i, "point", message))
        }
        Some((i, field)) => {
            let message =
                format!("{brush}'s stroke has nothing for Space to move, so takes no {field}");
            Err(bad_sample(i, field, message))
        }
        None => Ok(()),
    }
}

/// A radius from a length: positive and finite, or `E_ARGS_INVALID` at `/size`.
fn radius_of(size: Length, diagonal: f32) -> Result<f32, CommandError> {
    let r = resolve_length(size, diagonal);
    if r > 0.0 && r.is_finite() {
        Ok(r)
    } else {
        let message = format!("the brush size must come to a positive radius, not {r}");
        Err(errors::args_invalid("/size", message))
    }
}

/// The view of a world stroke: as given, or looking straight at the surface nearest `first`.
fn world_view(
    store: &MeshStore,
    given: Option<Vec3>,
    first: gs_core::Vec3,
) -> Result<View, CommandError> {
    // Looking along the surface's normal under the first sample, into it.
    let looking_at = || {
        let near = store.nearest_vertex(first).found;
        -near.map_or(gs_core::Vec3::ZERO, |(v, _)| store.nrm().get(v))
    };
    let dir = given.map_or_else(looking_at, |Vec3(d)| gs_core::Vec3::from_array(d));
    if !(dir.len2() > 0.0 && dir.len2().is_finite()) {
        let message = "needs a finite direction, not zero (the surface under the first \
                       sample gives none: give view_dir)";
        return Err(errors::args_invalid("/view_dir", message));
    }
    Ok(View { dir, eye: None })
}
