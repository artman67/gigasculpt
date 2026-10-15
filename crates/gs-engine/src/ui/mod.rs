//! The window, as commands see it: `pen.input`, `ui.describe`, `ui.screenshot`, `ui.palette`,
//! `ui.layout`, `ui.library`, `ui.keymap`, `ui.wheel`, `ui.settings`, `ui.quick`, `ui.menu`,
//! `ui.navigation`, `ui.gizmo`, `ui.curve`, `ui.deform` and `ui.adjust`, and the
//! [`WindowHost`] the window attaches to the engine.
//!
//! The window is the engine's process (ARCHITECTURE 7.2), so commands reach it directly. A
//! session without a window keeps [`NoWindow`]: pen input only moves the session's cursor, and
//! commands that need the window say why they cannot run.

mod adjust;
mod curve;
mod deform;
mod describe;
mod gizmo;
mod keymap;
mod layout;
mod library;
mod menu;
mod navigation;
mod palette;
mod pen;
mod preferences;
mod quick;
mod screenshot;
mod settings;
mod wheel;

use std::sync::{Arc, Mutex, PoisonError};

use gs_core::brush::BrushPreset;
use gs_core::gizmo::{Pose, Style};
use gs_core::{ColumnSet, MeshStore, NodeId};
use gs_protocol::{CommandError, PenSample, Role, Step};
use schemars::JsonSchema;
use serde::Serialize;

use crate::display::{DisplaySettings, Environment, EnvironmentInfo};
use crate::material::MaterialPalette;
use crate::sculpt::PenStroke;
use crate::session::{ReferenceState, SessionState};
use crate::view::RenderRequest;

pub use adjust::{AdjustInfo, UiAdjust};
pub use curve::{HeldCurve, UiCurve};
pub use deform::{DeformSettings, UiDeform};
pub use gizmo::{GizmoSettings, Surface, UiGizmo};
pub use keymap::{KeyBinding, KeymapInfo, UiKeymap};
pub use layout::{Hand, LayoutPart, SETTINGS_VERSION, UiLayout};
pub use library::{LibraryInfo, UiLibrary};
pub use menu::{MenuInfo, MenuOpen, UiMenu};
pub use navigation::{BarrelClick, BarrelTwo, Eraser, Navigation, UiNavigation};
pub use palette::UiPalette;
pub use pen::PenReads;
pub use preferences::{PreferencesInfo, ResetScope, SettingInfo};
pub use quick::{QuickInfo, UiQuick};
pub use settings::{SettingsInfo, UiSettings, WindowSettings};
pub use wheel::UiWheel;

/// What a running engine tells the window, and asks of it. The window's implementation lives
/// in gs-ui; the engine never sees how it draws.
pub trait WindowHost: Send {
    /// False for [`NoWindow`].
    fn has_window(&self) -> bool;
    /// Feeds pen samples through the window's input path, oldest first, reading what the gizmo
    /// tool needs through `reads`. Returns the cursor and whether the gizmo took the samples.
    fn pen_input(&mut self, samples: &[PenSample], reads: &PenReads) -> PenTaken;
    /// The visible interface, after every input so far.
    fn describe(&mut self) -> UiDescription;
    /// The window as presented, with its panels; `None` without a window.
    fn screenshot(&mut self) -> Option<Result<Screenshot, String>>;
    /// Opens or closes the command palette as `request` asks; `None` without a window, an
    /// error naming what is wrong when its form names no command.
    fn palette(&mut self, request: &UiPalette) -> Option<Result<PaletteInfo, String>>;
    /// Switches the layout or its hand as `change` asks; `None` without a window, the refusal
    /// coded by what went wrong: a name or a file not there, a file it may not use, a settings
    /// file it cannot read as settings.
    fn layout(&mut self, _change: &UiLayout) -> Option<Result<LayoutInfo, CommandError>> {
        None
    }
    /// Opens, closes or changes the Library and its favorites as `change` asks; `None`
    /// without a window, an error naming what does not exist.
    fn library(&mut self, _change: &UiLibrary) -> Option<Result<LibraryInfo, String>> {
        None
    }
    /// Binds a key or puts keys back as `change` asks; `None` without a window, the refusal
    /// naming what holds a taken key or what does not exist.
    fn keymap(&mut self, _change: &UiKeymap) -> Option<Result<KeymapInfo, CommandError>> {
        None
    }
    /// Opens the brush wheel at the pointer, lets go of it or changes its slots as `change`
    /// asks; `None` without a window, an error naming a brush or slot that does not exist.
    fn wheel(&mut self, _change: &UiWheel) -> Option<Result<WheelInfo, String>> {
        None
    }
    /// The settings window, the window's own settings and the slider settings, after first
    /// making `change` when given; `None` without a window, the refusal naming a page or
    /// setting that does not exist. Without `change` it leaves the window's lists empty.
    fn settings(
        &mut self,
        _change: Option<&UiSettings>,
    ) -> Option<Result<SettingsInfo, CommandError>> {
        None
    }
    /// The quick menu after `change`: open or shut, the menu shown and what each holds;
    /// `None` without a window.
    fn quick(&mut self, _change: &UiQuick) -> Option<Result<QuickInfo, String>> {
        None
    }
    /// The header's menus after `change`: the one open and all of them; `Err` naming them for
    /// a name that is none of them; `None` without a window.
    fn menu(&mut self, _change: &UiMenu) -> Option<Result<MenuInfo, String>> {
        None
    }
    /// The adjust panel after `change`; `None` without a window.
    fn adjust(&mut self, _change: &UiAdjust) -> Option<AdjustInfo> {
        None
    }
    /// The layout shown and its hand, for `session.state`; `None` without a window or
    /// when no layout is loaded.
    fn shown(&mut self) -> Option<LayoutInfo> {
        None
    }
    /// The 3D view's size in pixels, `main`'s viewport; `None` without a window or before the
    /// first frame.
    fn viewport(&mut self) -> Option<[u32; 2]> {
        None
    }
    /// Each Reference View picture's size in pixels, as last drawn, by view name: the
    /// viewports of the references the window shows. Empty without a window or while it shows
    /// none.
    fn reference_sizes(&mut self) -> Vec<(String, [u32; 2])> {
        Vec::new()
    }
    /// The navigation settings, after first making `change` when given; `None` without a
    /// window.
    fn navigation(&mut self, _change: Option<&UiNavigation>) -> Option<Navigation> {
        None
    }
    /// The gizmo tool's settings, after first making `change` when given; `None` without a
    /// window.
    fn gizmo(&mut self, _change: Option<&UiGizmo>) -> Option<GizmoSettings> {
        None
    }
    /// The curve the window holds to paste, after first holding `change`'s; `None` without a
    /// window.
    fn curve(&mut self, _change: &UiCurve) -> Option<HeldCurve> {
        None
    }
    /// Header > Deform's axes and Mask switch, after first making `change`; `None`
    /// without a window.
    fn deform(&mut self, _change: &UiDeform) -> Option<DeformSettings> {
        None
    }
    /// How large the window draws the gizmo, in its pixels: `gizmo.describe` draws with the
    /// same, so its handles are the window's. The default style without a window.
    fn gizmo_style(&mut self) -> Style {
        Style::default()
    }
    /// Hands the window the pose of the gizmo it shows, if it shows one: `pose` reads a part's
    /// pose by the part's name.
    fn show_gizmo(&mut self, _pose: &dyn Fn(&str) -> Option<Pose>) {}
    /// What the window's controls run, as last drawn, each once: the engine says in every
    /// [`CallReport`] which of them cannot run now (`refused`). Empty without a window.
    fn controls(&mut self) -> Vec<Step> {
        vec![]
    }
    /// Called after every call, and once when the window attaches.
    fn after_call(&mut self, report: &CallReport);
    /// What the pen did to the 3D view since the last call, in order: strokes the engine
    /// sculpts as the pen moves (see `sculpt::live`). None without a window.
    fn take_strokes(&mut self) -> Vec<PenStroke> {
        vec![]
    }
}

/// Renders the scene offscreen for `view.snapshot` and `view.contact_sheet`, from cameras of
/// its own, so the window's camera never moves. gs-ui implements it for the window (on the
/// window's device) and for headless sessions; the engine never sees how it draws.
pub trait RenderHost: Send {
    /// The parts, by their mesh stores, seen by every camera of `request`, as one PNG; `None`
    /// in a session that cannot render. Drawn as the window draws them (decision 0024).
    fn render(
        &mut self,
        parts: &[&MeshStore],
        request: &RenderRequest,
    ) -> Option<Result<Screenshot, String>>;
    /// Takes the session's material palette, handed over after every command that changes it
    /// or its active material (decision 0043): every look is drawn from it from now on, in the
    /// window too. Both sides start from `MaterialPalette::shipped`. An error if the active
    /// material cannot be drawn; `None` in a session that cannot render.
    fn materials(&mut self, _palette: &MaterialPalette) -> Option<Result<(), String>> {
        None
    }
    /// Reads and prepares Lit mode's `environment` (a shipped one or a file; decision 0023), so
    /// the next draw that names it lights with it, and reports it and its light 1. An error if
    /// it cannot be read; `None` in a session that cannot render.
    fn environment(
        &mut self,
        _environment: &Environment,
    ) -> Option<Result<EnvironmentInfo, String>> {
        None
    }
    /// Whether live subsurface runs with `display`'s mode on the device that draws this
    /// session's pictures (decision 0023: `auto` is off on an integrated GPU); `None` in a
    /// session that cannot render.
    fn subsurface_runs(&mut self, _display: &DisplaySettings) -> Option<bool> {
        None
    }
}

/// A host the engine thread and its jobs share: the window, or the offscreen renderer. Calls
/// take turns behind one lock, so a job can render or look at the window as a call in place
/// would (decision 0010). Hosts only need to be `Send`.
pub(crate) struct Shared<T: ?Sized>(Arc<Mutex<Box<T>>>);

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> Shared<T> {
    pub fn new(host: Box<T>) -> Self {
        Self(Arc::new(Mutex::new(host)))
    }

    /// Runs `f` on the host, waiting for its turn. A panic in another call does not lock the
    /// host out.
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut host = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut host)
    }

    /// Puts another host in this one's place, for everyone sharing it.
    pub fn replace(&self, host: Box<T>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = host;
    }
}

/// A session that cannot render.
pub struct NoRenderer;

impl RenderHost for NoRenderer {
    fn render(
        &mut self,
        _parts: &[&MeshStore],
        _request: &RenderRequest,
    ) -> Option<Result<Screenshot, String>> {
        None
    }
}

/// What one call did, for the window's status bar and viewport.
pub struct CallReport<'a> {
    /// Session name.
    pub session: &'a str,
    /// Who ran it (`name#connection`), empty when the window just attached.
    pub client: &'a str,
    pub role: Role,
    /// The commands it ran, in order.
    pub commands: &'a [String],
    pub ok: bool,
    /// Document revision after it.
    pub rev: u64,
    /// Every part after it, by name. Empty for a session without a window.
    pub parts: Vec<PartReport<'a>>,
    /// The session's state after it, exactly as `session.state` reports it: the brush the pen
    /// strokes with, the look, the layout, the session switches.
    pub state: &'a SessionState,
    /// The brush the pen strokes with, as a stroke uses it (its preset with this session's
    /// changes, as `brush.describe` reports it): the window draws its falloff and pressure
    /// curves and its smoothing string. `None` for a session without a window.
    pub preset: Option<BrushPreset>,
    /// What else went wrong beside the call itself, for the status line (`Sculpting.problem`):
    /// the recent list or a new command log that could not be written, an opened file's
    /// materials the renderer cannot draw, a user materials folder that no longer reads. A
    /// refusal of a pen stroke, gizmo drag or deformer comes in `stroke`, and a job's failure
    /// with the job the window follows.
    pub problem: Option<&'a str>,
    /// Why the window's last pen stroke, gizmo drag or accepted deformer was refused, whole
    /// (`Sculpting.refused`), so the window says it in a person's words by its code (gs-ui
    /// `shell/refusals.rs`).
    pub stroke: Option<&'a CommandError>,
    /// The file a save wrote in this call, for the window to say for a moment (review 0009,
    /// R11); not autosave's copy.
    pub saved: Option<&'a std::path::Path>,
    /// The window's view, `main`, as it is after the call.
    pub main: MainView,
    /// Every open Reference View, by number: the window shows them all (decision 0024).
    pub references: Vec<ShownReference<'a>>,
    /// The pen's press landed on empty canvas: the window navigates with it until it lifts.
    pub on_canvas: bool,
    /// Those of the window's controls ([`WindowHost::controls`]) whose command cannot run now,
    /// each with why: the human's dry run of it (the session's policy, the arguments and the
    /// command's own check) refused it as unavailable or naming nothing that exists.
    pub refused: Vec<(Step, String)>,
    /// The open deformer's preview on the graphics card (decision 0041, amended 2026-10-12 for
    /// `DF9a`): the window draws its part's points as they were at the open, moved by it. `None`
    /// while no deformer previews.
    pub preview: Option<&'a crate::gizmo::DeformPreview>,
    /// What Live Boolean draws beside the parts, and how its operands stand in (decision 0057,
    /// section 9). Empty for a session without a window, and while the live mode is off.
    pub live: crate::LiveDraw<'a>,
}

/// A Reference View the window shows, as the window draws it (decisions 0020, 0024).
pub struct ShownReference<'a> {
    /// Its name, links and look, as `session.state` reports the active one's (`reference.*`).
    pub setup: ReferenceState,
    /// It is the active one, whose controls the header shows.
    pub active: bool,
    /// Its camera, lock and pivot, as for `main`.
    pub shown: MainView,
    /// The parts of its own scene; they never change, so they are sent whole. `None` while it
    /// shows the session's own parts, live.
    pub parts: Option<Vec<PartReport<'a>>>,
}

/// The window's view as the window draws it.
#[derive(Clone, Copy, Debug)]
pub struct MainView {
    /// Its exact camera.
    pub camera: gs_core::camera::Camera,
    /// Locked: navigation is off and a press on empty canvas sculpts.
    pub locked: bool,
    /// The point it turns about now.
    pub pivot: [f32; 3],
    /// A gesture is moving it.
    pub moving: bool,
}

impl MainView {
    /// From `view.get`'s report of `main`.
    pub(crate) fn of(info: &crate::view::ViewInfo) -> Self {
        let camera = info
            .camera
            .camera("")
            .unwrap_or_else(|_| gs_core::camera::Camera::preset(gs_core::camera::Preset::Front));
        Self {
            camera,
            locked: info.locked,
            pivot: info.pivot.0,
            moving: info.gesture.is_some(),
        }
    }
}

/// One part, for drawing: its store, and the nodes changed since the last report.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one part, each one the window reads"
)]
pub struct PartReport<'a> {
    pub name: &'a str,
    /// Changes whenever the part is replaced by another (added, removed, swapped by undo), so
    /// a copy of a part with the same name and another id is stale.
    pub id: u64,
    pub store: &'a MeshStore,
    /// What `MeshStore::take_dirty` returned.
    pub dirty: Vec<(NodeId, ColumnSet)>,
    /// Whether the part shows (decisions 0028 and 0033): a hidden part is listed but not drawn,
    /// unless it is the active part. Showing
    /// it again keeps its id (decision 0029), so the window keeps its GPU copy meanwhile.
    pub shown: bool,
    /// Whether the 3D view draws it now: `shown`, except that with Dynamic isolate on and Isolate
    /// off, only the active part is drawn while the camera moves (decision 0028, section 10).
    pub drawn: bool,
    /// Its own visibility, the eye `part.set_visible` sets, as `part.list` reports it. It
    /// differs from `shown` while Isolate is on: a part Isolate set aside is not shown, though
    /// it may be visible.
    pub visible: bool,
    /// It is the active part (decision 0033), which always shows.
    pub active: bool,
    /// Its paint shows (`part.set_paint_visible`, decision 0039 section 10); off, it draws in
    /// its material alone.
    pub painted: bool,
    /// Where the part is drawn: its transform, part to scene (decision 0029).
    pub transform: gs_core::transform::Similarity,
}

/// What the window did with pen samples.
pub struct PenTaken {
    /// The pen after the last sample, or `None` if it left range.
    pub cursor: Option<PenSample>,
    /// The gizmo tool took them: they pressed, dragged or let go of a handle, or put the gizmo
    /// on the model. The commands they led to are logged, so the samples are not.
    pub gizmo: bool,
}

/// A session with no window: pen input moves the session's cursor and nothing else.
#[derive(Default)]
pub struct NoWindow {
    cursor: Option<PenSample>,
}

impl WindowHost for NoWindow {
    fn has_window(&self) -> bool {
        false
    }

    fn pen_input(&mut self, samples: &[PenSample], _reads: &PenReads) -> PenTaken {
        if let Some(last) = samples.last() {
            self.cursor = last.in_proximity.then_some(*last);
        }
        PenTaken {
            cursor: self.cursor,
            gizmo: false,
        }
    }

    fn describe(&mut self) -> UiDescription {
        UiDescription {
            cursor: self.cursor,
            ..UiDescription::default()
        }
    }

    fn screenshot(&mut self) -> Option<Result<Screenshot, String>> {
        None
    }

    fn palette(&mut self, _request: &UiPalette) -> Option<Result<PaletteInfo, String>> {
        None
    }

    fn after_call(&mut self, _report: &CallReport) {}
}

/// Why a window command cannot run in a session without one.
fn no_window() -> gs_protocol::CommandError {
    crate::errors::unavailable(
        crate::Unavailable::new("this session has no window")
            .fix("open the session in the window: run `gigasculpt` with no arguments"),
    )
}

/// A PNG of the window.
pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// The visible interface: `ui.describe`'s answer.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct UiDescription {
    /// False in a session without a window; then only `cursor` is set.
    pub window: bool,
    /// Window size in physical pixels.
    pub size: [u32; 2],
    /// Physical pixels per window point (1.25 on a monitor scaled to 125%).
    pub pixel_density: f32,
    /// The visible panels, in drawing order.
    pub panels: Vec<PanelInfo>,
    /// The pen after the last sample, while it is in range.
    pub cursor: Option<PenSample>,
    /// The 3D view, if it is visible.
    pub viewport: Option<ViewportInfo>,
    /// Frame timing and pen-to-screen latency.
    pub perf: Option<PerfInfo>,
}

/// One panel and its widgets.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct PanelInfo {
    /// Stable id, for example `status` or `palette`.
    pub id: String,
    /// What the user reads.
    pub title: String,
    /// Min x, min y, max x, max y in physical pixels.
    pub rect: [f32; 4],
    /// Its widgets that show, top to bottom. In a panel that scrolls, each is cut to what
    /// shows, and one scrolled wholly out of sight is left out.
    pub widgets: Vec<WidgetInfo>,
    /// A tray's titled sections that show, top to bottom (decision 0025), cut like its
    /// widgets; none for other panels.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionInfo>,
}

/// One titled section of a tray, or one of the header's view buttons with the options it opens.
/// A tray section's title row, on top, folds it open or shut; folding is not a command, like
/// scrolling.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct SectionInfo {
    /// Stable id, the piece it shows, for example `geometry`.
    pub id: String,
    /// What the user reads.
    pub title: String,
    /// Shut: only its title row shows.
    pub folded: bool,
    /// One of the header's view buttons: lit, as the state value it names (`shows` in the
    /// layout file) is true. Left out when not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lit: bool,
    /// Min x, min y, max x, max y in physical pixels: the title row and what it shows.
    pub rect: [f32; 4],
}

/// One widget.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct WidgetInfo {
    /// Its text.
    pub label: String,
    /// `button`, `toggle`, `slider`, `text field`, `list item`, `label` or `scroll bar`.
    pub role: String,
    /// The command it runs, if any.
    pub command: Option<Step>,
    /// The key bound to that command, if any.
    pub key: Option<String>,
    /// False when it cannot be used now.
    pub enabled: bool,
    /// Why it is dimmed: what it waits for or why its command cannot run now, or, while it is
    /// enabled, why it has no effect now (decision 0025, section 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What hovering it says beside its label, if anything: an alpha's name, size and bits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// True for the highlighted row of a list, or a toggle that is on.
    pub selected: bool,
    /// For a toggle, the `session.state` value it shows and the value that lights it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shows: Option<Shows>,
    /// Min x, min y, max x, max y in physical pixels.
    pub rect: [f32; 4],
    /// Partly scrolled out of its list: `rect` is the part that shows. Left out when not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

/// What a toggle shows: it is lit while the `session.state` value called `value` equals `on`
/// (numbers as numbers), or, for an object `on` such as `{"on": true}` for `light.1`, while
/// it holds `on`'s fields.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Shows {
    pub value: String,
    pub on: serde_json::Value,
}

/// The 3D view.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct ViewportInfo {
    /// Min x, min y, max x, max y in physical pixels.
    pub rect: [f32; 4],
    /// Parts drawn, by name (with no renderer, the parts the window knows of).
    pub parts: Vec<String>,
    /// How surfaces are shaded, for example `matcap gray`.
    pub shading: String,
    /// The camera it draws through, `main`'s, exactly: pass it as a look's `view` or a
    /// stroke's `camera` to see the same picture.
    pub camera: crate::view::ExactCamera,
}

/// Frame timing and latency.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct PerfInfo {
    /// The GPU, for example `NVIDIA GeForce RTX 5090`.
    pub adapter: String,
    /// `Mailbox`, `Immediate` or `Fifo`.
    pub present_mode: String,
    /// Frames presented since the window opened.
    pub frames: u64,
    /// Time to build and submit one frame, over the recent frames.
    pub frame: Timing,
    /// Pen latency, hop by hop, over the recent pen frames (ARCHITECTURE section 11 target:
    /// pen sample to GPU submit).
    pub hops: Vec<Hop>,
}

/// One latency hop.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct Hop {
    /// For example `compositor to app`.
    pub name: String,
    pub timing: Timing,
}

/// Recent durations in milliseconds.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct Timing {
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

/// The command palette after `ui.palette`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct PaletteInfo {
    pub open: bool,
    /// What is typed into it.
    pub query: String,
    /// What it lists, best match first, as each row reads: the labels of the layout's controls,
    /// then commands by their titles (those no control lists), then by their names, which lead
    /// for a query with a dot (at most 20 controls and 20 commands).
    pub matches: Vec<String>,
    /// The same rows in full: what each runs, its key and where it lives.
    pub rows: Vec<PaletteRow>,
    /// The command whose arguments the palette is asking for, if it is.
    pub form: Option<String>,
}

/// One row of the palette.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct PaletteRow {
    /// A control's label, a command's title, or a command's name.
    pub label: String,
    /// What picking it runs.
    pub command: Step,
    /// The key bound to it, if any.
    pub key: Option<String>,
    /// Where it lives in the layout shown, for example `Brushes > Build` or `Brush wheel`;
    /// empty when only the palette and keys reach it.
    pub places: Vec<String>,
}

/// The layout after `ui.layout`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per thing the window shows"
)]
pub struct LayoutInfo {
    /// The layout shown, by name.
    pub name: String,
    /// What it is called in the window.
    pub title: String,
    /// The hand it is arranged for.
    pub hand: Hand,
    /// The interface's size on top of the scale the display asks for (`ui.layout` `scale`).
    pub scale: f32,
    /// Every layout, by name, in order.
    pub layouts: Vec<String>,
    /// Whether the Library drawer is open (`ui.library`).
    pub library: bool,
    /// The theme shown, by name (`ui.layout` `theme`).
    pub theme: String,
    /// Every theme, by name, in the order `theme_step` steps through them.
    pub themes: Vec<String>,
    /// Whether the tray on the window's left edge shows, and the one on its right (`ui.layout`
    /// `left` and `right`); false where there is none. Minimal chrome does not change them.
    pub left: bool,
    pub right: bool,
    /// False while minimal chrome hides the panels (`ui.layout` `chrome`).
    pub chrome: bool,
    /// One section open at a time in each tray (`ui.layout` `one_open`).
    pub one_open: bool,
}

/// The brush wheel after `ui.wheel`.
#[derive(Clone, Debug, Default, Serialize, JsonSchema)]
pub struct WheelInfo {
    pub open: bool,
    /// The control letting go picked, by its label; its command runs next.
    pub picked: Option<String>,
    /// Its slots' labels, the top one first and then clockwise.
    pub slots: Vec<String>,
}
