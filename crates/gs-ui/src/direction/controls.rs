//! The shared controls file: named sets of controls, each a label and the command it runs.

use gs_engine::{SessionState, Shows, StateValues};
use gs_protocol::Step;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::split;
use crate::record::with_args;

/// One control: what it reads and what it runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    pub label: String,
    /// The label on a shelf or a narrow header, such as the Reference View's, and on a slider
    /// wherever it is drawn (the slider's value shares its row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    /// The group it is listed under in its set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub args: toml::Table,
    /// Said on hover while it can run, beside what its label says (while it cannot, why shows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// For a feature that is not built yet, what it waits for; it shows, disabled, saying so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub later: Option<String>,
    /// The `session.state` value it shows: it is lit while that value equals `on`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shows: Option<String>,
    /// The `session.state` value its label names: while that value is text, the label reads
    /// "{label} {text}" (Undo reads "Undo Draw stroke" by `history.undo`), else the label alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub names: Option<String>,
    /// The `session.state` number its label ends with while it is above 0: Undo reads "Undo
    /// Draw stroke (3)" by `history.undos` (decision 0016, section 17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counts: Option<String>,
    /// The value of `shows` that lights it; `true` when left out. A table lights it while
    /// `shows` is an object holding those fields (`on = { on = true }` for a light).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<toml::Value>,
    /// For a slider: the lowest and highest it shows. It sets the argument given as `"drag"`
    /// to the value `shows` has, once, when the pen lets go. That argument may sit inside a
    /// table, named by a dotted path (`args = { subsurface.strength = "drag" }`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f32; 2]>,
    /// For a slider: a value it reads as a word, not a number (`[0.0, "endless"]`, decision
    /// 0062).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reads: Option<(f32, String)>,
    /// For a slider: it reads, and takes typed, its value as a percentage (0.5 as 50%).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub percent: bool,
    /// The set its options show: under a header view button, a tray's or a shelf's
    /// (decision 0025, rule 8.5; the Scene set's Save, decision 0016, section 17). A tap still
    /// runs its command; a press held past a click, or a right click, opens them. Options are
    /// one level deep, so that set's controls have none of their own. With no command, the
    /// button only lists them and a tap opens them too (Open a recent scene, section 17, item 7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
    /// The row it shares with the controls of the same `line`, after this label, each showing
    /// its `short` (decision 0060, section 8: `Mirror [X][Y][Z]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    /// It shows only while this holds (decision 0060, section 8's rule 5): a `session.state`
    /// value's name while that value is truthy, a table while each value in it is equal, or a
    /// list while any of its entries holds. The palette lists it either way.
    #[serde(default, rename = "while", skip_serializing_if = "Option::is_none")]
    pub when: Option<While>,
    /// In a header menu, a thin line above it, which starts a group with no heading.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub separator: bool,
    /// A tray's run of it, or of a pick among its options, passes the adjust panel's door
    /// (decision 0060 as amended for S1, section 5): Subdivide opens the panel with its fields.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub adjust: bool,
}

/// When a control shows: its `while`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum While {
    /// While the state value of this name is truthy: not null, false, 0 or empty.
    Holds(String),
    /// While each state value named in it equals its value.
    Equals(toml::Table),
    /// While any entry holds.
    Any(Vec<While>),
}

impl While {
    /// Whether it holds for `state`.
    pub fn holds(&self, state: &StateValues) -> bool {
        match self {
            Self::Holds(name) => state.get(name).is_some_and(|v| {
                !(v.is_null() || *v == json!(false) || v.as_f64() == Some(0.0) || *v == json!(""))
            }),
            Self::Equals(values) => {
                (values.iter()).all(|(name, v)| state.get(name).is_some_and(|s| same(s, &json!(v))))
            }
            Self::Any(any) => any.iter().any(|w| w.holds(state)),
        }
    }

    /// The state values it reads.
    fn names(&self) -> Vec<&str> {
        match self {
            Self::Holds(name) => vec![name],
            Self::Equals(values) => values.keys().map(String::as_str).collect(),
            Self::Any(any) => any.iter().flat_map(Self::names).collect(),
        }
    }
}

impl Control {
    /// Whether it shows with `state` as it is now: always, but for a `while` that does not
    /// hold.
    pub fn shown(&self, state: &StateValues) -> bool {
        self.when.as_ref().is_none_or(|w| w.holds(state))
    }

    /// What it runs as written, if it runs anything.
    pub fn step(&self) -> Option<Step> {
        let args = serde_json::to_value(&self.args).unwrap_or_default();
        Some(with_args(self.command.as_deref()?, args))
    }

    /// What it runs with `state` as it is now: an argument given as `"flip"` is false while
    /// it is lit and true while it is not, one given as `{ state = "name" }` is that state value
    /// (left out while it is null), and a slider asks for its value in the palette.
    pub fn step_in(&self, state: &StateValues) -> Option<Step> {
        let command = self.command.as_deref()?;
        if let Some(arg) = self.dragged() {
            return Some(with_args(
                "ui.palette",
                json!({ "form": command, "ask": [arg] }),
            ));
        }
        let mut args = self.args.clone();
        flip(&mut args, !self.lit(state).unwrap_or(false));
        let mut args = serde_json::to_value(args).unwrap_or_default();
        read(&mut args, state);
        Some(with_args(command, args))
    }

    /// The argument a slider sets: the one given as `"drag"`, by its dotted path
    /// (`subsurface.strength` inside the table `subsurface`). `None` for any other control.
    pub fn dragged(&self) -> Option<String> {
        self.range?;
        drag_path(&self.args)
    }

    /// A control running `command` with `args` (a JSON object).
    pub fn runs(label: &str, command: &str, args: &Value) -> Self {
        Self {
            label: label.into(),
            command: Some(command.into()),
            args: serde_json::from_value(args.clone()).unwrap_or_default(),
            ..Self::default()
        }
    }

    /// The same control, lit while the state value `value` is `on`.
    #[must_use]
    pub fn showing(self, value: &str, on: &str) -> Self {
        Self {
            shows: Some(value.into()),
            on: Some(on.into()),
            ..self
        }
    }

    /// `label` followed by the text of the state value it names and the number it counts in
    /// brackets, by `state`; `None` when neither is there (no text, a count of 0).
    pub fn named(&self, label: &str, state: &StateValues) -> Option<String> {
        let value = |name: &Option<String>| state.get(name.as_deref()?);
        let text = value(&self.names).and_then(Value::as_str);
        let count = value(&self.counts)
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        let named = text.map(|text| format!("{label} {text}"));
        match count {
            Some(n) => Some(format!("{} ({n})", named.as_deref().unwrap_or(label))),
            None => named,
        }
    }

    /// What it shows, if it shows a state value.
    pub fn shows(&self) -> Option<Shows> {
        let on = self.on.as_ref().map_or(json!(true), |v| json!(v));
        Some(Shows {
            value: self.shows.clone()?,
            on,
        })
    }

    /// Whether it is lit by `state`; `None` for a control that shows no state value.
    pub fn lit(&self, state: &StateValues) -> Option<bool> {
        let shows = self.shows()?;
        Some(state.get(&shows.value).is_some_and(|v| same(v, &shows.on)))
    }
}

/// Sets each argument given as `"flip"` in `args` to `to`, looking inside tables.
fn flip(args: &mut toml::Table, to: bool) {
    for (_, v) in args.iter_mut() {
        if v.as_str() == Some("flip") {
            *v = to.into();
        } else if let Some(t) = v.as_table_mut() {
            flip(t, to);
        }
    }
}

/// Sets each argument given as `{ state = "name" }` in `args` to the state value `name`, looking
/// inside tables, and leaves it out while that value is null (Fill's Color Strength while the
/// brush paints nothing).
fn read(args: &mut Value, state: &StateValues) {
    let Some(fields) = args.as_object_mut() else {
        return;
    };
    fields.retain(|_, v| {
        if let Some(name) = reads(v).map(str::to_string) {
            *v = state.get(&name).cloned().unwrap_or_default();
            return !v.is_null();
        }
        read(v, state);
        true
    });
}

/// The state value an argument reads: `name` when it is `{ state = "name" }`.
fn reads(v: &Value) -> Option<&str> {
    let fields = v.as_object().filter(|f| f.len() == 1)?;
    fields.get("state")?.as_str()
}

/// Every state value `args` reads, looking inside tables.
fn read_names(args: &Value) -> Vec<&str> {
    match (reads(args), args.as_object()) {
        (Some(name), _) => vec![name],
        (None, Some(fields)) => fields.values().flat_map(read_names).collect(),
        (None, None) => Vec::new(),
    }
}

/// The dotted path to the argument given as `"drag"` in `args`, looking inside tables.
fn drag_path(args: &toml::Table) -> Option<String> {
    args.iter().find_map(|(k, v)| match v {
        toml::Value::String(s) if s == "drag" => Some(k.clone()),
        toml::Value::Table(t) => Some(format!("{k}.{}", drag_path(t)?)),
        _ => None,
    })
}

/// Sets the argument at the dotted `path` in `args` (`a.b`: `b` inside the table `a`) to
/// `value`, making the tables on the way.
pub fn put(args: &mut Value, path: &str, value: Value) {
    let mut at = args;
    for key in path.split('.') {
        if !at.is_object() {
            *at = json!({});
        }
        at = &mut at[key];
    }
    *at = value;
}

/// Two state values are the same; numbers compare as numbers, so `on = 4` matches 4.0, and an
/// object matches `b` when it holds `b`'s fields.
fn same(a: &Value, b: &Value) -> bool {
    if let (Value::Object(a), Value::Object(b)) = (a, b) {
        return (b.iter()).all(|(k, v)| a.get(k).is_some_and(|x| same(x, v)));
    }
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y).is_eq(),
        _ => a == b,
    }
}

/// A named list of controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Set {
    pub name: String,
    pub title: String,
    /// Its controls run for one row of a list, which fills in `"@row"` with the row's item:
    /// they show only on the rows, so the command palette does not list them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub row: bool,
    /// The settings window's page it is on (decision 0052); left out, the page of the piece
    /// before it in the layout's `settings`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    pub controls: Vec<Control>,
}

/// The shared controls file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Controls {
    #[serde(rename = "set", default)]
    pub sets: Vec<Set>,
    /// What each brush family's shelf button says on hover, by family (`[families]`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub families: std::collections::BTreeMap<String, String>,
}

impl Controls {
    pub fn set(&self, name: &str) -> Option<&Set> {
        self.sets.iter().find(|s| s.name == name)
    }

    /// The controls a piece names (`set`, `set/Group`, or `set/` for those with no group), if it
    /// names a set.
    pub fn piece(&self, piece: &str) -> Option<(&Set, Vec<&Control>)> {
        let (name, group) = split(piece);
        let set = self.set(name)?;
        let picked = (set.controls.iter())
            .filter(|c| group.is_none_or(|g| c.group.as_deref().unwrap_or_default() == g))
            .collect();
        Some((set, picked))
    }

    /// Whether a tray's `step` passes the adjust panel's door: a control marked `adjust` runs
    /// it, or lists it among its options.
    pub fn adjusts(&self, step: &Step) -> bool {
        let marked = (self.sets.iter().flat_map(|s| &s.controls)).filter(|c| c.adjust);
        let listed = |c: &Control| {
            let set = c.options.as_deref().and_then(|name| self.set(name));
            set.into_iter().flat_map(|s| &s.controls)
        };
        let runs = |c: &Control| c.step().as_ref() == Some(step);
        marked
            .flat_map(|c| std::iter::once(c).chain(listed(c)))
            .any(runs)
    }

    /// Every control runs a command, waits for a feature or only lists its options, and shows
    /// and names only state values `session.state` reports.
    pub(super) fn check(&self) -> Result<(), String> {
        let names = SessionState::names();
        for set in &self.sets {
            for c in &set.controls {
                let at = format!("set `{}`, control `{}`", set.name, c.label);
                let lists = c.options.is_some() && c.command.is_none() && c.later.is_none();
                if c.command.is_some() == c.later.is_some() && !lists {
                    return Err(format!(
                        "{at}: give `command`, `later` for a feature not built yet, or only \
                         `options` for a button that lists them"
                    ));
                }
                let read = [
                    ("shows", c.shows.as_deref()),
                    ("names", c.names.as_deref()),
                    ("counts", c.counts.as_deref()),
                ];
                let when = c
                    .when
                    .iter()
                    .flat_map(While::names)
                    .map(|n| ("while", Some(n)));
                for (field, value) in read.into_iter().chain(when) {
                    if let Some(value) = value.filter(|v| !names.iter().any(|n| n == v)) {
                        return Err(format!(
                            "{at}: {field}: no state value called `{value}`; session.state has {}",
                            names.join(", ")
                        ));
                    }
                }
                let args = serde_json::to_value(&c.args).unwrap_or_default();
                if let Some(value) = read_names(&args)
                    .into_iter()
                    .find(|v| !names.iter().any(|n| n == v))
                {
                    return Err(format!(
                        "{at}: args: no state value called `{value}` to read"
                    ));
                }
                if c.on.is_some() && c.shows.is_none() {
                    return Err(format!("{at}: on: give `shows`, the value it is for"));
                }
                if c.range.is_some() && (c.shows.is_none() || c.dragged().is_none()) {
                    let message = "a slider shows a value and sets one argument given as \"drag\"";
                    return Err(format!("{at}: range: {message}"));
                }
                if let Some(name) = &c.options {
                    let Some(inner) = self.set(name) else {
                        return Err(format!("{at}: options: no set called `{name}`"));
                    };
                    if inner.controls.iter().any(|c| c.options.is_some()) {
                        let message = "options are one level deep, so its controls have none";
                        return Err(format!("{at}: options: `{name}`: {message}"));
                    }
                }
            }
        }
        Ok(())
    }
}
