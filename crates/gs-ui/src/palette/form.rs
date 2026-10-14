//! The palette's form: a command waiting for its arguments, each drawn by its kind (decision
//! 0025, rule 13), so a choice is never typed. A word is a lit button in a row, a boolean a
//! switch, a number between two ends a slider, a part a pick of the scene's parts and a color a
//! color button; only a value (a path, a name, a number with no ends) is typed. The arguments
//! asked for come first, then the rest under More, then Run, which reads the command's title.
//! A boolean an engine's refusal asks for is a plain question instead, with two buttons. Each
//! choice is recorded with the step Run would send with it chosen.

mod fields;
mod kind;
mod words;

use std::collections::BTreeMap;

use egui::RichText;
use gs_protocol::{CommandInfo, Step};
use serde_json::{Value, json};

use crate::pieces::{Face, Now, Press, Reads, label, pressed_as};
use crate::record::{Recorder, Role, Widget, with_args};
use crate::theme::theme;
use fields::argument;
use kind::Kind;
use words::{friendly, hint, yes};

/// How tall More's arguments show before they scroll.
const MORE: f32 = 360.0;

/// A command waiting for its arguments.
#[derive(Clone, Debug)]
pub struct Form {
    /// The command and the arguments it already has.
    pub step: Step,
    /// Each argument asked for (the required ones the step lacks, then those named in `ask`),
    /// and what is typed for it.
    pub fields: Vec<(String, String)>,
    /// The command's other arguments but the structured ones, under More, and what is typed
    /// for each.
    pub more: Vec<(String, String)>,
    /// What a control chose (a word, a switch, a slider, a part, a color), by argument.
    pub chosen: BTreeMap<String, Value>,
    /// Whether More is open.
    pub open: bool,
    /// Asked by an engine's refusal (decision 0016, section 17), where a boolean is a question.
    pub refused: bool,
    /// A field and why the engine refused its value, shown under it (the adjust panel's).
    pub reason: Option<(String, String)>,
    /// The command's title, which Run reads.
    title: String,
    /// The command's summary, which heads the form.
    summary: String,
    /// How each argument shown is drawn, by name.
    args: BTreeMap<String, Arg>,
}

/// An argument as the form draws it.
#[derive(Clone, Debug)]
struct Arg {
    kind: Kind,
    /// Its description on one line.
    about: String,
    /// What shows under its name and on hover ([`hint`]).
    hint: String,
    /// Its default, which shows as chosen.
    default: Option<Value>,
    /// Required: Run waits for it.
    required: bool,
    /// Left out, the engine takes the active part for it (`pinned`).
    active: bool,
    /// It may be left out (its schema takes null), the engine then settling it.
    optional: bool,
    /// The other arguments' values it belongs with (`x-when`, a sphere's Subdivisions with the
    /// cube form): with another shown, it is the other form's, and neither shows nor is sent.
    when: Option<serde_json::Map<String, Value>>,
}

/// What the form's buttons asked for.
pub(super) enum Answer {
    /// Run this step.
    Run(Step),
    /// Close the form, running nothing.
    Cancel,
}

impl Form {
    /// The form for `step`, a command of `info`, asking `fields`, each with what it starts
    /// with.
    pub(super) fn new(step: Step, info: &CommandInfo, fields: Vec<(String, String)>) -> Form {
        let schema = &info.args_schema;
        let required = schema["required"].as_array().into_iter().flatten();
        let required: Vec<&str> = required.filter_map(Value::as_str).collect();
        let arg = |name: &str| {
            let defs = &schema["$defs"];
            let raw = kind::at(schema, name);
            let resolved = raw.map(|s| kind::resolve(s, defs));
            let about = raw.and_then(|s| s["description"].as_str());
            let about = about.or_else(|| resolved?["description"].as_str());
            let about = about
                .unwrap_or_default()
                .replace('\n', " ")
                .replace('`', "");
            let kind = raw.map_or(Kind::Typed, |s| kind::of(s, defs));
            Arg {
                hint: hint(&about, &kind),
                kind,
                about,
                default: raw.map(|s| s["default"].clone()).filter(|v| !v.is_null()),
                required: required.contains(&name),
                active: info.pinned.iter().any(|p| p == name),
                optional: raw.is_some_and(|s| kind::optional(s, defs)),
                when: raw.and_then(|s| s["x-when"].as_object()).cloned(),
            }
        };
        let asked =
            |name: &str| step.args.get(name).is_some() || fields.iter().any(|(n, _)| n == name);
        let others = schema["properties"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.keys());
        let more: Vec<(String, String)> = (others.filter(|n| !asked(n.as_str())))
            .filter(|n| !matches!(arg(n.as_str()).kind, Kind::Given))
            .map(|n| (n.clone(), String::new()))
            .collect();
        let args = (fields.iter().chain(&more))
            .map(|(n, _)| (n.clone(), arg(n.as_str())))
            .collect();
        let title = spec(&step.command).map_or_else(|| step.command.clone(), |c| c.title.into());
        Form {
            step,
            fields,
            more,
            chosen: BTreeMap::new(),
            open: false,
            refused: false,
            reason: None,
            title,
            summary: info.summary.clone(),
            args,
        }
    }

    /// The form of a step already run, for the adjust panel: each argument but the pinned and
    /// the given ones, filled from the step, with no More. The step's own arguments come first
    /// (those it has, and those never left out), in the order the command declares them, then
    /// the rest by name (decision 0062): a remesh's Resolution first, a sphere's form first.
    pub fn adjusting(step: Step, info: &CommandInfo) -> Form {
        let props = info.args_schema["properties"].as_object();
        let shown = props.into_iter().flat_map(|p| p.keys());
        let shown = shown.filter(|n| !info.pinned.contains(n));
        let fields = shown.map(|n| (n.clone(), String::new())).collect();
        let mut form = Form::new(step.clone(), info, fields);
        let args = &form.args;
        form.fields
            .retain(|(n, _)| args.get(n).is_some_and(|a| !matches!(a.kind, Kind::Given)));
        let declared = spec(&step.command).map_or(&[][..], |s| s.fields());
        let own = |n: &str| step.args.get(n).is_some() || args.get(n).is_some_and(|a| !a.optional);
        let at = |n: &str| declared.iter().position(|d| *d == n);
        // A stable sort, so the rest keep their order by name.
        form.fields
            .sort_by_key(|(n, _)| (!own(n), at(n).filter(|_| own(n)).unwrap_or(usize::MAX)));
        form.more.clear();
        form.fill(step);
        form
    }

    /// Fills the fields from `step`, which the form holds from then on: each typed as the step
    /// has it, nothing chosen.
    pub fn fill(&mut self, step: Step) {
        for (name, text) in &mut self.fields {
            *text = match step.args.pointer(&pointer(name)) {
                None | Some(Value::Null) => String::new(),
                Some(Value::String(s)) => s.clone(),
                Some(value) => value.to_string(),
            };
        }
        self.chosen.clear();
        self.step = step;
    }

    /// The first field given a value other than the step's, by name.
    pub fn changed(&self) -> Option<String> {
        let mut fields = self.fields.iter();
        let differs = |(name, text): &&(String, String)| {
            let had = self.step.args.pointer(&pointer(name));
            self.given(name, text).is_some_and(|v| Some(&v) != had)
        };
        fields.find(differs).map(|(name, _)| name.clone())
    }

    /// The command's title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Draws each field by its kind, and under the refused one why. Returns whether a slider
    /// took a typed value.
    pub fn arguments(
        &mut self,
        ui: &mut egui::Ui,
        focus: &mut bool,
        now: &Now,
        rec: &mut Recorder,
    ) -> bool {
        let mut set = false;
        for name in names(&self.fields) {
            if self.unused(&name) {
                continue;
            }
            set |= argument(ui, self, &name, focus, now, rec);
            if let Some((_, why)) = self.reason.as_ref().filter(|(n, _)| *n == name) {
                let problem = theme(ui.ctx()).colors.problem;
                label(ui, RichText::new(why).small().color(problem), rec);
            }
        }
        set
    }

    /// The command with what is chosen and typed. An argument left alone is left out, so its
    /// default applies (the active part, say); a dotted one (a slider's
    /// `subsurface.strength`) goes inside its table; one of a form not chosen goes, as the
    /// engine refuses it beside the other form's (a sphere made with Subdivisions, made again
    /// as uv).
    pub fn step(&self) -> Step {
        let mut args = self.step.args.clone();
        for (name, text) in self.fields.iter().chain(&self.more) {
            if let Some(value) = self.given(name, text) {
                crate::direction::put(&mut args, name, value);
            }
        }
        if let Some(all) = args.as_object_mut() {
            all.retain(|name, _| !self.unused(name));
        }
        with_args(&self.step.command, args)
    }

    /// Whether `name` belongs with other values of the arguments its `x-when` names than those
    /// shown: the other form's.
    fn unused(&self, name: &str) -> bool {
        let when = self.args.get(name).and_then(|a| a.when.as_ref());
        when.is_some_and(|w| {
            (w.iter()).any(|(other, value)| self.shows(other).is_some_and(|v| v != *value))
        })
    }

    /// What `name` is given: its control's choice, else `text` as its kind reads it (text as
    /// typed, numbers as a list, anything else as JSON where it reads as JSON); `None` while
    /// nothing is.
    fn given(&self, name: &str, text: &str) -> Option<Value> {
        if let Some(value) = self.chosen.get(name) {
            return Some(value.clone());
        }
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let json = |t: &str| serde_json::from_str(t).unwrap_or_else(|_| Value::from(text));
        Some(match self.args.get(name).map(|a| &a.kind) {
            Some(Kind::Text) => Value::from(text),
            Some(Kind::Numbers | Kind::Three(_)) => {
                let bare = text.trim_start_matches('[').trim_end_matches(']');
                json(&format!("[{bare}]"))
            }
            _ => json(text),
        })
    }

    /// What `name` shows: what it is given, else what the step has, else its default.
    fn shows(&self, name: &str) -> Option<Value> {
        let had = || self.step.args.pointer(&pointer(name)).cloned();
        let default = || self.args.get(name)?.default.clone();
        (self.given(name, self.text(name)).or_else(had)).or_else(default)
    }

    /// The rail of each slider the form shows in `ui`: the room beside the widest label any of
    /// them shows, at most the theme's slider width and at least half of it, so each has its
    /// label beside it as Levels' does, the rails lined up (decision 0025, rule 13, amended
    /// 2026-10-13). A label too wide even for half goes above its rail, as rule 12 has it.
    fn rail(&self, ui: &egui::Ui) -> f32 {
        let wide = |text: String| {
            let (wrap, style) = (Some(egui::TextWrapMode::Extend), egui::TextStyle::Body);
            let text = egui::WidgetText::from(text);
            text.into_galley(ui, wrap, f32::INFINITY, style).size().x
        };
        let names = self
            .fields
            .iter()
            .chain(&self.more)
            .map(|(n, _)| n.as_str());
        let labels = names
            .filter(|n| !self.unused(n))
            .flat_map(|n| self.readings(n));
        let widest = labels.map(wide).fold(0.0, f32::max);
        let width = ui.spacing().slider_width;
        let room = ui.available_width() - widest - ui.spacing().item_spacing.x;
        room.clamp(width / 2.0, width)
    }

    /// The labels the slider of `name` shows, as [`running`] reads them: its title and its
    /// value at either end or as it is, "auto" while nothing gives an optional one; none when
    /// it is not a slider.
    fn readings(&self, name: &str) -> Vec<String> {
        let Some(arg) = self.args.get(name) else {
            return Vec::new();
        };
        let Kind::Slider(ends, whole) = arg.kind else {
            return Vec::new();
        };
        let shown = self.shows(name).and_then(|v| v.as_f64());
        let rail = ends.map(|v| v as f32);
        let word = (arg.optional && shown.is_none()).then_some((rail[0], "auto"));
        let reads = Reads::from((whole, word));
        let at = shown.map_or(rail[0], |v| v as f32);
        let title = friendly(name);
        [rail[0], rail[1], at]
            .map(|v| format!("{title}  {}", reads.read(v, rail)))
            .into()
    }

    /// The step Run would send with `value` chosen for `name`.
    fn with(&self, name: &str, value: Value) -> Step {
        let mut form = self.clone();
        form.chosen.insert(name.into(), value);
        form.step()
    }

    /// What is typed for `name`.
    fn text(&self, name: &str) -> &str {
        let mut all = self.fields.iter().chain(&self.more);
        all.find(|(n, _)| n == name).map_or("", |(_, t)| t.as_str())
    }

    /// Why Run cannot send the step yet: a required argument that nothing gives.
    fn waits(&self) -> Option<String> {
        let args = self
            .fields
            .iter()
            .filter_map(|(n, _)| Some((n, self.args.get(n)?)));
        let mut lacking = args.filter(|(n, a)| a.required && self.shows(n).is_none());
        let (name, arg) = lacking.next()?;
        Some(match arg.kind {
            Kind::Given => format!("Waits for {}, which a script gives", friendly(name)),
            _ => format!("Waits for {}", friendly(name)),
        })
    }

    /// The boolean a refusal asked for, when that is all the form asks: a question.
    fn question(&self) -> Option<String> {
        match self.fields.as_slice() {
            [(name, _)] if self.refused => {
                let arg = self.args.get(name)?;
                matches!(arg.kind, Kind::Switch).then(|| name.clone())
            }
            _ => None,
        }
    }
}

/// The form: each argument by its kind, the More fold and Run; or a question's line and its two
/// buttons. Returns what was asked for: Run's step (on Enter too, but not the Enter that set a
/// slider's typed value), or Cancel.
pub(super) fn draw(
    ui: &mut egui::Ui,
    form: &mut Form,
    focus: &mut bool,
    enter: bool,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Answer> {
    if let Some(name) = form.question() {
        return question(ui, form, &name, now, rec);
    }
    label(ui, RichText::new(&form.summary).weak(), rec);
    let mut set = form.arguments(ui, focus, now, rec);
    if !form.more.is_empty() {
        set |= more(ui, form, focus, now, rec);
    }
    ui.separator();
    let step = form.step();
    if let Some(why) = form.waits() {
        let shown = ui.add_enabled(false, egui::Button::new(&form.title));
        let shown = shown.on_disabled_hover_text(&why);
        let widget = Widget::runs(&form.title, Role::Button, step, None);
        rec.widget(widget.disabled(why), shown.rect);
        return None;
    }
    let held = now.applies.and_then(|a| a.why(&step)).is_some();
    let run = pressed_as(ui, plain(&form.title, step.clone()), Face::Label, now, rec);
    (run.clicked() || (enter && !set && !held)).then_some(Answer::Run(step))
}

/// More: the command's other arguments, shut at first, scrolling when long. Returns whether a
/// slider in it took a typed value.
fn more(
    ui: &mut egui::Ui,
    form: &mut Form,
    focus: &mut bool,
    now: &Now,
    rec: &mut Recorder,
) -> bool {
    let header = ui.selectable_label(form.open, RichText::new("More").strong());
    rec.section("more", "More", !form.open, false, header.rect);
    if header.clicked() {
        form.open = !form.open;
    }
    if !form.open {
        return false;
    }
    let mut set = false;
    let outer = rec.listing();
    // The palette's window has no height of its own, so the area's is set, not left to fit.
    let area = egui::ScrollArea::vertical()
        .id_salt("form more")
        .max_height(MORE)
        .min_scrolled_height(MORE);
    area.show(ui, |ui| {
        rec.list(Some(ui.clip_rect()));
        for name in names(&form.more) {
            if !form.unused(&name) {
                set |= argument(ui, form, &name, focus, now, rec);
            }
        }
    });
    rec.list(outer);
    set
}

/// A question an engine's refusal asked: the boolean's description on a line, then a button
/// that sends it true and Cancel. Enter answers neither.
fn question(
    ui: &mut egui::Ui,
    form: &Form,
    name: &str,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Answer> {
    let about = form.args.get(name).map_or("", |a| a.about.as_str());
    label(ui, RichText::new(about), rec);
    let yes = yes(name, about);
    let step = form.with(name, Value::Bool(true));
    let cancel = with_args("ui.palette", json!({ "open": false }));
    let mut answer = None;
    ui.horizontal(|ui| {
        if pressed_as(ui, plain(&yes, step.clone()), Face::Label, now, rec).clicked() {
            answer = Some(Answer::Run(step));
        }
        if pressed_as(ui, plain("Cancel", cancel), Face::Label, now, rec).clicked() {
            answer = Some(Answer::Cancel);
        }
    });
    answer
}

/// A button that runs `step`, unlit.
fn plain(label: &str, step: Step) -> Press<'_> {
    Press {
        label,
        step,
        key: None,
        lit: false,
        shows: None,
    }
}

/// The engine's registration of `command`.
fn spec(command: &str) -> Option<&'static gs_engine::CommandSpec> {
    gs_engine::registry()
        .into_iter()
        .find(|c| c.name == command)
}

/// The names in `list`.
fn names(list: &[(String, String)]) -> Vec<String> {
    list.iter().map(|(n, _)| n.clone()).collect()
}

/// The JSON pointer of a dotted argument name.
fn pointer(name: &str) -> String {
    format!("/{}", name.replace('.', "/"))
}
