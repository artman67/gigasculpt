//! Secondary views (decision 0020): a view with a scene of its own and a link to `main`. The
//! Reference View (`ref:<n>`) is one. Its camera is its own, with each linked quantity taken
//! from `main` as the reference sees it through its alignment, worked out whenever it is read
//! ([`Secondary::camera`]). That is a pure function of `main`'s camera and the reference's
//! setup, both set by logged commands, so a replay gives the same bits and `main`'s moves need
//! no log line for it. This module is the one home of the link and alignment maths.

use std::sync::Arc;

use gs_core::camera::Camera;
use gs_core::{Aabb, Quat, Vec3};
use gs_protocol::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Unavailable;
use crate::document::Document;
use crate::errors;
use crate::file::{Format, UpAxis};

/// What a secondary view shows.
#[derive(Clone)]
pub(crate) enum Scene {
    /// The session's own scene, as it changes.
    Live,
    /// A document of its own, never part of the session's: a file read in, or frozen copies of
    /// parts. Shared by reference count, so a job's copy of the session costs nothing.
    Own(Arc<Document>),
}

impl std::fmt::Debug for Scene {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scene::Live => f.write_str("Live"),
            Scene::Own(doc) => write!(f, "Own({} parts)", doc.len()),
        }
    }
}

impl Scene {
    /// The document it shows: its own, or `live`, the session's.
    pub fn doc<'a>(&'a self, live: &'a Document) -> &'a Document {
        match self {
            Scene::Live => live,
            Scene::Own(doc) => doc,
        }
    }
}

/// Which parts of a reference view's camera follow `main`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Link {
    /// It turns with main. Left out: true.
    #[serde(default = "crate::command::yes")]
    pub rotation: bool,
    /// Its distance, lens and projection follow main's, scaled by the alignment. Left out:
    /// false, so the reference stays framed whole while main works close.
    #[serde(default)]
    pub zoom: bool,
    /// The point it looks at follows main's, through the alignment. Left out: false.
    #[serde(default)]
    pub pan: bool,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            rotation: true,
            zoom: false,
            pan: false,
        }
    }
}

/// Where the reference sits in the sculpt's space: a reference point `p` shows where the sculpt
/// has `scale * turn(p) + offset`. It never changes the reference's meshes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alignment {
    /// A unit quaternion `[x, y, z, w]`; `[0, 0, 0, 1]` leaves the reference as it is.
    pub turn: [f32; 4],
    /// Above 0.
    pub scale: f32,
    /// In the sculpt's units.
    pub offset: [f32; 3],
}

impl Alignment {
    /// The reference as it is.
    pub const IDENTITY: Alignment = Alignment {
        turn: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        offset: [0.0; 3],
    };

    /// The alignment under which a reference seen with orientation `seen` shows the side
    /// `main` shows, with the reference's box `theirs` put on the scene's box `ours`, centre on
    /// centre and diagonal on diagonal (scale 1 and no offset when either box is empty).
    pub fn matching(main: Quat, seen: Quat, ours: Aabb, theirs: Aabb) -> Alignment {
        let turn = (main * seen.inverse()).normalized();
        let diagonal = |b: Aabb| (b.hi - b.lo).len2().sqrt();
        let empty = ours == Aabb::EMPTY || theirs == Aabb::EMPTY;
        if empty || !(diagonal(theirs) > 0.0 && diagonal(ours) > 0.0) {
            return Alignment {
                turn: turn.to_array(),
                ..Alignment::IDENTITY
            };
        }
        let scale = diagonal(ours) / diagonal(theirs);
        let centre = |b: Aabb| (b.lo + b.hi) * 0.5;
        let offset = centre(ours) - turn.rotate(centre(theirs)) * scale;
        Alignment {
            turn: turn.to_array(),
            scale,
            offset: offset.to_array(),
        }
    }

    /// The turn as a quaternion.
    pub fn quat(&self) -> Quat {
        Quat::from_array(self.turn)
    }

    /// Why these are not an alignment, if they are not: a turn that is not a rotation, a scale
    /// that is not above 0, numbers that are not finite.
    pub fn check(&self, path: &str) -> Result<(), CommandError> {
        let q = self.turn;
        let n: f32 = q.iter().map(|v| v * v).sum();
        if !(n - 1.0).abs().le(&1e-3) {
            let message = "the turn must be a unit quaternion [x, y, z, w]";
            return Err(errors::args_invalid(&format!("{path}/turn"), message));
        }
        if !(self.scale > 0.0 && self.scale.is_finite()) {
            let message = "the scale must be a number above 0";
            return Err(errors::args_invalid(&format!("{path}/scale"), message));
        }
        if !self.offset.iter().all(|v| v.is_finite()) {
            let message = "the offset must be finite";
            return Err(errors::args_invalid(&format!("{path}/offset"), message));
        }
        Ok(())
    }

    /// `main`'s camera as the reference sees it: the same picture, with the reference placed in
    /// the scene by this alignment. The turn is not renormalized: a product of two unit turns
    /// is unit to a rounding, and nothing builds on it (it is worked out afresh from `main`
    /// whenever it is read), so a reference that is not aligned turns with exactly `main`'s
    /// numbers.
    pub fn follow(&self, main: &Camera) -> Camera {
        let back = self.quat().inverse();
        let shift = Vec3::from_array(main.target) - Vec3::from_array(self.offset);
        let shrink = 1.0 / self.scale;
        Camera {
            orientation: back * main.orientation,
            target: (back.rotate(shift) * shrink).to_array(),
            distance: main.distance * shrink,
            lens: main.lens,
            projection: main.projection,
        }
    }
}

/// How a reference view is drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefLook {
    /// The MatCap it is shaded with, by name (`material.list`). Left out: the one the main view
    /// draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcap: Option<String>,
    /// Its edges are drawn over the surface.
    #[serde(default)]
    pub wireframe: bool,
}

/// Where a reference's scene came from, and how a file was read: what a saved scene keeps to
/// bring it back (decision 0024, section 8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A file, read through `file.import`'s readers.
    File {
        /// The path as given.
        path: String,
        /// The BLAKE3 hash of the bytes read.
        hash: String,
        /// The format it was read as; left out: from its extension.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        format: Option<Format>,
        /// Its up axis.
        #[serde(default)]
        up: UpAxis,
        /// For a project: the one tool shown; left out: every tool.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool: Option<u32>,
    },
    /// Frozen copies of these parts of the scene, as they were when it opened.
    Parts { parts: Vec<String> },
    /// The scene itself, live.
    Scene,
}

/// The parts of a camera a change moves, for the link check.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Touches {
    pub turn: bool,
    pub zoom: bool,
    pub pan: bool,
}

impl Touches {
    /// Changes nothing a link can hold (`view.set`, which sets the view's own camera).
    pub const NONE: Touches = Touches {
        turn: false,
        zoom: false,
        pan: false,
    };
    /// Turns (orbit, roll, align).
    pub const TURN: Touches = Touches {
        turn: true,
        zoom: false,
        pan: false,
    };
    /// Slides.
    pub const PAN: Touches = Touches {
        turn: false,
        zoom: false,
        pan: true,
    };
    /// Comes closer or changes the lens.
    pub const ZOOM: Touches = Touches {
        turn: false,
        zoom: true,
        pan: false,
    };
    /// Frames: moves the target and the distance.
    pub const FRAME: Touches = Touches {
        turn: false,
        zoom: true,
        pan: true,
    };
}

/// A secondary view's setup.
#[derive(Clone, Debug)]
pub(crate) struct Secondary {
    pub scene: Scene,
    pub source: Source,
    pub link: Link,
    pub align: Alignment,
    pub look: RefLook,
}

impl Secondary {
    /// The view's camera: `own`, with each linked quantity taken from `main` as the reference
    /// sees it.
    pub fn camera(&self, own: Camera, main: &Camera) -> Camera {
        let follow = self.align.follow(main);
        let mut camera = own;
        if self.link.rotation {
            camera.orientation = follow.orientation;
        }
        if self.link.zoom {
            camera.distance = follow.distance;
            camera.lens = follow.lens;
            camera.projection = follow.projection;
        }
        if self.link.pan {
            camera.target = follow.target;
        }
        camera
    }

    /// Refuses a camera change of view `name` that would move something linked to `main`.
    pub fn allows(&self, name: &str, moves: Touches) -> Result<(), CommandError> {
        let held = [
            (
                moves.turn && self.link.rotation,
                "turns with main",
                "rotation",
            ),
            (
                moves.zoom && self.link.zoom,
                "comes closer with main",
                "zoom",
            ),
            (moves.pan && self.link.pan, "slides with main", "pan"),
        ];
        let Some((_, does, what)) = held.into_iter().find(|h| h.0) else {
            return Ok(());
        };
        let reason = Unavailable::new(format!(
            "{name} {does} ({what} is linked), so this move would not show"
        ))
        .fix(format!(
            "move main instead, or unlink it first with reference.link view={name} {what}=false"
        ));
        Err(errors::unavailable(reason))
    }

    /// As `view.get` and `reference.list` report it, showing `parts`.
    pub fn info(&self, parts: Vec<String>) -> ReferenceInfo {
        ReferenceInfo {
            source: self.source.clone(),
            parts,
            link: self.link,
            align: self.align,
            look: self.look.clone(),
        }
    }
}

/// A Reference View's setup, as `view.get` and `reference.list` report it.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ReferenceInfo {
    /// Where its scene came from.
    pub source: Source,
    /// The parts it shows, by name.
    pub parts: Vec<String>,
    /// What follows main.
    pub link: Link,
    /// Where the reference sits in the sculpt's space.
    pub align: Alignment,
    /// How it is drawn.
    pub look: RefLook,
}
