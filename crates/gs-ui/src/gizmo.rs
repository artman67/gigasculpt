//! The gizmo tool in the window (decision 0027, section 11). While it is on (`ui.gizmo`), the 3D
//! view shows one part's gizmo over the model, drawn from gs-core's `handles` in the interface
//! layer (`draw`), and the pen drives it instead of sculpting (`press`). A press looks for a
//! handle first (gs-core's `pick`, a pen's radius around it), then for the model, which puts the
//! gizmo there along the surface, and a drag from there aims it toward the pen; on empty canvas
//! it navigates. A held handle's motion becomes a transform (`drag`): `gizmo.drag` updates while
//! the gizmo is locked (with Ctrl, of a copy of the part), one `gizmo.place` when it is not or
//! Alt is held. Its buttons in the Brush tray are `pose`, and its deformers there `deformer`.
//!
//! The window shows the gizmo of the part a press last put it on, while that part shows, or
//! else of the first part that shows; after a Ctrl drag, the copy's. It keeps that part's pose
//! as the session last handed it over (`show`): every window command that draws or picks the
//! gizmo hands it over first, and the window asks for it again (an empty `ui.gizmo`) after any
//! other call, which may have moved it.
//!
//! While a deformer is open its handles show over the gizmo and a press grabs them first
//! (`deforming`, decision 0041 as amended for `DF7c`).

pub(crate) mod deformer;
mod deforming;
mod drag;
mod draw;
mod pose;
mod press;

use gs_core::camera::Camera;
use gs_core::gizmo::{HandleId, Handles, MOVE_SNAP, Pose, Style, handles, pick};
use gs_engine::{BrushState, GizmoSettings, UiGizmo};
use gs_protocol::{PenSample, Step};
use serde::{Deserialize, Serialize};

use crate::pieces::ListedPart;
use deforming::{Deforming, Hold, Rectangle};
use drag::Grab;
pub use draw::{Lit, draw};
pub use pose::{MENU, menu, pose_buttons};
use press::Aim;

/// The gizmo's settings kept with the layout state: its size on screen in points, which
/// `gizmo.describe` draws with too, the steps of a full turn a Shift drag snaps to, whether the
/// mask holds points back in a drag, and a drag's falloff.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GizmoPrefs {
    pub size: f32,
    pub snap: u32,
    pub mask: bool,
    pub falloff: f32,
}

impl Default for GizmoPrefs {
    /// 75 points (gs-core's default style), 72 steps (5 degrees), the mask holds back, and a
    /// drag is rigid: the whole part moves (decision 0027, section 5).
    fn default() -> Self {
        Self {
            size: Style::default().size_pt,
            snap: 72,
            mask: true,
            falloff: -100.0,
        }
    }
}

impl GizmoPrefs {
    /// The style the window draws and picks with on a screen of `ppp` pixels per point.
    pub fn style(self, ppp: f32) -> Style {
        Style {
            size_pt: self.size,
            px_per_pt: ppp,
            ..Style::default()
        }
    }
}

/// The gizmo tool between frames.
#[derive(Default)]
pub struct Tool {
    /// The pen drives the gizmo.
    pub on: bool,
    /// The part whose gizmo shows, and its pose as the session last handed it over.
    shown: Option<(String, Pose)>,
    /// What the pen's press does until the pen lifts.
    press: PenPress,
    /// A Ctrl drag made a copy, which is the active part: show its gizmo next.
    to_active: bool,
    /// The open deformer, whose handles show while the tool is on.
    deforming: Option<Deforming>,
}

/// What the pen's press is doing to the gizmo.
#[derive(Default)]
enum PenPress {
    /// The pen is up.
    #[default]
    Up,
    /// It holds a handle.
    Grab(Box<Grab>),
    /// It put the gizmo on the model; a drag aims it.
    Aim(Box<Aim>),
    /// It holds one of the open deformer's handles.
    Hold(Box<Hold>),
    /// It draws the pen rectangle that selects a cage's or a curve's dots.
    Rect(Box<Rectangle>),
    /// It put the gizmo on the model, or its drag was cancelled: it waits for the pen to lift.
    Done,
    /// It is not the gizmo's (it landed on empty canvas, or off the 3D view): it waits for the
    /// pen to lift.
    Away,
}

/// The 3D view as the pen sees it: its camera, its top-left corner in the window and its size,
/// in pixels, the gizmo's style, the brush, the gizmo's settings, and whether Alt, Shift and
/// Ctrl are held.
pub struct Seen<'a> {
    pub camera: &'a Camera,
    pub corner: [f32; 2],
    pub size: [f32; 2],
    pub style: Style,
    pub brush: &'a BrushState,
    pub prefs: GizmoPrefs,
    pub alt: bool,
    pub shift: bool,
    pub ctrl: bool,
}

/// What one pen sample did.
#[derive(Default)]
pub struct Took {
    /// The gizmo took it: it drives a drag or a placement, not a stroke.
    pub gizmo: bool,
    /// It pressed on empty canvas: the drag navigates.
    pub canvas: bool,
    /// The commands it asks for, in order.
    pub steps: Vec<Step>,
    /// Why it refused a press, for the status line.
    pub problem: Option<String>,
}

impl Tool {
    /// The settings, as `ui.gizmo` answers.
    pub fn settings(&self, prefs: GizmoPrefs) -> GizmoSettings {
        GizmoSettings {
            tool: self.on,
            size: prefs.size,
            snap: prefs.snap,
            mask: prefs.mask,
            falloff: prefs.falloff,
            part: self.drawn().map(|(part, _)| part.to_string()),
        }
    }

    /// `ui.gizmo`: makes `change`. Turning the tool off lets a press on it finish.
    pub fn change(&mut self, prefs: &mut GizmoPrefs, change: &UiGizmo) {
        self.on = change.tool.unwrap_or(self.on);
        prefs.size = change.size.unwrap_or(prefs.size);
        prefs.snap = change.snap.unwrap_or(prefs.snap);
        prefs.mask = change.mask.unwrap_or(prefs.mask);
        prefs.falloff = change.falloff.unwrap_or(prefs.falloff);
    }

    /// Takes the session's word on the gizmo shown, while the tool is on: the part a press last
    /// put it on if that part still shows (after a Ctrl drag, the active part: the copy), else
    /// the first part that shows, and its pose (`pose` reads a part's pose by name). True when
    /// what shows changed.
    pub fn show(&mut self, parts: &[ListedPart], pose: &dyn Fn(&str) -> Option<Pose>) -> bool {
        if !self.on {
            return false;
        }
        let shows = |name: &str| parts.iter().any(|p| p.shown && p.name == name);
        let active = (std::mem::take(&mut self.to_active))
            .then(|| parts.iter().find(|p| p.shown && p.active))
            .flatten()
            .map(|p| p.name.clone());
        let kept = (self.shown.as_ref()).and_then(|(part, _)| shows(part).then(|| part.clone()));
        let first = || parts.iter().find(|p| p.shown).map(|p| p.name.clone());
        let part = active.or(kept).or_else(first);
        let shown = part.and_then(|part| Some((part.clone(), pose(&part)?)));
        let changed = shown != self.shown;
        self.shown = shown;
        changed
    }

    /// The part whose gizmo is drawn and its pose as drawn: while a handle is held, where the
    /// drag puts it; while a press on the model drags, where it aims. `None` while the tool is
    /// off and nothing is held.
    pub fn drawn(&self) -> Option<(&str, Pose)> {
        match &self.press {
            PenPress::Grab(g) => return Some((&g.part, g.drawn())),
            PenPress::Aim(a) => return Some((&a.part, a.pose())),
            _ => {}
        }
        let (part, pose) = self.shown.as_ref().filter(|_| self.on)?;
        Some((part, *pose))
    }

    /// The handle the pen holds.
    fn held(&self) -> Option<HandleId> {
        match &self.press {
            PenPress::Grab(g) => Some(g.id),
            _ => None,
        }
    }

    /// The handle in use among `drawn`: the one the pen holds, or else the one a press at
    /// `hover` (the view's pixels) would grab.
    fn hot(&self, drawn: &Handles, style: &Style, hover: Option<[f32; 2]>) -> Option<HandleId> {
        let radius = style.pick_pt * style.px_per_pt;
        (self.held()).or_else(|| hover.and_then(|at| pick(&drawn.handles, at, radius)))
    }

    /// What the held keys work on while the tool is on: the handle in use, with `pen` (in
    /// window pixels) hovering where the brush shows over the 3D view `rect` (in points) as
    /// `camera` sees it, drawn with `prefs` at `ppp` pixels per point.
    pub fn in_use(
        &self,
        prefs: GizmoPrefs,
        pen: Option<PenSample>,
        (rect, camera): (egui::Rect, &Camera),
        ppp: f32,
    ) -> Option<InUse> {
        if !self.on {
            return None;
        }
        let style = prefs.style(ppp);
        let corner = rect.min * ppp;
        let hover =
            (pen.filter(|p| !p.contact)).map(|p| [p.x as f32 - corner.x, p.y as f32 - corner.y]);
        let size = rect.size() * ppp;
        let handle = self.drawn().and_then(|(_, pose)| {
            let drawn = handles(&pose, camera, [size.x, size.y], &style);
            self.hot(&drawn, &style, hover)
        });
        Some(InUse {
            handle,
            steps: prefs.snap,
        })
    }
}

/// What the held keys do to a gizmo drag (decision 0027, sections 8 and 14), by the handle in
/// use: the one the pen holds, or else the one a press where it hovers would grab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InUse {
    pub handle: Option<HandleId>,
    /// Shift snaps a turn to this many steps of a full turn (the gizmo's Snap).
    pub steps: u32,
}

impl Default for InUse {
    /// No handle, the gizmo's default steps.
    fn default() -> Self {
        Self {
            handle: None,
            steps: GizmoPrefs::default().snap,
        }
    }
}

impl InUse {
    /// What `m` (some key held) does to the drag: Alt moves the gizmo only and wins over Ctrl;
    /// Ctrl copies, but a scale box ignores it; Shift snaps a move to [`MOVE_SNAP`] and a turn
    /// to the steps, and a scale not at all. With no handle in use, Shift snaps both.
    pub fn says(self, m: egui::Modifiers) -> String {
        let kind = self.handle.map(HandleId::kind);
        let first = match (m.alt, m.ctrl) {
            (true, _) => Some("Gizmo only"),
            (false, true) if kind != Some("box") => Some("Copy"),
            (false, _) => None,
        };
        let snap = m.shift.then(|| match kind {
            None => "Snap moves and turns".to_string(),
            Some("ring" | "view_ring") => format!("Snap turns to {}°", turn_step(self.steps)),
            Some("box" | "grip") => "Scale does not snap".to_string(),
            Some(_) => format!("Snap moves to {MOVE_SNAP}"),
        });
        match (first, snap) {
            (Some(first), Some(snap)) => format!("{first}, {}", snap.to_lowercase()),
            (Some(first), None) => first.to_string(),
            (None, Some(snap)) => snap,
            (None, None) => "No copy on a box".to_string(),
        }
    }
}

/// The degrees a Shift turn snaps to, `steps` to a full turn, to two places.
fn turn_step(steps: u32) -> f32 {
    (360.0 / steps.max(1) as f32 * 100.0).round() / 100.0
}
