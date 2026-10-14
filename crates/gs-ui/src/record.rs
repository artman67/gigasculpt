//! What the interface drew this frame, as `ui.describe` reports it: panels, their widgets, the
//! command each widget runs and its key, and the trays' sections.

use gs_engine::{PanelInfo, SectionInfo, Shows, WidgetInfo};
use gs_protocol::Step;
use serde_json::{Value, json};

/// A widget's role, as `ui.describe` names it ([`ROLE_NAMES`], in the same order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Button,
    Toggle,
    Slider,
    TextField,
    ListItem,
    Label,
    ScrollBar,
}

/// What `ui.describe` calls each [`Role`].
const ROLE_NAMES: [&str; 7] = [
    "button",
    "toggle",
    "slider",
    "text field",
    "list item",
    "label",
    "scroll bar",
];

impl Role {
    fn name(self) -> &'static str {
        ROLE_NAMES[self as usize]
    }
}

/// Collects panels and widgets as they are drawn. Rectangles arrive in points and are stored
/// in physical pixels. Inside a list that scrolls, each is cut to what the list shows (a widget
/// says so), and one scrolled wholly out of sight is left out, so a tap at a reported rectangle
/// lands on it.
#[derive(Debug, Default)]
pub struct Recorder {
    pixels_per_point: f32,
    panels: Vec<PanelInfo>,
    /// What the list being drawn shows, in points, while one is.
    shown: Option<egui::Rect>,
    /// Whether the widget offered last was recorded (one scrolled out of sight is not).
    recorded: bool,
}

/// A command with no arguments.
pub fn step(command: &str) -> Step {
    with_args(command, json!({}))
}

/// A command with arguments.
pub fn with_args(command: &str, args: Value) -> Step {
    Step {
        command: command.into(),
        args,
    }
}

impl Recorder {
    /// Starts a frame drawn at `pixels_per_point`.
    pub fn new(pixels_per_point: f32) -> Self {
        Self {
            pixels_per_point,
            panels: Vec::new(),
            shown: None,
            recorded: false,
        }
    }

    fn pixels(&self, rect: egui::Rect) -> [f32; 4] {
        let r = rect * self.pixels_per_point;
        [r.min.x, r.min.y, r.max.x, r.max.y]
    }

    /// `rect` cut to what the list being drawn shows, in pixels; `None` when none of it shows.
    fn seen(&self, rect: egui::Rect) -> Option<[f32; 4]> {
        match self.shown {
            None => Some(self.pixels(rect)),
            Some(shown) => {
                let cut = shown.intersect(rect);
                cut.is_positive().then(|| self.pixels(cut))
            }
        }
    }

    /// Widgets and sections recorded next are in a list showing only `shown` (in points), or
    /// in none with `None`.
    pub fn list(&mut self, shown: Option<egui::Rect>) {
        self.shown = shown;
    }

    /// What the list being drawn shows, as [`Recorder::list`] last set it: a list inside
    /// another gives it back when it ends.
    pub fn listing(&self) -> Option<egui::Rect> {
        self.shown
    }

    /// Starts a panel; widgets recorded next belong to it.
    pub fn panel(&mut self, id: &str, title: &str, rect: egui::Rect) {
        self.panels.push(PanelInfo {
            id: id.into(),
            title: title.into(),
            rect: self.pixels(rect),
            widgets: Vec::new(),
            sections: Vec::new(),
        });
    }

    /// Records a section of the current panel, drawn in `rect`; `lit` for a header view button
    /// whose state value is on.
    pub fn section(&mut self, id: &str, title: &str, folded: bool, lit: bool, rect: egui::Rect) {
        let Some(rect) = self.seen(rect) else {
            return;
        };
        if let Some(panel) = self.panels.last_mut() {
            panel.sections.push(SectionInfo {
                id: id.into(),
                title: title.into(),
                folded,
                lit,
                rect,
            });
        }
    }

    /// Gives the current panel its rectangle once drawn: a floating panel's size is only known
    /// after its contents.
    pub fn fit(&mut self, rect: egui::Rect) {
        let rect = self.pixels(rect);
        if let Some(panel) = self.panels.last_mut() {
            panel.rect = rect;
        }
    }

    /// Records a widget of the current panel.
    pub fn widget(&mut self, widget: Widget, rect: egui::Rect) {
        self.recorded = false;
        let cut = self.shown.is_some_and(|shown| !shown.contains_rect(rect));
        let Some(rect) = self.seen(rect) else {
            return;
        };
        if let Some(panel) = self.panels.last_mut() {
            panel.widgets.push(WidgetInfo {
                label: widget.label,
                role: widget.role.name().into(),
                command: widget.command,
                key: widget.key,
                enabled: widget.enabled,
                selected: widget.selected,
                shows: widget.shows,
                reason: widget.reason,
                hint: None,
                rect,
                cut,
            });
            self.recorded = true;
        }
    }

    /// Gives the widget recorded last what hovering it says; nothing when it was left out.
    pub fn hint(&mut self, hint: &str) {
        let recorded = self.recorded;
        let last = self.panels.last_mut().and_then(|p| p.widgets.last_mut());
        if let Some(w) = last.filter(|_| recorded) {
            w.hint = Some(hint.into());
        }
    }

    /// Everything recorded.
    pub fn finish(self) -> Vec<PanelInfo> {
        self.panels
    }
}

/// One widget as `ui.describe` reports it.
pub struct Widget {
    pub label: String,
    pub role: Role,
    pub command: Option<Step>,
    pub key: Option<String>,
    pub enabled: bool,
    pub selected: bool,
    /// For a toggle, the state value it shows.
    pub shows: Option<Shows>,
    /// Why it is dimmed, when it is: disabled, or idle (it still runs).
    pub reason: Option<String>,
}

impl Widget {
    /// A widget that runs nothing.
    pub fn plain(label: impl Into<String>, role: Role) -> Self {
        Self {
            label: label.into(),
            role,
            command: None,
            key: None,
            enabled: true,
            selected: false,
            shows: None,
            reason: None,
        }
    }

    /// The same widget as a toggle showing the state value `shows`, lit when `lit`.
    #[must_use]
    pub fn toggle(self, shows: Shows, lit: bool) -> Self {
        Self {
            role: Role::Toggle,
            shows: Some(shows),
            selected: lit,
            ..self
        }
    }

    /// The same widget, disabled because of `why`.
    #[must_use]
    pub fn disabled(self, why: impl Into<String>) -> Self {
        Self {
            enabled: false,
            reason: Some(why.into()),
            ..self
        }
    }

    /// The same widget, dimmed because it has no effect now (`why`); it still runs (decision
    /// 0025, section 6, amended 2026-10-12).
    #[must_use]
    pub fn idle(self, why: impl Into<String>) -> Self {
        let reason = Some(why.into());
        Self { reason, ..self }
    }

    /// The same widget, highlighted or switched on when `selected`.
    #[must_use]
    pub fn selected(self, selected: bool) -> Self {
        Self { selected, ..self }
    }

    /// A widget that runs `command`, bound to `key`.
    pub fn runs(label: impl Into<String>, role: Role, command: Step, key: Option<&str>) -> Self {
        Self {
            command: Some(command),
            key: key.map(Into::into),
            ..Self::plain(label, role)
        }
    }
}
