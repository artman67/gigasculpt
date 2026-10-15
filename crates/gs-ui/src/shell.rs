//! The interface itself (Workbench, decision 0025). It draws the status line, the layout's
//! panels (`arrange`), the header with its menus and view buttons (`header`), the 3D
//! view with the cursor, the Look options,
//! the settings window (`settings`), the quick menu (`quick`), the Library (`library`), the brush
//! wheel and the command palette, with egui, one region at a time in the order `regions` lists.
//! It only draws and records; every action it takes leaves as a command (`take_commands`) for
//! the engine to run, so nothing here is out of reach of agents. The layout file (`direction`)
//! says where the shared pieces (`pieces`) go. Its answers to the engine's window commands and
//! call reports are in `answers`.

use gs_core::brush::BrushPreset;
use gs_core::curve::Curve;
use gs_engine::{MainView, Materials, PaletteEntry, PanelInfo, SessionState};
use gs_protocol::{Catalog, PenSample, Step};
use gs_render::{Camera, Preset};
use serde_json::{Value, json};

use crate::applies::Applies;
use crate::arrange::places;
use crate::blender::Exchange;
use crate::direction::{Control, Direction, Directions};
use crate::gizmo::Tool;
use crate::held::Held;
use crate::keymap::Keymap;
use crate::keys::{Keys, Reads};
use crate::layout::{Layout, LayoutFile};
use crate::library::Library;
use crate::navigate::Navigator;
use crate::palette::{Listed, Palette};
use crate::pieces::{Dragging, ListedPart, Now, all_controls};
use crate::preferences::Preferences;
use crate::record::{Recorder, with_args};
use crate::recovery::Recovery;
use crate::reference::RefPanel;
use crate::settings::SettingsWindow;
use crate::status::Status;
use crate::theme::{self, Theme, Themes, fonts};
use crate::viewport::{ActivePart, Cursor, Detail, symmetry_text};
use crate::wheel::Wheel;
use regions::REGIONS;

mod answers;
mod early;
pub(crate) mod escape;
mod files;
mod prefs;
pub(crate) mod refusals;
mod regions;
mod settings_file;

/// Sets the fonts on a context the first time the shell draws in it, and shows the theme
/// `shown` in it (with egui's style) whenever it is not the one it has.
fn prepare(ctx: &egui::Context, shown: &'static Theme) {
    let has = theme::shown(ctx);
    if has.is_none() {
        ctx.set_fonts(fonts());
    }
    if has.map(|t| t.name.as_str()) != Some(shown.name.as_str()) {
        theme::show(ctx, shown);
        ctx.all_styles_mut(|style| shown.apply(style));
    }
}

/// The interface's state between frames.
pub struct Shell {
    pub catalog: Catalog,
    pub keymap: Keymap,
    /// The user's `keymap.toml`, where `ui.keymap` writes the changes; `None`: nowhere.
    pub keymap_file: Option<std::path::PathBuf>,
    /// Which layout shows, its hand, and the interface scale.
    pub layout: LayoutFile,
    /// The user's preferences (decision 0052); kept in no file unless the window's start
    /// loaded one.
    pub prefs: Preferences,
    /// The layout files and the controls they share.
    pub directions: Directions,
    /// The user's `layouts` folder, where `ui.layout` `save` writes; `None`: nowhere to save.
    pub layouts: Option<std::path::PathBuf>,
    pub palette: Palette,
    pub wheel: Wheel,
    pub status: Status,
    /// Crashed sessions the window offers to recover, asked once at start.
    pub recovery: Recovery,
    /// The parts, as the engine last reported them.
    pub parts: Vec<ListedPart>,
    /// The active part's box and transform, for the symmetry planes.
    pub active_part: Option<ActivePart>,
    /// The gesture map's state between frames.
    pub nav: Navigator,
    /// The window's view, as the engine last reported it: what the 3D view draws.
    pub main: MainView,
    /// The pen's press landed on empty canvas and has not lifted: its drag navigates.
    pub canvas: bool,
    /// The pen's cursor: the pen while it is in range, the smoothing string and the dabs.
    pub cursor: Cursor,
    /// The session's state as the engine last reported it (`session.state`): the brush the pen
    /// strokes with, the look, the layout, the session switches. Toggles are lit from it.
    pub state: SessionState,
    /// The brush the pen strokes with, as a stroke uses it, as the engine last reported it.
    pub preset: Option<BrushPreset>,
    /// The curve the curve editor's Copy holds to paste (`ui.curve`).
    pub copied: Option<Curve>,
    /// The MatCaps, as the renderer last listed them.
    pub looks: Materials,
    /// The palette's materials with their recipes, as the renderer last had them.
    pub materials: Vec<PaletteEntry>,
    /// The themes it can show: the shipped ones, then the user's.
    pub themes: Themes,
    /// The Reference View the window shows, and its panel's state between frames.
    pub reference: RefPanel,
    /// The gizmo tool: on or off, the gizmo it shows, and what the pen's press does to it.
    pub gizmo: Tool,
    /// The settings window: open, its page, its search and Reset's question (`ui.settings`).
    settings: SettingsWindow,
    /// Where the quick menu is open (`ui.quick`), in points.
    pub quick: Option<crate::quick::Open>,
    /// The header's menu open, and its submenu (`ui.menu`).
    pub menus: crate::header::Menus,
    /// The adjust panel, and the commands the window's doors handed on (`ui.adjust`).
    pub adjust: crate::adjust::Adjust,
    /// The Library drawer: open or shut, its tab and its thumbnails (`ui.library`).
    pub library: Library,
    /// The Blender exchange folders, and what the window has counted as read from Blender.
    pub exchange: Exchange,
    /// The keymap's keys in progress, the held keys' brush slot and sticky modifier keys.
    pub keys: Keys,
    /// A slider sets a brush value (a slider key held, a slider shown alone).
    keeping: bool,
    /// While `keeping`: the cursor as it was when that began, where the brush's circle stays
    /// (decision 0025 section 2, as amended 2026-10-12); `None` if the pen was not over the 3D
    /// view then.
    kept: Option<Cursor>,
    dragging: Dragging,
    /// The pointer, in points, as of the last frame that saw it.
    pointer: Option<egui::Pos2>,
    outbox: Vec<Step>,
    panels: Vec<PanelInfo>,
    /// Which of the controls drawn can run now, as the engine last said.
    applies: Applies,
    viewport: egui::Rect,
}

/// What the pieces see, from the parts of the shell they read. `state` is the session's state
/// as shown, with any brush value being set in place.
fn now<'a>(
    directions: &'a Directions,
    layout: &'a Layout,
    keymap: &'a Keymap,
    (looks, themes): (&'a Materials, &'a Themes),
    parts: &'a [ListedPart],
    state: &'a SessionState,
    held: Option<Held>,
) -> Now<'a> {
    let direction = shown_direction(directions, layout);
    Now {
        brush: &state.brush,
        preset: None,
        copied: None,
        state: state.values(),
        looks,
        materials: &[],
        themes,
        parts,
        keymap,
        directions,
        layout,
        direction,
        held,
        applies: None,
        gizmo: None,
    }
}

/// The layout `layout` shows: its current one, or the first.
fn shown_direction<'a>(directions: &'a Directions, layout: &Layout) -> Option<&'a Direction> {
    (directions.list.iter())
        .find(|d| d.name == layout.current)
        .or(directions.list.first())
}

/// Every control the layouts know, for the palette to find, and where those of the layout
/// shown live.
fn listed<'a>(now: &'a Now) -> Listed<'a> {
    let controls = (all_controls(now).into_iter())
        .filter_map(|c| Some((c.label.clone(), c.step_in(&now.state)?)))
        .collect();
    let places = now.direction.map(|d| places(d, now)).unwrap_or_default();
    Listed {
        keymap: now.keymap,
        controls,
        places,
    }
}

/// The brush wheel's slots and radius in the layout shown.
fn wheel_slots(now: &Now) -> (Vec<Control>, f32) {
    crate::wheel::slots(now)
}

/// The status line's first parts: the brush, symmetry and what the held keys do.
fn sculpt_line(now: &Now) -> Vec<String> {
    let brush = format!("Brush: {}", now.brush.brush);
    let shown = now.layout.settings.status_label;
    let held = now.held.as_ref().filter(|_| shown).map(Held::text);
    let all = [Some(brush), Some(symmetry_text(now.brush)), held];
    all.into_iter().flatten().collect()
}

impl Shell {
    /// A shell showing the shipped layouts.
    pub fn new(catalog: Catalog, keymap: Keymap, layout: LayoutFile) -> Self {
        let mut status = Status::default();
        let directions = Directions::shipped().unwrap_or_else(|e| {
            status.says(&e);
            Directions::default()
        });
        Self {
            catalog,
            keymap,
            keymap_file: None,
            layout,
            prefs: Preferences::in_memory(),
            directions,
            layouts: None,
            palette: Palette::default(),
            wheel: Wheel::default(),
            status,
            recovery: Recovery::default(),
            parts: Vec::new(),
            active_part: None,
            nav: Navigator::default(),
            main: MainView {
                camera: Camera::preset(Preset::Front),
                locked: false,
                pivot: [0.0; 3],
                moving: false,
            },
            canvas: false,
            cursor: Cursor::default(),
            state: SessionState::default(),
            preset: None,
            copied: None,
            looks: Materials::default(),
            materials: Vec::new(),
            themes: Themes::default(),
            reference: RefPanel::default(),
            gizmo: Tool::default(),
            settings: SettingsWindow::default(),
            quick: None,
            menus: crate::header::Menus::default(),
            adjust: crate::adjust::Adjust::default(),
            library: Library::default(),
            exchange: Exchange::default(),
            keys: Keys::default(),
            keeping: false,
            kept: None,
            dragging: None,
            pointer: None,
            outbox: Vec::new(),
            panels: Vec::new(),
            applies: Applies::default(),
            viewport: egui::Rect::NOTHING,
        }
    }

    /// Starts a frame: egui's style, if the theme changed, whether Read from Blender has
    /// anything new to read, the level steps that would change nothing, and whether the
    /// brush's automask dims the Live Detail switch.
    fn start(&mut self, ctx: &egui::Context) {
        prepare(ctx, self.theme_shown());
        // The window's job that ended refused, said as a refusal at once is.
        if let Some((step, e)) = self.status.ended.take() {
            self.refused(step, &e);
        }
        let why = self.exchange.nothing_new(&self.status.session);
        self.applies.hold("exchange.read", why);
        for (step, why) in crate::pieces::level_ends(&self.state) {
            self.applies.hold_step(step, why);
        }
        let none = self.recovery.offers.is_empty();
        let why =
            none.then(|| "No session that closed without saving waits to be recovered".into());
        self.applies.hold_step(crate::recovery::row(), why);
        let (shows, why) = Detail::switch(&self.state);
        self.applies.idle(shows, why);
    }

    /// Draws one frame of the interface: its regions, in order (see `regions`).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.start(ui.ctx());
        let rec = Recorder::new(ui.ctx().pixels_per_point());
        // Every key to the one thing in front, kept once a frame before any region or key sees
        // it (`escape`): Escape taken out of the frame, Enter, Tab and the arrows given, and the
        // rest for a piece that takes keys of its own to ask (`in_front`).
        self.escape(ui.ctx());
        self.route(ui.ctx());
        self.keys(ui);
        // With the gizmo tool on, a press works the gizmo and Live Detail has no say.
        let detail = Detail::of(&self.state, self.preset.as_ref());
        self.cursor.detail = detail.filter(|_| !self.gizmo.on);
        self.pointer = ui.ctx().pointer_latest_pos().or(self.pointer);
        // The state as shown: the engine's, with a brush value being set in place.
        let mut shown = self.state.clone();
        shown.brush = self.keys.live(&self.state.brush);
        if let Some((value, at)) = self.dragging {
            value.put(&mut shown.brush, at);
        }
        let over = self.cursor.pen.is_some_and(|c| self.over_view(&c));
        let kept = self.kept_cursor(over);
        let brush_shown = over || kept.is_some();
        let slots = &self.state.brush.slots;
        // A stroke keeps the slot it pressed with: the cursor says what it does, not the keys.
        let mut m = ui.input(|i| i.modifiers);
        if self.cursor.stroking && !self.gizmo.on {
            m = self.keys.slots.stroking(m);
        }
        let pen = self.cursor.pen.filter(|_| over);
        let view = (self.viewport, &self.main.camera);
        let ppp = ui.ctx().pixels_per_point();
        let gizmo = (self.gizmo).in_use(self.layout.layout.gizmo, pen, view, ppp);
        let held = Held::of(m, gizmo, self.preset.as_ref(), slots);
        // The frame draws the kept cursor; the pen's own comes back after it.
        let live = kept.map(|k| std::mem::replace(&mut self.cursor, k));
        let mut frame = self.frame((&shown, held), brush_shown, rec);
        for region in REGIONS {
            region(&mut frame, ui);
        }
        // Tab from a field moves the keyboard on (`escape`), and the keymap would stop on the
        // widget it lands on: outside a text field the window keeps it, so the keys go on
        // working and Space never presses that widget. `escape::route` cannot: egui moves the
        // keyboard while the regions draw, after it. Pinned by the test
        // `front::tab_from_a_field_leaves_no_button_for_space_to_press`.
        if ui.input(|i| i.key_pressed(egui::Key::Tab)) && !ui.ctx().text_edit_focused() {
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        let drawn = frame.end();
        if let Some(live) = live {
            self.cursor = live;
        }
        self.viewport = drawn.view;
        self.finish(drawn.asked, drawn.rec, (drawn.fold, drawn.width));
        escape::keep_field(ui.ctx());
    }

    /// While a slider sets a brush value (a slider key held, a slider shown alone), the cursor
    /// the brush's circle is drawn from: kept where the pen was when that began, if it was
    /// `over` the 3D view then, at the value being set and with the pen's own state otherwise,
    /// so the circle grows and shrinks in place while the pen works the slider (decision 0025
    /// section 2, as amended 2026-10-12).
    fn kept_cursor(&mut self, over: bool) -> Option<Cursor> {
        let alone = self.quick.as_ref().is_some_and(|q| q.slider.is_some());
        if !alone && !self.keys.holding() {
            (self.keeping, self.kept) = (false, None);
            return None;
        }
        if !self.keeping {
            self.keeping = true;
            self.kept = over.then(|| self.cursor.clone());
        }
        let kept = self.kept.as_ref()?;
        Some(Cursor {
            pen: kept.pen,
            surface: kept.surface,
            dabs: kept.dabs.clone(),
            slider_alone: alone,
            ..self.cursor.clone()
        })
    }

    /// Ends a frame: queues what it asked to run, keeps what it drew for `ui.describe`, folds
    /// the section it folded, keeps the width a tray was dragged to, and saves the layout file
    /// and the preferences file if that or a call changed them.
    fn finish(
        &mut self,
        asked: Vec<Step>,
        rec: Recorder,
        (fold, width): (Vec<String>, Option<(String, f32)>),
    ) {
        for step in asked {
            self.queue(step);
        }
        self.panels = rec.finish();
        for id in fold {
            self.layout.layout.fold(&id);
        }
        if let Some((id, width)) = width {
            self.layout.layout.widths.insert(id, width);
        }
        if let Err(problem) = self.layout.save_if_changed() {
            self.status.says(&problem);
        }
        self.keep_preferences();
        // A recover the offer sent waits while it is queued, runs as a job, or waits on the
        // question about unsaved changes.
        let (outbox, unsaved) = (&self.outbox, &self.palette.unsaved);
        let running = self.status.busy.as_ref().and_then(|b| b.step.as_ref());
        let waits = |command: &str| {
            let mut steps = outbox.iter().chain(running).chain(unsaved.waiting());
            steps.any(|s| s.command == command)
        };
        self.recovery.settle(waits);
    }

    /// Runs the keymap on this frame's keys, unless a text field has the keyboard or the
    /// palette is open (it takes its own keys); an open brush wheel takes the letters its
    /// slots start with first.
    fn keys(&mut self, ui: &egui::Ui) {
        // The held modifiers hold a brush slot, unless they are typing or work the gizmo, and
        // not while a stroke runs (Shift after the press draws a straight line) or the pen is
        // down or lifting elsewhere, so a slider drag sets the brush it started on.
        let typing = self.palette.open || ui.ctx().egui_wants_keyboard_input();
        let (m, now, dt, down) = ui.input(|i| {
            let down = i.pointer.any_down() || i.pointer.any_released();
            (i.modifiers, i.time, i.predicted_dt, down)
        });
        // The open deformer's lift, and its Enter when nothing in front takes it (`escape`).
        let enter = escape::entered(ui.ctx(), escape::To::Deformer);
        let held = self.state.deformer.as_ref().map(|d| d.held);
        (self.outbox).extend(crate::gizmo::deformer::keys(held, down, enter));
        let m = if typing || self.gizmo.on {
            egui::Modifiers::NONE
        } else {
            m
        };
        let settings = self.layout.layout.settings;
        if !self.cursor.stroking && !down {
            let sticky = settings.sticky_modifiers;
            self.outbox.extend(self.keys.slots.frame(m, now, sticky));
        }
        if !typing {
            let mut events = ui.input(|i| i.events.clone());
            escape::unbind_given(ui.ctx(), &mut events);
            if self.wheel.at.is_some() {
                let (slots, _) = wheel_slots(&self.now_plain());
                events.retain(|e| !self.wheel.letter(e, &slots));
            }
            let ppp = ui.ctx().pixels_per_point();
            // With no pointer known (it left the window), a key that works off the view runs.
            let pointer = ui.ctx().pointer_latest_pos();
            let reads = Reads {
                keymap: &self.keymap,
                state: &self.state,
                controls: &self.directions.controls,
                ppp,
                now,
                settings,
                over_view: pointer.is_some_and(|p| self.view_at(p * ppp)),
            };
            let steps = crate::pieces::picker::keyed(ui.ctx(), self.keys.frame(&events, &reads));
            self.outbox.extend(self.adjust.offer(steps));
            // A slider key's activation draws a frame of its own; egui takes a frame off.
            if let Some(secs) = self.keys.wake(now, &settings) {
                ui.ctx().request_repaint_after_secs(secs as f32 + dt);
            }
        }
        self.cursor.adjusting = self.keys.adjusting();
    }

    /// Takes the wheel notches the keymap binds out of the next frame's `events` before egui
    /// sees them, so nothing scrolls or dollies for them, and asks for their commands; not while
    /// `typing` or with the palette open, as with every key.
    pub fn wheel_keys(&mut self, events: &mut Vec<egui::Event>, typing: bool) {
        if typing || self.palette.open {
            return;
        }
        let (keys, keymap, outbox) = (&mut self.keys, &self.keymap, &mut self.outbox);
        let seen = (&self.state, &self.directions.controls);
        events.retain(|e| {
            let egui::Event::MouseWheel {
                delta, modifiers, ..
            } = e
            else {
                return true;
            };
            let steps = keys.notch(delta.y, *modifiers, keymap, seen);
            let bound = steps.is_some();
            outbox.extend(steps.into_iter().flatten());
            !bound
        });
    }

    /// Asks for `step` as if the interface had: it runs with the next commands taken. A gizmo
    /// drag's or an open deformer's update replaces one of its own still waiting, which it
    /// carries on from (each says the whole transform since the drag or the press began).
    pub fn queue(&mut self, step: Step) {
        let deforms = ["gizmo.drag", "gizmo.deformer"].contains(&step.command.as_str());
        let update = |s: &Step| s.command == step.command && s.args["phase"] == "update";
        if deforms && update(&step) && self.outbox.last().is_some_and(update) {
            self.outbox.pop();
        }
        self.outbox.push(step);
    }

    /// The commands the interface asked for since the last call, in order, a control's view
    /// `reference` named (see [`RefPanel::resolve`]) and its exchange folder given by name
    /// (see [`Exchange::resolve`]). The person's next step clears the problem line (review
    /// 0009, C2): it succeeds, or its own refusal says why. A form kept for the step it sent
    /// lives until that step's answer ([`Palette::taken`]).
    pub fn take_commands(&mut self) -> Vec<Step> {
        let running = self.status.busy.as_ref().and_then(|b| b.step.as_ref());
        self.palette.taken(running);
        let mut steps = std::mem::take(&mut self.outbox);
        if steps.iter().any(refusals::moves_on) {
            (self.status.problem, self.status.ended) = (None, None);
        }
        self.reference.resolve(&mut steps, self.pointer);
        self.exchange
            .resolve(&mut steps, &self.status.session, true);
        let gesture = self.state.gesture;
        steps.extend(crate::gizmo::deformer::lifted(
            &mut self.status.waiting,
            gesture,
        ));
        steps
    }

    /// The window's start, from `file.recovery`'s answer and `file.recent`'s (`None` when
    /// refused, as with autosave off): a crashed session is offered back (decision 0016,
    /// section 10); with none offered, `open_last` on and the newest recent scene still there,
    /// it queues `file.open` with `recent: 1`, as Open recent's first row does, for the first
    /// frame to run (section 17).
    pub fn launched(&mut self, found: Option<&Value>, recent: Option<&Value>) {
        if let Some(found) = found {
            self.recovery = Recovery::from_result(found);
        }
        let newest = recent.and_then(|r| r["files"].get(0));
        let there = newest.is_some_and(|f| f["gone"] == false);
        // The preferences file's, which the first frame has not run yet.
        let kept = self
            .prefs
            .get("settings.open_last")
            .and_then(|v| v.as_bool());
        let open_last = kept.unwrap_or(self.layout.layout.settings.open_last);
        if there && self.recovery.offers.is_empty() && open_last {
            self.outbox
                .push(with_args("file.open", json!({ "recent": 1 })));
        }
    }

    /// Panels and widgets of the last frame.
    pub fn panels(&self) -> &[PanelInfo] {
        &self.panels
    }

    /// What the controls of the last frame run, each once, for the engine to say which cannot
    /// run now.
    pub fn controls(&mut self) -> Vec<Step> {
        let (reference, exchange) = (&self.reference, &self.exchange);
        let session = &self.status.session;
        self.applies.ask(&self.panels, |steps| {
            reference.resolve(steps, None);
            exchange.resolve(steps, session, false);
        })
    }

    /// Whether the pen at `s` is over the 3D view and nothing else (the panel on top under it in
    /// the last frame is the view) and no palette or header menu is open: where it sculpts and
    /// shows the brush. A press while a menu is open only shuts it (`header::early`).
    pub fn over_view(&self, s: &PenSample) -> bool {
        let open = self.palette.open || self.menus.shows();
        !open && self.view_at(egui::pos2(s.x as f32, s.y as f32))
    }

    /// Whether the panel on top at `at`, in pixels, in the last frame is the 3D view.
    fn view_at(&self, at: egui::Pos2) -> bool {
        let area =
            |r: [f32; 4]| egui::Rect::from_min_max(egui::pos2(r[0], r[1]), egui::pos2(r[2], r[3]));
        let top = (self.panels.iter().rev()).find(|p| area(p.rect).contains(at));
        top.is_some_and(|p| p.id == "viewport")
    }

    /// The 3D view's area in the last frame, in points.
    pub fn viewport(&self) -> egui::Rect {
        self.viewport
    }

    /// The layout shown, by name; empty when no layout file is loaded.
    pub fn shown(&self) -> &str {
        let now = self.now_plain();
        now.direction.map_or("", |d| d.name.as_str())
    }

    fn now_plain(&self) -> Now<'_> {
        let (l, k) = (&self.layout.layout, &self.keymap);
        now(
            &self.directions,
            l,
            k,
            (&self.looks, &self.themes),
            &self.parts,
            &self.state,
            None,
        )
    }
}
