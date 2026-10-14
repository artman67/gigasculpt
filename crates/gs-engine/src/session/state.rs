//! `session.state`: the session's state as named values, and the one place they are named
//! (decision 0013). The window draws from the same values: the engine sends them with every
//! call's report, built by the same function, so a lit button always means what an agent
//! reads here.

use std::collections::BTreeMap;
use std::sync::Arc;

use gs_core::brush::{BrushPreset, Depth, PaintSpec, Post, TipSpec};
use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ctx::Control;
use crate::display::DisplaySettings;
use crate::document::Document;
use crate::engine::{EngineConfig, State};
use crate::file::{AutosaveSettings, FileState, FreeSpace, LowDisk, RecentFile, StateFolder};
use crate::history::{Ends, Moved};
use crate::mesh::VoxelSettings;
use crate::sculpt::{BrushSettings, BrushState, Colors, Refusal, blocked_on_active};
use crate::ui::WindowSettings;
use crate::ui::{GizmoSettings, LayoutInfo, Navigation, RenderHost, Shared, WindowHost};
use crate::view::{Link, MAIN, MainSettings, NavSettings, Views};
use crate::{Command, Ctx, Example, register_command};

/// The session's state, typed: the brush settings, the look, the window's layout and the
/// session switches. [`SessionState::values`] names each value.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, each a value session.state names"
)]
pub struct SessionState {
    /// The brush the pen strokes with and its settings.
    pub brush: BrushState,
    /// The alphas used lately, slot by slot, as the Alpha palette shows them (`alpha.pick`).
    pub alpha_recent: Vec<String>,
    /// The paint colors and their switches (`color.set`).
    pub colors: Colors,
    /// The active material's name, what MatCap and Lit shading draw: always there, renderer
    /// or not (decision 0043).
    pub look: Option<String>,
    /// The layout the window shows; `None` without a window.
    pub layout: Option<LayoutInfo>,
    /// A human paused agents.
    pub paused: bool,
    /// A human gesture is in progress.
    pub gesture: bool,
    /// The human lets agents move `main` while the window shows it.
    pub agents_move_view: bool,
    /// The steps undo and redo would take, and which way the history moved last.
    pub history: HistoryState,
    /// The scene's own file, as text; `None` before its first save (decision 0016, section 17).
    pub file: Option<String>,
    /// Autosave's settings (`file.autosave`); `None` with autosave off.
    pub autosave: Option<AutosaveSettings>,
    /// The recent scenes (`file.recent`'s list); `None` with autosave off (section 17, item 7).
    pub recent: Option<Vec<RecentFile>>,
    /// A folder the session saves in that is short of space; `None` while there is room.
    pub low_disk: Option<LowDisk>,
    /// `main`'s lock, orbit mode, pivot rule, projection and wireframe.
    pub view: MainSettings,
    /// How `main`'s moves behave (`view.settings`): the recenter rate, snap step, zoom
    /// distance, pan by lens and right-drag pivot.
    pub moves: NavSettings,
    /// The brush the window starts with, `last` or a pinned brush's name (`ui.settings`
    /// `launch_brush`); `None` without a window.
    pub launch_brush: Option<String>,
    /// The session's Voxel Remesh settings (`mesh.voxel_settings`).
    pub voxel: VoxelSettings,
    /// The window's navigation settings; `None` without a window.
    pub navigation: Option<Navigation>,
    /// The window's gizmo tool and settings; `None` without a window.
    pub gizmo: Option<GizmoSettings>,
    /// The window's own settings (`ui.settings`); `None` without a window.
    pub settings: Option<WindowSettings>,
    /// The active Reference View; `None` while none is open.
    pub reference: Option<ReferenceState>,
    /// How the window and the looks draw (decision 0023).
    pub display: DisplaySettings,
    /// Whether live subsurface runs with the display's mode on this session's device: `auto`
    /// resolved (decision 0023). `None` in a session that cannot render.
    pub subsurface_running: Option<bool>,
    /// Isolate is on: a part it chose is in the scene (decision 0029, section 9).
    pub isolated: bool,
    /// The active part's name (decision 0033); `None` in an empty scene.
    pub active: Option<String>,
    /// The active part's Voxel mode switch (decision 0002); `None` in an empty scene.
    pub voxel_mode: Option<bool>,
    /// The active part's detail levels: how many, the base included, the active one from 1 at
    /// the base, and how many `level.freeze` set aside; `None` in an empty scene.
    pub levels: Option<[usize; 3]>,
    /// Why Live Detail cannot refine the active part under the brush, for the window's cursor
    /// (decision 0021, section 8); `None` when it can, and in an empty scene.
    pub live_detail_blocked: Option<Refusal>,
    /// The pen's brush's settings in effect that the trays light from.
    pub in_effect: BrushInEffect,
    /// The open deformer (`gizmo.deformer`); `None` while none is open.
    pub deformer: Option<crate::gizmo::OpenDeformer>,
    /// Live Boolean's groups and their results (decision 0057, section 8).
    pub live: crate::boolean::LiveState,
}

/// The pen's brush's Depth, gravity, tilt, profile and paint settings in effect: its preset with
/// this session's changes on it, as a stroke reads them (decision 0053, section 8, decision 0059,
/// section 4, and decision 0039, section 11, as amended 2026-10-12).
#[derive(Clone, Debug, Default)]
pub struct BrushInEffect {
    pub depth: Depth,
    pub post: Post,
    pub tip: TipSpec,
    pub paint: Option<PaintSpec>,
}

impl BrushInEffect {
    /// `preset`'s, or every default without one.
    fn of(preset: Option<&BrushPreset>) -> Self {
        preset.map_or_else(Self::default, |p| Self {
            depth: p.depth.clone(),
            post: p.post,
            tip: p.tip.clone(),
            paint: p.paint,
        })
    }

    /// Each as `brush.` and its path under `brush.set`'s `settings`; Adaptive Map Size is null
    /// while the brush has no alpha, and Color Strength while it paints nothing.
    fn values(&self) -> Vec<(String, Value)> {
        let (d, p, t) = (&self.depth, &self.post, &self.tip);
        [
            ("brush.depth.imbed", json!(d.imbed)),
            ("brush.depth.mask", json!(d.mask)),
            ("brush.depth.inner", json!(d.inner)),
            ("brush.depth.outer", json!(d.outer)),
            ("brush.depth.infinite", json!(d.infinite)),
            ("brush.depth.axes", json!(d.axes)),
            ("brush.post.gravity", json!(p.gravity)),
            ("brush.post.gravity_dir", json!(p.gravity_dir)),
            ("brush.tip.tilt", json!(t.tilt)),
            ("brush.tip.constant_tilt", json!(t.constant_tilt)),
            ("brush.tip.align", json!(t.align)),
            ("brush.tip.profile", json!(t.profile)),
            ("brush.tip.accurate_curve", json!(t.accurate_curve)),
            ("brush.tip.curve_by_pen", json!(t.curve_by_pen)),
            (
                "brush.tip.alpha.adaptive",
                json!(t.alpha.as_ref().map(|a| a.adaptive)),
            ),
            (
                "brush.paint.color_strength",
                json!(self.paint.map(|p| p.color_strength)),
            ),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into()
    }
}

/// The active Reference View (the one opened last or moved to by `reference.next`), as
/// `session.state` names it (decisions 0020, 0024).
#[derive(Clone, Debug, PartialEq)]
pub struct ReferenceState {
    /// Its name, for example `ref:1`.
    pub view: String,
    pub link: Link,
    /// The MatCap it draws with; `None` while it draws the main view's.
    pub matcap: Option<String>,
    pub wireframe: bool,
}

/// Every named value, by name. Every name is always there; a value that does not apply now
/// is `null`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct StateValues(pub BTreeMap<String, Value>);

impl StateValues {
    /// The value called `name`, if there is one by that name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }
}

impl SessionState {
    /// Each value by the name `session.state` gives it. A name follows the argument that sets
    /// the value.
    pub fn values(&self) -> StateValues {
        let b = &self.brush;
        let s = &b.symmetry_settings;
        let r = self.reference.as_ref();
        let reference_look = r.map(|r| r.matcap.as_ref().or(self.look.as_ref()));
        let values = [
            ("brush", json!(b.brush)),
            ("brush.previous", json!(b.previous)),
            ("brush.size_px", json!(b.size_px)),
            ("brush.size", json!(b.size)),
            ("brush.strength", json!(b.strength)),
            ("brush.falloff", json!(b.falloff)),
            ("brush.modifier", json!(b.modifier)),
            ("brush.smoothing", json!(b.smoothing)),
            ("brush.mode", json!(b.mode)),
            ("brush.slot.smooth", json!(b.slots.smooth)),
            ("brush.slot.mask", json!(b.slots.mask)),
            ("brush.slot.select", json!(b.slots.select)),
            ("brush.slot.held", json!(b.slots.held)),
            ("brush.settings", json!(b.settings)),
            ("brush.alpha", json!(b.alpha)),
            ("alpha.recent", json!(self.alpha_recent)),
            ("symmetry.on", json!(s.on)),
            ("symmetry.x", json!(s.x)),
            ("symmetry.y", json!(s.y)),
            ("symmetry.z", json!(s.z)),
            ("symmetry.x_type", json!(s.x_type)),
            ("symmetry.y_type", json!(s.y_type)),
            ("symmetry.z_type", json!(s.z_type)),
            ("symmetry.radial_axis", json!(s.radial_axis())),
            ("symmetry.radial_count", json!(s.radial_count)),
            ("look", json!(self.look)),
            ("session.paused", json!(self.paused)),
            ("session.gesture", json!(self.gesture)),
            ("session.agents_move_view", json!(self.agents_move_view)),
            ("view.locked", json!(self.view.locked)),
            ("view.orbit", json!(self.view.orbit)),
            ("view.pivot", json!(self.view.pivot)),
            ("view.projection", json!(self.view.projection)),
            ("view.wireframe", json!(self.view.wireframe)),
            ("gizmo.tool", json!(self.gizmo.as_ref().map(|g| g.tool))),
            ("gizmo.size", json!(self.gizmo.as_ref().map(|g| g.size))),
            ("gizmo.snap", json!(self.gizmo.as_ref().map(|g| g.snap))),
            ("gizmo.mask", json!(self.gizmo.as_ref().map(|g| g.mask))),
            (
                "gizmo.falloff",
                json!(self.gizmo.as_ref().map(|g| g.falloff)),
            ),
            ("reference", json!(r.map(|r| &r.view))),
            ("reference.link.rotation", json!(r.map(|r| r.link.rotation))),
            ("reference.link.zoom", json!(r.map(|r| r.link.zoom))),
            ("reference.link.pan", json!(r.map(|r| r.link.pan))),
            ("reference.look", json!(reference_look.flatten())),
            ("reference.wireframe", json!(r.map(|r| r.wireframe))),
            ("parts.isolated", json!(self.isolated)),
            ("parts.active", json!(self.active)),
            ("parts.voxel_mode", json!(self.voxel_mode)),
            ("startup.brush", json!(self.launch_brush)),
            ("startup.pinned", json!(self.pinned())),
            ("boolean.live", json!(self.live)),
        ];
        let mut values: BTreeMap<String, Value> = values.map(|(k, v)| (k.to_string(), v)).into();
        values.extend(b.stroke_values());
        values.extend(color_values(&self.colors));
        values.extend(self.in_effect.values());
        values.extend(fields("brush", Some(&b.increments)));
        values.extend(layout_values(self.layout.as_ref()));
        values.extend(display_values(&self.display, self.subsurface_running));
        values.extend(self.detail_values());
        values.extend(self.settings_values());
        values.extend(self.autosave_values());
        values.extend(self.history_and_file_values());
        values.extend(self.deformer_values());
        values.extend(fields("pen", Some(&self.brush.pen)));
        values.extend(fields("voxel", Some(&self.voxel)));
        values.extend(fields("navigation", self.navigation.as_ref()));
        values.extend(fields("view", Some(&self.moves)));
        StateValues(values)
    }

    /// The brush the window starts with when one is pinned; `None` with Last used or without a
    /// window.
    fn pinned(&self) -> Option<&str> {
        (self.launch_brush.as_deref()).filter(|b| *b != "last")
    }

    /// The open deformer (`gizmo.deformer`): what it does, its part, whether a handle is held,
    /// and its arguments now; null while none is open.
    fn deformer_values(&self) -> Vec<(String, Value)> {
        let d = self.deformer.as_ref();
        [
            ("deformer", json!(d.map(|d| &d.kind))),
            ("deformer.part", json!(d.map(|d| &d.part))),
            ("deformer.held", json!(d.map(|d| d.held))),
            ("deformer.transforms", json!(d.map(|d| &d.transforms))),
            ("deformer.smooth", json!(d.and_then(|d| d.smooth.as_ref()))),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into()
    }

    /// Autosave's settings (`file.autosave`); null with autosave off.
    fn autosave_values(&self) -> Vec<(String, Value)> {
        let a = self.autosave.as_ref();
        [
            ("autosave.max_minutes", json!(a.map(|a| a.max_minutes))),
            ("autosave.rest_minutes", json!(a.map(|a| a.rest_minutes))),
            ("autosave.keep", json!(a.map(|a| a.keep))),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into()
    }

    /// The undo history's values and the scene file's: its path, the recent scenes and a low
    /// disk warning.
    fn history_and_file_values(&self) -> Vec<(String, Value)> {
        let h = &self.history;
        [
            ("history.undo", json!(h.undo)),
            ("history.undo_id", json!(h.undo_id)),
            ("history.redo", json!(h.redo)),
            ("history.moved", json!(h.moved)),
            ("history.undos", json!(h.undos)),
            ("history.redos", json!(h.redos)),
            ("file.path", json!(self.file)),
            ("file.recent", json!(self.recent)),
            ("file.low_disk", json!(self.low_disk)),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into()
    }

    /// The window's own settings (`ui.settings`), each as `settings.` and its field's name;
    /// null without a window.
    fn settings_values(&self) -> Vec<(String, Value)> {
        let fields = json!(self.settings.unwrap_or_default());
        let fields = fields.as_object().cloned().unwrap_or_default();
        let on = self.settings.is_some();
        let shown = |v| if on { v } else { Value::Null };
        (fields.into_iter())
            .map(|(k, v)| (format!("settings.{k}"), shown(v)))
            .collect()
    }

    /// Live Detail's values (`live_detail.set`), with whether it can refine the active part
    /// under the brush.
    fn detail_values(&self) -> Vec<(String, Value)> {
        let (ld, blocked) = (&self.brush.live_detail, self.live_detail_blocked.as_ref());
        let eligible = self.active.as_ref().map(|_| blocked.is_none());
        [
            ("live_detail.on", json!(ld.on)),
            ("live_detail.mode", json!(ld.settings.mode)),
            ("live_detail.size", json!(ld.settings.size)),
            ("live_detail.merge", json!(ld.settings.merge)),
            ("live_detail.combined", json!(ld.settings.combined)),
            ("live_detail.eligible", json!(eligible)),
            ("live_detail.blocked", json!(blocked.map(|r| r.blocked))),
            ("live_detail.why_not", json!(blocked.map(|r| &r.why))),
            ("parts.levels", json!(self.levels.map(|l| l[0]))),
            ("parts.level", json!(self.levels.map(|l| l[1]))),
            ("parts.frozen", json!(self.levels.map(|l| l[2]))),
        ]
        .map(|(k, v)| (k.to_string(), v))
        .into()
    }

    /// Every name `session.state` reports, in its order.
    pub fn names() -> Vec<String> {
        Self::default().values().0.into_keys().collect()
    }
}

/// Each field of `settings` as `prefix.` and its name: `pen.set`'s feel, `ui.navigation`'s
/// settings. With no `settings` (no window), every field of the default, each null.
fn fields<T: Serialize + Default>(prefix: &str, settings: Option<&T>) -> Vec<(String, Value)> {
    let all = json!(T::default());
    let given = settings.map(|s| json!(s));
    let shown = |k: &str| given.as_ref().map_or(Value::Null, |g| g[k].clone());
    (all.as_object().into_iter().flatten())
        .map(|(k, _)| (format!("{prefix}.{k}"), shown(k)))
        .collect()
}

/// The palette's values (`color.set`), each always there: the colors' serde form leaves out
/// switches at their defaults.
fn color_values(c: &Colors) -> [(String, Value); 4] {
    [
        ("color.main", json!(c.main)),
        ("color.secondary", json!(c.secondary)),
        ("color.gradient", json!(c.gradient)),
        ("color.alternate", json!(c.alternate)),
    ]
    .map(|(k, v)| (k.to_string(), v))
}

/// The display's values (`view.display`, `view.environment`, `light.set`), by name, with
/// whether subsurface runs on this session's device.
/// The window's layout values (`ui.layout`, `ui.library`), each null without a window.
fn layout_values(layout: Option<&LayoutInfo>) -> Vec<(String, Value)> {
    let values = [
        ("layout", json!(layout.map(|l| &l.name))),
        ("layout.hand", json!(layout.map(|l| l.hand))),
        ("layout.scale", json!(layout.map(|l| l.scale))),
        ("layout.library", json!(layout.map(|l| l.library))),
        ("layout.theme", json!(layout.map(|l| &l.theme))),
        ("layout.left", json!(layout.map(|l| l.left))),
        ("layout.right", json!(layout.map(|l| l.right))),
        ("layout.chrome", json!(layout.map(|l| l.chrome))),
        ("layout.one_open", json!(layout.map(|l| l.one_open))),
    ];
    values
        .map(|(k, v)| (k.to_string(), v))
        .into_iter()
        .collect()
}

fn display_values(d: &DisplaySettings, running: Option<bool>) -> Vec<(String, Value)> {
    let s = &d.subsurface;
    let values = [
        ("view.shading", json!(d.shading)),
        ("view.exposure", json!(d.exposure)),
        ("view.transform", json!(d.transform)),
        ("view.occlusion_strength", json!(d.occlusion.strength)),
        ("view.occlusion_radius", json!(d.occlusion.radius)),
        ("view.floor", json!(d.floor)),
        ("view.mask", json!(d.mask.shown)),
        ("view.mask_opacity", json!(d.mask.opacity)),
        ("view.active_outline", json!(d.active_outline)),
        ("view.face_groups", json!(d.face_groups)),
        ("view.matcap_flip", json!(d.matcap_flip)),
        ("view.live_boolean", json!(d.live_boolean)),
        ("view.show_issues", json!(d.show_issues)),
        ("view.show_coplanar", json!(d.show_coplanar)),
        ("view.environment", json!(d.environment_name())),
        ("view.subsurface.mode", json!(s.mode)),
        ("view.subsurface.running", json!(running)),
        ("view.subsurface.strength", json!(s.strength)),
        ("view.subsurface.radius", json!(s.radius)),
        ("view.subsurface.unit_mm", json!(s.unit_mm)),
        ("light.ambient", json!(d.ambient)),
    ]
    .map(|(k, v)| (k.to_string(), v));
    let lights = (d.lights.iter().enumerate()).map(|(i, l)| (format!("light.{}", i + 1), json!(l)));
    values
        .into_iter()
        .chain(lights)
        .chain(d.canvas_values())
        .collect()
}

/// Reference View `view`'s name, links and look; `None` when it is not open.
pub(crate) fn reference_state(views: &Views, view: String) -> Option<ReferenceState> {
    let s = views.secondary(&view)?;
    Some(ReferenceState {
        link: s.link,
        matcap: s.look.matcap.clone(),
        wireframe: s.look.wireframe,
        view,
    })
}

/// Where each part of the session's state lives, for [`state_of`]. Built here only, from the
/// engine's own state for the window's call report, or from a call's context for
/// `session.state`, which borrows the same state.
pub(crate) struct StateSources<'a> {
    settings: &'a BrushSettings,
    control: Control,
    views: &'a Views,
    display: &'a DisplaySettings,
    doc: &'a Document,
    history: Ends,
    palette: &'a crate::MaterialPalette,
    file: &'a FileState,
    autosave: Option<&'a StateFolder>,
    free: &'a FreeSpace,
    render: &'a Shared<dyn RenderHost>,
    window: &'a Shared<dyn WindowHost>,
    gizmos: &'a crate::gizmo::Gizmos,
    live: &'a crate::boolean::Board,
}

impl<'a> StateSources<'a> {
    /// The engine's own state, between calls.
    pub(crate) fn of(state: &'a State, config: &'a EngineConfig) -> Self {
        Self {
            settings: &state.sculpt.settings,
            control: state.control,
            views: &state.views,
            display: &state.display,
            doc: &state.doc,
            history: state.history.ends(),
            palette: &state.palette.now,
            file: &state.file,
            autosave: config.autosave.as_ref(),
            free: &config.free_space,
            render: &state.render,
            window: &state.window,
            gizmos: &state.gizmos,
            live: &state.live,
        }
    }

    /// The state a call sees, as it stands at this step.
    fn of_call(cx: &'a Ctx) -> Self {
        Self {
            settings: &cx.sculpt.settings,
            control: *cx.control,
            views: &*cx.views,
            display: &*cx.display,
            doc: cx.doc.document(),
            history: cx.history.ends(),
            palette: &cx.palette.now,
            file: cx.file,
            autosave: cx.config.autosave.as_ref(),
            free: &cx.config.free_space,
            render: cx.render,
            window: cx.window,
            gizmos: &*cx.gizmos,
            live: cx.live,
        }
    }
}

/// The session's state now, from where each part lives: the brush settings, the session
/// switches, `main`'s settings, the display, Isolate, the renderer's selected look and the
/// window's layout and navigation. Asks the renderer and the window one after the other, never
/// holding both.
pub(crate) fn state_of(sources: StateSources) -> SessionState {
    let StateSources {
        settings,
        control,
        views,
        display,
        doc,
        history,
        palette,
        file,
        autosave,
        free,
        render,
        window,
        gizmos,
        live,
    } = sources;
    let subsurface_running = render.with(|r| r.subsurface_runs(display));
    let (layout, navigation, gizmo, prefs) = window.with(|w| {
        let prefs = w.settings(None).and_then(Result::ok);
        (w.shown(), w.navigation(None), w.gizmo(None), prefs)
    });
    let reference = (views.active_reference()).and_then(|view| reference_state(views, view));
    let brush = settings.state();
    let preset = settings.resolved(&brush.brush).ok();
    let in_effect = BrushInEffect::of(preset.as_ref());
    SessionState {
        in_effect,
        deformer: gizmos.open_deformer(),
        live: live.state(),
        reference,
        live_detail_blocked: preset.and_then(|p| blocked_on_active(&p, doc)),
        brush,
        alpha_recent: settings.recent.list().to_vec(),
        colors: settings.colors.clone(),
        look: Some(palette.active().name.clone()),
        layout,
        paused: control.paused,
        gesture: control.gesture,
        agents_move_view: control.agents_move_view,
        view: views.settings_of(MAIN),
        moves: views.get(MAIN).nav,
        voxel: settings.voxel,
        navigation,
        gizmo,
        settings: prefs.as_ref().map(|p| p.settings),
        launch_brush: prefs.map(|p| p.prefs.launch_brush),
        display: display.clone(),
        subsurface_running,
        isolated: !doc.isolation().chosen.is_empty(),
        active: doc.active().map(str::to_string),
        voxel_mode: (doc.active().and_then(|name| doc.get(name))).map(|p| p.props().voxel_mode),
        levels: (doc.active().and_then(|name| doc.get(name))).map(|p| {
            let (count, active) = p.levels().map_or((1, 0), |l| (l.count(), l.active()));
            [
                count,
                active + 1,
                p.frozen().map_or(0, gs_core::Levels::count),
            ]
        }),
        history: HistoryState::of(history),
        file: (file.path.as_ref()).map(|p| p.display().to_string()),
        autosave: autosave.map(StateFolder::settings),
        recent: autosave.map(|a| crate::file::shown_recent(&a.state_dir)),
        low_disk: crate::file::low_disk(
            free,
            file.path.as_deref(),
            autosave.map(|a| a.state_dir.as_path()),
            doc,
        ),
    }
}

/// Read the session's state as named values: what the window's lit buttons and status line
/// show. `brush` and `brush.*` (set by `brush.set`; `brush.settings` is what this session
/// changed for the brush, `brush.alpha` the alpha it stamps with, `brush.mode` `add` or
/// `subtract`, which way it works before Alt; `brush.depth.imbed`, `.mask`, `.inner`, `.outer`,
/// `.infinite` and `.axes`, `brush.post.gravity` and `.gravity_dir`, and `brush.tip.tilt`,
/// `.constant_tilt`, `.align` and `.alpha.adaptive` (null with no alpha), and
/// `brush.paint.color_strength` (null while the brush paints nothing), each that setting in
/// effect, the preset's own with this session's changes on it; while a held key's slot is held,
/// every `brush.*` value is the slot brush's); `brush.slot.smooth`, `.mask` and `.select`, the
/// brush on each held key's slot, and `brush.slot.held`, the slot held or null (`brush.slot`);
/// `brush.strength_increment` and `.falloff_increment`, how far a step moves for every brush
/// (`brush.set`); `startup.brush`, the brush the window starts with, `last` or a pinned name
/// (`ui.settings`; null without a window), and `startup.pinned`, the pinned name (null with
/// `last` or without a window); `pen.size_sensitivity`, `.strength_sensitivity`,
/// `.color_sensitivity` and `.lazy_pressure` (`pen.set`); `voxel.close_holes`, the hole closing
/// a voxel call leaving it out takes (`mesh.voxel_settings`); `alpha.recent` (the alphas used
/// lately, slot by slot, as the Alpha palette shows them, `alpha.pick`); `color.main` and
/// `color.secondary` (`"#rrggbb"`), `color.gradient` and `color.alternate` (`color.set`);
/// `live_detail.on`, `.mode`,
/// `.size`, `.merge` and `.combined` (`live_detail.set`); `live_detail.eligible` (whether Live
/// Detail can refine the active part under the brush, whether or not it is on; null in an empty
/// scene), `live_detail.blocked` (what stands in the way: `levels`, `uvs`, `brush` for a brush
/// that never refines, `backface`, `face_group` or `topology` for a brush whose automask keeps
/// it off, which dims the window's switch, or `opt_out` for one whose settings turn it off;
/// null when nothing does)
/// and `live_detail.why_not` (the same in a sentence, with what to do about it); `symmetry.on`, the master switch,
/// `symmetry.x`, `.y` and `.z`, each axis on (kept while the master is off), `.x_type`,
/// `.y_type`, `.z_type`, `.radial_axis` (the axis on as Radial, or null) and `.radial_count`
/// (`symmetry.set`, or `brush.set` with `flip_mirror` or `symmetry`); `look` (`view.look` on
/// `main`, or `material.select`; null in a session that cannot render); `layout`, `layout.hand`
/// and `layout.scale` (`ui.layout`; null without a window); `layout.library`, whether the
/// Library drawer is open (`ui.library`; null without a window); `layout.theme`, the theme shown,
/// and `layout.left`, `.right`, `.chrome` and `.one_open` (`ui.layout`; null without a
/// window); `session.paused`, `session.gesture`
/// and `session.agents_move_view` (`session.control`); `view.locked`, `view.orbit`, `view.pivot`,
/// `view.projection` and `view.wireframe`, all of `main` (`view.lock`, `view.settings`,
/// `view.lens`, `view.look`); `view.recenter_rate`, `.snap_deg`, `.auto_distance`,
/// `.align_to_object` and `.right_drag_pivot`, how `main`'s moves behave (`view.settings`);
/// `history.undo` and `history.redo` (the step `history.undo` or
/// `history.redo` would take, by its label: a stroke by its brush, `Draw stroke`, a transform by
/// its operation, `Bend`, a batch by its first command and how many follow, anything else by its
/// command's title, `Add sphere`; null with none), `history.undo_id` (the id `history.list`
/// gives the step `history.undo` would take; null with none) and `history.moved` (`undo` or
/// `redo` while the history's last change was one, so the step just undone is `history.redo`
/// and the one just redone `history.undo`; null after a new step or an open); `history.undos`
/// and `history.redos` (how many steps there are to undo and to redo, which the History buttons
/// count); `file.path` (the scene's own file, null before its first save, `file.save`,
/// `file.open`); `file.recent` (`file.recent`'s files, each with its place, path, name,
/// folder's name and whether it is gone; null with autosave off); `file.low_disk` (null while
/// there is room, else the folder of the scene's file or autosave's with less free space than
/// the larger of 1 GiB and twice the scene file's estimate: its `folder`, `free` bytes and that
/// `threshold`);
/// `autosave.max_minutes`, `.rest_minutes` and `.keep` (`file.autosave`; null with autosave
/// off);
/// `navigation.barrel_click`, `.barrel_two`, `.eraser`, `.classic`, `.orbit_speed`,
/// `.dolly_speed` and `.wheel_step` (`ui.navigation`; null without a window);
/// `settings.sticky_modifiers`,
/// `.status_label`, `.sticky_keys`, `.sticky_delay`, `.ring_on_surface`, `.mirror_rings`,
/// `.show_values`, `.center_snap`, `.falloff_snap`, `.wheel_offset` and `.open_last`
/// (`ui.settings`; null without a window); `gizmo.tool`, `.size`,
/// `.snap`, `.mask` and `.falloff` (`ui.gizmo`; null without a window); `deformer` (the open
/// deformer's first operation, `bend`; null while none is open), `deformer.part`,
/// `deformer.held` (the pen holds one of its handles) and `deformer.transforms` (its arguments
/// now, as the `transform.apply` calls Accept runs) or `deformer.smooth` (as the `mesh.smooth`
/// call, `deformer` then `smooth`) (`gizmo.deformer`); `reference` (the Reference View the window shows,
/// the one opened last or moved to by `reference.next`), `reference.link.rotation`, `.zoom` and `.pan`
/// (`reference.link`), `reference.look` (the MatCap it draws) and `reference.wireframe`
/// (`view.look`), all null while no reference is open; `view.shading`, `view.exposure`,
/// `view.transform`, `view.occlusion_strength`, `view.occlusion_radius` (scene units, or
/// "auto"), `view.floor`, `view.mask` (whether the mask's tint shows), `view.mask_opacity`,
/// `view.active_outline` (whether the window outlines the active part), `view.face_groups`
/// (whether every part shows its face group colors), `view.matcap_flip` (whether MatCaps read
/// mirrored), `view.live_boolean` (whether Live Boolean's live mode is on), `view.show_issues`
/// and `view.show_coplanar` (whether it marks issues and coplanar faces) and
/// `view.subsurface.mode`, `.strength`, `.radius` and `.unit_mm` (`view.display`);
/// `boolean.live` (the live mode's `groups`, each its `operands`, its `state`, `running`,
/// `settled` or `refused`, and when settled its result's `hash`, `faces`, `points` and `solver`,
/// or when refused `why`; and the counts `runs`, `imports`, `cancelled` and `landed`);
/// `view.subsurface.running` (whether subsurface runs on this session's device: `auto` is off
/// on an integrated GPU and on elsewhere; null in a session that cannot
/// render); `view.environment` (`view.environment`; null while each look lights by
/// its own); `light.ambient` and `light.1` to `light.8` (`light.set`); `parts.isolated` (true
/// while Isolate shows only the parts it chose, `part.isolate`); `parts.active` (the active
/// part's name, null in an empty scene, `part.select`); `parts.voxel_mode` (the active part's
/// Voxel mode switch, null in an empty scene, `mesh.voxel_mode`); `parts.levels`, `parts.level`
/// and `parts.frozen` (the active part's detail level count with the base, its active level
/// from 1 at the base, and how many levels `level.freeze` set aside; null in an empty scene).
/// Every name is always there; a value that does not apply is null. Changes nothing.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StateQuery {}

impl Command for StateQuery {
    const NAME: &'static str = "session.state";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = StateValues;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("every state value", Self {})]
    }

    fn run(self, cx: &mut Ctx) -> Result<StateValues, CommandError> {
        Ok(state_of(StateSources::of_call(cx)).values())
    }
}
register_command!(StateQuery, "Session state");

/// The history's two ends as `session.state` names them, for the status line's "Undid Draw
/// stroke".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryState {
    /// The step an undo would undo, by its label (`Draw stroke`, `Add sphere`).
    pub undo: Option<String>,
    /// That step's id, as `history.list` gives it.
    pub undo_id: Option<u64>,
    /// That step's command and arguments, for the window's adjust panel.
    pub undo_step: Option<(String, Arc<Value>)>,
    /// The step a redo would redo, by its label.
    pub redo: Option<String>,
    /// `undo` or `redo` while the last change to the history was one; then the step it moved
    /// over is `redo` or `undo`.
    pub moved: Option<&'static str>,
    /// How many steps there are to undo.
    pub undos: usize,
    /// How many steps there are to redo.
    pub redos: usize,
}

impl HistoryState {
    /// The ends of a history, labelled.
    pub(crate) fn of(ends: Ends) -> Self {
        let label = |(command, args): &(String, Arc<Value>)| step_label(command, args);
        Self {
            undo: ends.undo.as_ref().map(label),
            undo_id: ends.undo_id,
            undo_step: ends.undo,
            redo: ends.redo.as_ref().map(label),
            moved: ends.moved.map(|m| match m {
                Moved::Undo => "undo",
                Moved::Redo => "redo",
            }),
            undos: ends.counts.0,
            redos: ends.counts.1,
        }
    }
}

/// How a step is named for a person: a stroke by its brush (`Draw stroke`), a transform by its
/// operation (`Bend`), a batch by its first command and how many follow (`Add sphere and 2
/// more`), anything else by its command's title (`Add sphere`, where it is registered), or its
/// name where it has none.
fn step_label(command: &str, args: &Value) -> String {
    if command == "brush.stroke"
        && let Some(brush) = args["brush"].as_str()
    {
        let found = crate::sculpt::presets::find(brush).ok();
        return format!("{} stroke", found.map_or(brush, |s| s.preset.name.as_str()));
    }
    if command == "transform.apply"
        && let Some(op) = args["op"].as_object().and_then(|o| o.keys().next())
    {
        let mut chars = op.chars();
        let first = chars.next().map(|c| c.to_ascii_uppercase());
        return first.into_iter().chain(chars).collect();
    }
    if command == gs_protocol::method::BATCH
        && let Some([first, rest @ ..]) = args["commands"].as_array().map(Vec::as_slice)
    {
        let first = step_label(
            first["command"].as_str().unwrap_or_default(),
            &first["args"],
        );
        return match rest.len() {
            0 => first,
            more => format!("{first} and {more} more"),
        };
    }
    let spec = crate::command::spec_named(command).filter(|s| !s.title.is_empty());
    spec.map_or(command, |s| s.title).to_string()
}
