//! The shell's answers to the engine: the window commands (`ui.palette`, `ui.layout`,
//! `ui.wheel`, `ui.quick`, `ui.library`, `ui.navigation`, `ui.gizmo`, `ui.curve`; `ui.settings`
//! is in `prefs`) and what every call did (`after_call`).

use gs_engine::{CallReport, GizmoSettings, LayoutInfo, Navigation, PaletteInfo, UiGizmo};
use gs_engine::{HeldCurve, UiCurve};
use gs_engine::{LibraryInfo, UiLayout, UiLibrary, UiNavigation};
use gs_engine::{PartReport, QuickInfo, UiPalette, UiQuick, UiWheel, WheelInfo};
use gs_protocol::{Role, Step};

use super::{Shell, listed, wheel_slots};
use crate::direction::{Control, Edge, is_shipped, to_toml};
use crate::keymap::write_file;
use crate::pieces::ListedPart;
use crate::record::with_args;
use crate::reference::Shown;
use crate::status::Scene;
use crate::theme::{self, Theme};
use crate::viewport::ActivePart;
use crate::{library, quick, wheel};

impl Shell {
    /// `ui.navigation`: the navigation settings, after `change`.
    pub fn navigation(&mut self, change: Option<&UiNavigation>) -> Navigation {
        let now = self.layout.layout.navigation;
        let next = change.map_or(now, |c| c.apply(now));
        self.layout.layout.navigation = next;
        next
    }

    /// `ui.gizmo`: the gizmo tool's settings, after `change`.
    pub fn gizmo_settings(&mut self, change: Option<&UiGizmo>) -> GizmoSettings {
        let prefs = &mut self.layout.layout.gizmo;
        if let Some(change) = change {
            self.gizmo.change(prefs, change);
        }
        self.gizmo.settings(*prefs)
    }

    /// `ui.curve`: the curve held to paste, after holding `change`'s.
    pub fn curve_held(&mut self, change: Option<&UiCurve>) -> HeldCurve {
        if let Some(copy) = change.and_then(|c| c.copy.clone()) {
            self.copied = Some(copy);
        }
        HeldCurve {
            copied: self.copied.clone(),
        }
    }

    /// `ui.palette`: opens or closes the palette, or asks for a command's arguments (New, Open
    /// and Recover asking about unsaved changes first). Recover with nothing given while a
    /// session waits to be recovered shows its offer again instead (File > Recover…, review
    /// 0009, C10). Quit closes the window as its close button does, asking first while there
    /// are unsaved changes (review 0009, R10). The keymap's form asked for a key opens Bind a
    /// key…, which takes the keys pressed (review 0009, C11).
    pub fn palette_request(&mut self, request: &UiPalette) -> Result<PaletteInfo, String> {
        if !self.palette.open {
            // A palette placed at the pen opens where the pointer is now, and stays there.
            self.palette.at = self.pointer;
        }
        let bare = request.args.is_empty() && request.ask.is_empty();
        let offered = !self.recovery.offers.is_empty();
        match &request.form {
            _ if request.quit => {
                self.palette
                    .first(crate::palette::Then::Close, &self.catalog)?;
            }
            Some(command) if command == "file.recover" && bare && offered => {
                self.recovery.reopen();
            }
            // Bind a key… asking for the key takes it pressed, not typed (review 0009, C11).
            Some(command) if command == "ui.keymap" && asks_key(request) => {
                self.palette.bind_key(&request.args);
            }
            Some(command) if crate::palette::guards(command) => {
                let given = with_args(command, request.args.clone().into());
                self.ask_dropping(given, &request.ask)?;
            }
            Some(command) => {
                let given = with_args(command, request.args.clone().into());
                self.palette.ask(given, &self.catalog, &request.ask)?;
                self.title_by_row(request);
            }
            None => (self.palette).set(request.open, request.query.as_deref()),
        }
        let now = self.now_plain();
        let rows = self.palette.rows(&self.catalog, &listed(&now));
        Ok(PaletteInfo {
            open: self.palette.open,
            query: self.palette.query.clone(),
            matches: rows.iter().map(|r| r.label.clone()).collect(),
            rows,
            form: (self.palette.form.as_ref()).map(|f| f.step.command.clone()),
        })
    }

    /// Titles the form `request` asked for by the row that sends the same request, when one
    /// does: File's Save… in the Part file group asks as Save Part file, not as the scene's
    /// Save (review 0009, R5). A request no row sends keeps the command's title.
    fn title_by_row(&mut self, request: &UiPalette) {
        let mut rows = (self.directions.controls.sets.iter()).flat_map(|s| &s.controls);
        let row = rows.find(|c| asks_as(c, request));
        if let (Some(row), Some(form)) = (row, self.palette.form.as_mut()) {
            form.retitle(&row.label);
        }
    }

    /// The theme the layout state names, or the first if it names none there is.
    pub fn theme_shown(&self) -> &'static Theme {
        (self.themes.find(&self.layout.layout.theme)).unwrap_or_else(theme::first)
    }

    /// `ui.layout` `theme` or `theme_step`: the name of the theme to show.
    fn theme_to_show(&self, change: &UiLayout) -> Result<Option<String>, String> {
        let names: Vec<String> = self.themes.list().iter().map(|t| t.name.clone()).collect();
        if let Some(name) = change.theme.as_ref().filter(|n| !names.contains(n)) {
            let there = names.join(", ");
            return Err(format!("no theme called {name}; there are {there}"));
        }
        let Some(step) = change.theme_step else {
            return Ok(change.theme.clone());
        };
        let at = (names.iter().position(|n| *n == self.theme_shown().name)).unwrap_or(0);
        let count = names.len().max(1);
        let next = if step > 0 { at + 1 } else { at + count - 1 } % count;
        Ok(names.get(next).cloned())
    }

    /// `ui.layout`: shows the layout called `name`, arranges it for a hand, shows a theme, or
    /// writes or reads the settings file (`settings_file.rs`), all or nothing.
    pub fn change_layout(&mut self, change: &UiLayout) -> Result<LayoutInfo, String> {
        let theme = self.theme_to_show(change)?;
        let names: Vec<&str> = (self.directions.list.iter())
            .map(|d| d.name.as_str())
            .collect();
        if names.is_empty() {
            return Err("no layout files are loaded".into());
        }
        // `step`: the next layout or the one before, wrapping.
        let at = names.iter().position(|n| *n == self.shown()).unwrap_or(0);
        let stepped = change.step.map(|step| {
            let len = names.len();
            names[if step > 0 { at + 1 } else { at + len - 1 } % len].to_string()
        });
        if let Some(name) = change.name.as_ref().or(stepped.as_ref()) {
            if !names.contains(&name.as_str()) {
                let there = names.join(", ");
                return Err(format!(
                    "no layout called {name}; there are {there} (`save` saves the one shown \
                     under a name)"
                ));
            }
            // Another layout shows its trays at its own widths.
            if name != self.shown() {
                self.layout.layout.widths.clear();
            }
            self.layout.layout.current.clone_from(name);
        }
        if change.restore {
            let shipped = names.iter().find(|n| is_shipped(n)).unwrap_or(&names[0]);
            self.layout.layout.restore(shipped);
        }
        if let Some(name) = &change.save {
            self.save_arrangement(name)?;
        }
        if let Some(path) = &change.export {
            self.export_settings(path)?;
        }
        if let Some(path) = &change.import {
            self.import_settings(path, &change.parts)?;
        }
        self.layout.layout.hand = change.hand.unwrap_or(self.layout.layout.hand);
        self.layout.layout.ui_scale = change.scale.unwrap_or(self.layout.layout.ui_scale);
        if let Some(theme) = theme {
            self.layout.layout.theme = theme;
        }
        self.show_panels(change);
        Ok(self.layout_info())
    }

    /// `ui.layout` `left`, `right`, `chrome` and `one_open`: which trays and panels show.
    fn show_panels(&mut self, change: &UiLayout) {
        for (edge, shown) in [(Edge::Left, change.left), (Edge::Right, change.right)] {
            let Some(shown) = shown else { continue };
            for id in self.trays_on(edge) {
                self.layout.layout.show(&id, shown);
            }
        }
        let l = &mut self.layout.layout;
        l.minimal = change.chrome.map_or(l.minimal, |c| !c);
        l.one_open = change.one_open.unwrap_or(l.one_open);
    }

    /// The trays of the layout shown on the window's `edge`, by id.
    fn trays_on(&self, edge: Edge) -> Vec<String> {
        let hand = self.layout.layout.hand;
        let panels = self.now_plain().direction.map(|d| d.panels.iter());
        (panels.into_iter().flatten())
            .filter(|p| p.edge.for_hand(hand) == edge)
            .map(|p| p.id.clone())
            .collect()
    }

    /// Whether a tray shows on the window's `edge`: there is one, and it is not hidden.
    fn tray_shown(&self, edge: Edge) -> bool {
        let trays = self.trays_on(edge);
        let hidden = &self.layout.layout.hidden;
        !trays.is_empty() && trays.iter().all(|id| !hidden.contains(id))
    }

    /// `ui.layout` `save`: writes the arrangement shown (the layout with its trays at their
    /// present widths) to `name.toml` in the user's layouts folder, replacing one there, adds
    /// it to the layouts and shows it. A shipped file's name is refused, so a saved
    /// arrangement never hides the Workbench.
    fn save_arrangement(&mut self, name: &str) -> Result<(), String> {
        if is_shipped(name) {
            return Err(format!(
                "{name} is a shipped layout file; save under another name"
            ));
        }
        let dir = (self.layouts.clone()).ok_or("this window has no layouts folder to save in")?;
        let shown = (self.now_plain().direction.cloned()).ok_or("no layout is shown")?;
        let last = self.directions.list.iter().map(|d| d.order).max();
        // Saved again, it keeps its place; a new one comes last.
        let order = (self.directions.get(name)).map_or(last.unwrap_or(0) + 1, |d| d.order);
        let saved = shown.arranged(name, &self.layout.layout.widths, order);
        let text = format!(
            "# {}: {} as it was arranged when saved (`ui.layout` with `save`). The file is\n\
             # yours to edit; saving again replaces it.\n\n{}",
            saved.title,
            shown.title,
            to_toml(&saved)?
        );
        let file = format!("{name}.toml");
        // Checked before it is written, and added once it is: all or nothing.
        let mut directions = self.directions.clone();
        directions.add(&file, &text)?;
        write_file(&dir.join(&file), &text)?;
        self.directions = directions;
        // Its file has the widths now.
        self.layout.layout.widths.clear();
        self.layout.layout.current = name.to_string();
        Ok(())
    }

    /// The layout shown and its hand, for `session.state`; `None` when no layout file is
    /// loaded.
    pub fn shown_layout(&self) -> Option<LayoutInfo> {
        (!self.directions.list.is_empty()).then(|| self.layout_info())
    }

    /// The layout shown, as `ui.layout` answers.
    pub fn layout_info(&self) -> LayoutInfo {
        let d = self.now_plain().direction;
        LayoutInfo {
            name: d.map(|d| d.name.clone()).unwrap_or_default(),
            title: d.map(|d| d.title.clone()).unwrap_or_default(),
            hand: self.layout.layout.hand,
            scale: self.layout.layout.ui_scale,
            layouts: (self.directions.list.iter())
                .map(|d| d.name.clone())
                .collect(),
            library: self.library.open,
            theme: self.theme_shown().name.clone(),
            themes: self.themes.list().iter().map(|t| t.name.clone()).collect(),
            left: self.tray_shown(Edge::Left),
            right: self.tray_shown(Edge::Right),
            chrome: !self.layout.layout.minimal,
            one_open: self.layout.layout.one_open,
        }
    }

    /// `ui.library`: opens or shuts the Library, picks its tab, and pins and unpins.
    pub fn library_action(&mut self, change: &UiLibrary) -> Result<LibraryInfo, String> {
        let tiles: Vec<Step> = {
            let now = self.now_plain();
            let listed = now.direction.map(|d| library::tiles(d, &now));
            listed
                .into_iter()
                .flatten()
                .filter_map(|c| c.step())
                .collect()
        };
        let d = super::shown_direction(&self.directions, &self.layout.layout);
        let favorites = &mut self.layout.layout.favorites;
        self.library.change(change, d, &tiles, favorites)
    }

    /// `ui.wheel`: puts back the layout's slots or a brush in a slot, then opens the brush
    /// wheel at the pointer (the middle of the 3D view if there is none yet; before the first
    /// frame there is no view, and it stays shut), or lets go of it, queueing the command of
    /// the slot aimed at. An error names a brush or slot that does not exist.
    pub fn wheel_action(&mut self, change: &UiWheel) -> Result<WheelInfo, String> {
        if change.restore {
            self.layout.layout.wheel.clear();
        }
        if let (Some(slot), Some(brush)) = (change.slot, &change.brush) {
            self.put_in_wheel(slot as usize, brush)?;
        }
        let (slots, _) = wheel_slots(&self.now_plain());
        let mut picked = None;
        if change.open == Some(false) {
            let control = self.wheel.let_go(self.pointer, &slots);
            picked = control.map(|c| c.label.clone());
            self.outbox.extend(control.and_then(Control::step));
        } else if change.open == Some(true) && !slots.is_empty() {
            let middle = self.viewport.is_finite().then(|| self.viewport.center());
            if let Some(at) = self.pointer.or(middle) {
                self.wheel.open(at, &self.layout.layout);
            }
        }
        Ok(WheelInfo {
            open: self.wheel.at.is_some(),
            picked,
            slots: slots.iter().map(|c| c.label.clone()).collect(),
        })
    }

    /// Puts `brush` in the wheel's slot `slot`, keeping the ring in the layout state.
    fn put_in_wheel(&mut self, slot: usize, brush: &str) -> Result<(), String> {
        if !(self.directions.brush_controls().iter()).any(|c| c.label == brush) {
            return Err(format!("there is no brush named {brush}"));
        }
        let mut ring = wheel::ring(&wheel_slots(&self.now_plain()).0);
        let Some(at) = ring.get_mut(slot) else {
            let n = ring.len();
            return Err(format!(
                "the wheel has {n} slots: give 0 to {}",
                n.saturating_sub(1)
            ));
        };
        brush.clone_into(at);
        self.layout.layout.wheel = ring;
        Ok(())
    }

    /// `ui.quick`: opens the quick menu at the pointer (the middle of the 3D view if there is
    /// none yet) or closes it, shows one of its menus, or changes what one holds
    /// (`quick::change`), or shows one slider alone in its window. A menu that holds nothing
    /// keeps it shut.
    pub fn quick_action(&mut self, change: &UiQuick) -> Result<QuickInfo, String> {
        if let Some(label) = change.slider.as_deref()
            && !quick::is_slider(&self.directions.controls, label)
        {
            return Err(format!(
                "no slider called `{label}`; give a label `ui.describe` reports for one, such \
                 as Size, Strength or Softness"
            ));
        }
        let listed = (self.now_plain().direction).map_or_else(Vec::new, |d| d.quick.clone());
        let directions = &self.directions;
        quick::change(&mut self.layout.layout, &listed, change, |p| {
            directions.knows(p)
        })?;
        let menus = quick::menus(&self.layout.layout, &listed);
        let at = quick::shown(&self.layout.layout, menus.len());
        let middle = self.viewport.is_finite().then(|| self.viewport.center());
        let here = self.pointer.or(middle);
        let open = match (change.open, change.slider.clone()) {
            (_, Some(label)) => here.map(|p| quick::Open::new(p, Some(label))),
            (Some(true), None) => here.map(|p| quick::Open::new(p, None)),
            (Some(false), None) => None,
            (None, None) => self.quick.take(),
        };
        self.quick = open.filter(|o| o.slider.is_some() || !menus[at].is_empty());
        Ok(QuickInfo {
            open: self.quick.is_some(),
            slider: self.quick.as_ref().and_then(|o| o.slider.clone()),
            menu: at + 1,
            menus,
        })
    }

    /// Takes in what a call did: revision, the session's state, parts, and agent activity.
    pub fn after_call(&mut self, report: &CallReport) {
        self.applies.heard(&report.refused);
        self.status.session = report.session.to_string();
        // A read from Blender, by any client, uses up what the folder held.
        if report.ok && report.commands.iter().any(|c| c == "exchange.read") {
            self.exchange.read(&self.status.session);
        }
        self.status.rev = report.rev;
        let before = self.state.history.undo_id;
        (self.outbox).extend(self.adjust.heard(report, before, &self.catalog));
        self.state.clone_from(report.state);
        self.gizmo.heard(&self.state, &report.parts);
        self.prefs.heard |= report.commands.first().is_none_or(|c| c != "pen.input");
        self.heard_scene(report.problem.is_some());
        self.preset.clone_from(&report.preset);
        self.main = report.main;
        self.reference.shown = (report.references.iter())
            .map(|r| Shown {
                setup: r.setup.clone(),
                camera: r.shown,
                live: r.parts.is_none(),
                active: r.active,
            })
            .collect();
        let listed = |p: &PartReport| ListedPart {
            name: p.name.to_string(),
            shown: p.shown,
            visible: p.visible,
            active: p.active,
            painted: p.painted,
        };
        self.parts = report.parts.iter().map(listed).collect();
        self.active_part = ActivePart::of(&report.parts);
        self.status.scene = Scene {
            parts: report.parts.len(),
            points: report
                .parts
                .iter()
                .map(|p| p.store.vertex_count() as u64)
                .sum(),
            faces: report
                .parts
                .iter()
                .map(|p| p.store.face_count() as u64)
                .sum(),
        };
        // Any other call may have moved the gizmo shown (an agent's `gizmo.place`, an edit that
        // moves what it centres on): an empty `ui.gizmo` hands it over again. A deformer still
        // open after its call has moved nothing for good yet (decision 0041, DF7a).
        let open = report.state.deformer.is_some();
        let window = |c: &String| {
            c.starts_with("ui.")
                || c == "pen.input"
                || c == "gizmo.drag"
                || (open && c == "gizmo.deformer")
        };
        let refresh = with_args("ui.gizmo", serde_json::json!({}));
        if self.gizmo.on && !report.commands.iter().all(window) && !self.outbox.contains(&refresh) {
            self.outbox.push(refresh);
        }
        self.heard_person(report, before);
        if report.role != Role::Human && !report.commands.is_empty() {
            let result = if report.ok { "done" } else { "refused" };
            self.status.agent = Some(format!(
                "{} ran {} ({result})",
                report.client,
                report.commands.join(", ")
            ));
        }
    }

    /// Hands the palette what it keeps of the scene (its file, active part, brush and unsaved
    /// changes); once a save the question before unsaved changes are lost waits for lands, what
    /// waited goes on, unless a call `failed`.
    fn heard_scene(&mut self, failed: bool) {
        let scene = &mut self.palette.scene;
        scene.own.clone_from(&self.state.file);
        scene.active.clone_from(&self.state.active);
        scene.brush.clone_from(&self.state.brush.brush);
        scene.unsaved = self.state.unsaved;
        if failed {
            self.palette.unsaved.stop();
        }
        match self.palette.heard(&self.catalog) {
            Ok(Some(now)) => self.queue(now),
            Ok(None) => {}
            Err(problem) => self.status.says(&problem),
        }
    }

    /// What a call that worked did to the problem line and the recovery offer: a step of the
    /// person's that landed in History (a pen stroke) clears the line, as a step sent does
    /// (review 0009, C2); what the report says went wrong (a pen stroke's refusal) is said; and
    /// a recover or a discard of the person's answers its offer. A failed call's report is
    /// passed over: a job's failure comes in a person's words through
    /// [`Status::ended`](crate::status::Status::ended), and a call's own through its refusal.
    fn heard_person(&mut self, report: &CallReport, before: Option<u64>) {
        if !report.ok {
            return;
        }
        let person = report.role == Role::Human;
        if person && self.state.history.undo_id != before {
            (self.status.problem, self.status.ended) = (None, None);
        }
        if let Some(problem) = report.problem {
            self.status.says(problem);
        }
        for command in report.commands.iter().filter(|_| person) {
            self.recovery.landed(command);
        }
    }
}

/// Whether `row` sends `request`: `ui.palette` asking for the same form, with the same
/// arguments given and the same ones asked.
fn asks_as(row: &Control, request: &UiPalette) -> bool {
    let args = serde_json::to_value(&row.args).ok();
    let sent: Option<UiPalette> = args.and_then(|a| serde_json::from_value(a).ok());
    row.command.as_deref() == Some("ui.palette")
        && sent.is_some_and(|s| {
            s.form == request.form && s.args == request.args && s.ask == request.ask
        })
}

/// Whether `request` (for the keymap's form) asks for the key to bind, not given one, and for
/// nothing but what Bind a key… fills in: the key, the command and its arguments.
fn asks_key(request: &UiPalette) -> bool {
    let fills = |a: &String| ["bind", "command", "args"].contains(&a.as_str());
    let asks = request.ask.iter().any(|a| a == "bind") && request.ask.iter().all(fills);
    asks && !request.args.contains_key("bind")
}
