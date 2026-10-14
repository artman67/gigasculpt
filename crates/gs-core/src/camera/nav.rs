//! The moves navigation makes (research 14, 7.4): orbit, roll, pan (by the camera or by the lens
//! shift), dolly, advance, recentering, turning to a given orientation, snapping to the axis
//! views or to a step of turn, and levelling. Each one moves the camera rigidly (or
//! scales it) about a point, so that point keeps its place on the screen; each ends with the
//! orientation renormalized, so a long run of moves never drifts from a rotation. A move by
//! nothing leaves the camera as it was, to the bit.

use std::f32::consts::FRAC_PI_2;

use super::{Camera, Projection};
use crate::math::{Quat, Vec3};

/// A scene axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// The unit vector along the axis.
    pub fn unit(self) -> Vec3 {
        match self {
            Axis::X => Vec3::new(1.0, 0.0, 0.0),
            Axis::Y => Vec3::new(0.0, 1.0, 0.0),
            Axis::Z => Vec3::new(0.0, 0.0, 1.0),
        }
    }
}

/// What a horizontal drag turns about. A vertical drag always turns about the screen's
/// horizontal axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrbitMode {
    /// The screen's vertical axis: the view turns relative to the screen (a two-axis
    /// valuator).
    Free,
    /// A scene axis, like a sculpture stand. `Axis(Y)` is a turntable
    /// with Y up, Blender's and Nomad's default.
    Axis(Axis),
}

impl OrbitMode {
    /// The direction that counts as up: the mode's axis, or +Y in free mode.
    pub fn up(self) -> Vec3 {
        match self {
            OrbitMode::Free => Axis::Y.unit(),
            OrbitMode::Axis(axis) => axis.unit(),
        }
    }
}

impl Camera {
    /// Turns the camera rigidly by `r` about `pivot`: the pivot keeps its place on the screen.
    fn turn(&mut self, r: Quat, pivot: Vec3) {
        let target = Vec3::from_array(self.target) - pivot;
        self.target = (pivot + r.rotate(target)).to_array();
        self.orientation = (r * self.orientation).normalized();
    }

    /// Orbits about `pivot`: the camera moves `right` radians to its right (the model seems to
    /// turn left) about `mode`'s axis, then `up` radians upward about the screen's horizontal
    /// axis (it looks more from above).
    pub fn orbit(&mut self, pivot: Vec3, mode: OrbitMode, right: f32, up: f32) {
        let around = match mode {
            OrbitMode::Free => self.axes().1,
            OrbitMode::Axis(axis) => axis.unit(),
        };
        if right != 0.0 {
            self.turn(Quat::about(around, right), pivot);
        }
        if up != 0.0 {
            let across = self.axes().0;
            self.turn(Quat::about(across, -up), pivot);
        }
    }

    /// The camera is upside down for `mode`: its up points below the horizon of the mode's
    /// axis. A horizontal drag that starts so turns the other way, so the model follows the
    /// pen (Blender's rule). Never in free mode, which turns about the screen's own axes.
    pub fn upside_down(&self, mode: OrbitMode) -> bool {
        match mode {
            OrbitMode::Free => false,
            OrbitMode::Axis(axis) => self.axes().1.dot(axis.unit()) < 0.0,
        }
    }

    /// This camera's orientation with the lean taken away: turned about the view direction
    /// until the screen's horizontal axis is level with `up` and the picture is upright,
    /// keeping where it looks. `None` when it is level already, or looks straight along `up`
    /// (no direction is level then).
    pub fn leveled(&self, up: Vec3) -> Option<Quat> {
        let (right, now, forward) = self.axes();
        let back = -forward;
        let upright = up - back * up.dot(back);
        if upright.len2() <= 1e-12 || (right.dot(up).abs() <= 1e-6 && now.dot(up) > 0.0) {
            return None;
        }
        let level = upright.normalized();
        Some(Quat::from_axes(level.cross(back), level, back))
    }

    /// Rolls about the view direction through `pivot`: positive turns the picture
    /// counterclockwise.
    pub fn roll(&mut self, pivot: Vec3, angle: f32) {
        if angle == 0.0 {
            return;
        }
        let forward = self.axes().2;
        self.turn(Quat::about(forward, angle), pivot);
    }

    /// Slides the camera parallel to the screen so `pivot`'s image moves `dx` pixels right and
    /// `dy` pixels down on an image `size` pixels wide and high. A pivot behind the eye moves
    /// at the target's scale.
    pub fn pan(&mut self, pivot: Vec3, dx: f32, dy: f32, size: [f32; 2]) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let (right, up, _) = self.axes();
        let depth = self.depth(pivot);
        let depth = if depth > 0.0 { depth } else { self.distance };
        let s = self.pixel_size(depth, size);
        let shift = right * (-dx * s) + up * (dy * s);
        self.target = (Vec3::from_array(self.target) + shift).to_array();
    }

    /// Slides the picture `dx` pixels right and `dy` pixels down on an image `size` pixels wide
    /// and high by the lens shift, the eye staying where it is: every point moves the same, so
    /// the model is seen from the same direction wherever it sits on the screen.
    pub fn pan_lens(&mut self, dx: f32, dy: f32, size: [f32; 2]) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let [x, y] = self.lens.shift;
        self.lens.shift = [x + dx / size[1], y - dy / size[1]];
    }

    /// Scales the camera about `about` by `factor`: below 1 it moves closer, and `about` keeps
    /// its place on the screen. An orthographic camera shows `factor` times the height.
    pub fn dolly(&mut self, about: Vec3, factor: f32) {
        // Exactly 1, not near it: only a move by nothing leaves the camera's bits alone.
        if (factor - 1.0) == 0.0 {
            return;
        }
        let target = Vec3::from_array(self.target) - about;
        self.target = (about + target * factor).to_array();
        self.distance *= factor;
    }

    /// Moves the eye `1 - factor` of the camera's distance along the line from it through
    /// `about`, keeping the distance: `about` keeps its place on the screen and the eye may pass
    /// through the model. An orthographic camera, whose picture the eye's depth does not
    /// change, scales as [`Camera::dolly`] does.
    pub fn advance(&mut self, about: Vec3, factor: f32) {
        if (factor - 1.0) == 0.0 {
            return;
        }
        if self.projection == Projection::Orthographic {
            self.dolly(about, factor);
            return;
        }
        let toward = about - Vec3::from_array(self.eye());
        let length = toward.len2().sqrt();
        let along = if length > 0.0 {
            toward * (1.0 / length)
        } else {
            self.axes().2
        };
        let step = along * ((1.0 - factor) * self.distance);
        self.target = (Vec3::from_array(self.target) + step).to_array();
    }

    /// After a turn of `turn` radians, slides the camera parallel to the screen part of the way
    /// that puts `pivot` on the view axis (the middle of the picture, with no lens shift):
    /// `rate` of the way per quarter turn, compounded, so many small turns take it as far as one
    /// large one. Rate 0 leaves the camera; rate 1 centres the pivot at any turn.
    pub fn recenter(&mut self, pivot: Vec3, rate: f32, turn: f32) {
        if rate == 0.0 || turn == 0.0 {
            return;
        }
        let k = 1.0 - libm::powf(1.0 - rate, turn.abs() / FRAC_PI_2);
        let (right, up, _) = self.axes();
        let off = pivot - Vec3::from_array(self.target);
        let slide = right * (off.dot(right) * k) + up * (off.dot(up) * k);
        self.target = (Vec3::from_array(self.target) + slide).to_array();
    }

    /// Turns the camera about `pivot` until its orientation is `to`.
    pub fn turn_to(&mut self, to: Quat, pivot: Vec3) {
        let r = to * self.orientation.inverse();
        let target = Vec3::from_array(self.target) - pivot;
        self.target = (pivot + r.rotate(target)).to_array();
        self.orientation = to.normalized();
    }

    /// The nearest of the six axis views to this camera's view, with a level horizon: up is +Y
    /// looking sideways; looking straight up or down, up is whichever horizontal axis is
    /// nearest the camera's up now.
    pub fn snapped(&self) -> Quat {
        let (_, up, forward) = self.axes();
        let along = nearest_axis(forward, None);
        let level = if along.y == 0.0 {
            Vec3::new(0.0, 1.0, 0.0)
        } else {
            nearest_axis(up, Some(Axis::Y))
        };
        let back = -along;
        Quat::from_axes(level.cross(back), level, back)
    }

    /// The nearest view whose azimuth and elevation ([`Camera::turned`]) are whole multiples of
    /// `step` radians, with a level horizon; looking straight up or down, the azimuth is the
    /// multiple nearest the way the camera's up points. A quarter turn or more is
    /// [`Camera::snapped`], to the bit.
    pub fn snapped_by(&self, step: f32) -> Quat {
        if step >= FRAC_PI_2 * (1.0 - 1e-6) {
            return self.snapped();
        }
        let (_, up, forward) = self.axes();
        let back = -forward;
        let azimuth = libm::atan2f(back.x, back.z);
        let elevation = libm::asinf(back.y.clamp(-1.0, 1.0));
        let multiples = |a: f32| [(a / step).floor() * step, (a / step).ceil() * step];
        let near = multiples(elevation)
            .map(|e| e.clamp(-FRAC_PI_2, FRAC_PI_2))
            .into_iter()
            .flat_map(|e| multiples(azimuth).map(|a| (a, e)));
        let toward = |(a, e): (f32, f32)| {
            let (sa, ca) = (libm::sinf(a), libm::cosf(a));
            let (se, ce) = (libm::sinf(e), libm::cosf(e));
            back.dot(Vec3::new(sa * ce, se, ca * ce))
        };
        let best = near.fold((f32::NEG_INFINITY, (0.0, 0.0)), |best, c| {
            if toward(c) > best.0 {
                (toward(c), c)
            } else {
                best
            }
        });
        let (mut azimuth, elevation) = best.1;
        if elevation.abs() >= FRAC_PI_2 * (1.0 - 1e-6) {
            // At a pole the turn's up is -(sin a, 0, cos a) looking down, +(..) looking up.
            let s = -elevation.signum();
            let heading = libm::atan2f(s * up.x, s * up.z);
            azimuth = (heading / step).round() * step;
        }
        Camera::turned(azimuth, elevation)
    }
}

/// The signed axis vector nearest `v`, leaving out `skip`.
fn nearest_axis(v: Vec3, skip: Option<Axis>) -> Vec3 {
    let axes = [Axis::X, Axis::Y, Axis::Z]
        .into_iter()
        .filter(|a| Some(*a) != skip);
    let best = axes
        .flat_map(|a| [a.unit(), -a.unit()])
        .map(|d| (d.dot(v), d))
        .fold((f32::NEG_INFINITY, Vec3::ZERO), |a, b| {
            if b.0 > a.0 { b } else { a }
        });
    best.1
}
