//! The layout state: a plain file with the layout shown (one of the layout files, see
//! `direction.rs`), the hand it is arranged for, the trays' folded sections and dragged widths,
//! the gizmo's size and snap (`ui.gizmo`), the Library's favorites (`ui.library`), Isolate's
//! options and the Parts list's rows (`ui.settings`), the brush wheel's edited slots
//! (`ui.wheel`) and which panels show, saved whenever they change.
//!
//! The interface scale, the theme, the navigation settings and the settings the settings window
//! shows are kept in `preferences.toml` (decision 0052 as amended 2026-10-12): still read from
//! an older file, which the window's start hands to `preferences.toml` where it lacks them
//! before it writes this file back without them (`app::shell`), and never written here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gs_engine::{Hand, Navigation, WindowSettings};
use gs_protocol::Step;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::direction::parse;
use crate::gizmo::GizmoPrefs;
use crate::keymap::write_file;

/// What the window shows and at what size.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layout {
    /// The layout file shown, by name. Empty or unknown: the first one.
    pub current: String,
    /// The mode shown, written by versions before decision 0025 (which has no modes): still
    /// read, so those files load, and never written.
    #[serde(rename = "mode", skip_serializing)]
    pub old_mode: IgnoredAny,
    /// The hand that holds the pen; `left` mirrors the window.
    pub hand: Hand,
    /// The trays' sections folded shut, by id (the piece each shows).
    pub folded: Vec<String>,
    /// The trays dragged wider or narrower than their layout file has them: width in points,
    /// by panel id. Showing another layout clears them, so it shows at its own widths.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub widths: BTreeMap<String, f32>,
    /// Interface size on top of the display's own scale: 1 is normal. Kept in
    /// `preferences.toml`: read from an older file, never written.
    #[serde(skip_serializing)]
    pub ui_scale: f32,
    /// How pen and mouse drags on the 3D view navigate (decision 0017). Kept in
    /// `preferences.toml`: read from an older file, never written.
    #[serde(skip_serializing)]
    pub navigation: Navigation,
    /// The gizmo's size on screen and how a Shift drag snaps its turns (decision 0027).
    pub gizmo: GizmoPrefs,
    /// The window's own settings (`ui.settings`): sticky modifier keys, the held keys' label, how
    /// sliders and the brush wheel behave. Only those the settings window does not show are
    /// written; the others are kept in `preferences.toml`.
    #[serde(serialize_with = "unlisted")]
    pub settings: WindowSettings,
    /// The brush wheel's slots as edited (`ui.wheel` `slot`), by brush name, the top one
    /// first; empty, the layout's own.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wheel: Vec<String>,
    /// The Library's favorites, in the order the shelf shows them: what each pinned tile runs
    /// (decision 0025, section 13), a reference to the item and never a copy of it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub favorites: Vec<Step>,
    /// The theme shown, by name (`ui.layout` `theme`). Empty or unknown: the first. Kept in
    /// `preferences.toml`: read from an older file, never written.
    #[serde(skip_serializing)]
    pub theme: String,

    // What shows (interface.layouts, `ui.layout` `left`, `right`, `chrome` and `one_open`).
    /// The trays hidden, by panel id, so a tray hidden stays hidden on either edge.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// Minimal chrome: the shelf, the trays and the header hidden at once,
    /// `hidden` kept.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub minimal: bool,
    /// One section open at a time in each tray, unless Shift is held.
    pub one_open: bool,

    // The quick menus (interface.quick-menu, `ui.quick`).
    /// What each quick menu holds, by piece, when the user has changed them; empty means one
    /// menu, the layout file's `quick` list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub quick: Vec<Vec<String>>,
    /// The quick menu shown, from 0.
    #[serde(skip_serializing_if = "is_zero")]
    pub quick_menu: usize,
}

/// Whether `n` is 0, for leaving it out of the file.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The window's settings the settings window does not show, which only the layout state keeps:
/// Isolate's options. A page that lists one moves it out by dropping it here (the Parts list's
/// rows, since S1).
const UNLISTED: [&str; 2] = ["dynamic_isolate", "click_isolate"];

/// Writes only the window's [`UNLISTED`] settings.
fn unlisted<S: serde::Serializer>(s: &WindowSettings, to: S) -> Result<S::Ok, S::Error> {
    let mut kept = fields(json!(s));
    kept.retain(|k, _| UNLISTED.contains(&k.as_str()));
    kept.serialize(to)
}

/// The fields of a struct as `json!` writes it.
fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            current: String::new(),
            old_mode: IgnoredAny,
            hand: Hand::Right,
            // The falloff curve and the depth settings are seldom changed; they start shut.
            folded: vec!["curve".into(), "depth".into()],
            widths: BTreeMap::new(),
            ui_scale: 1.0,
            navigation: Navigation::default(),
            gizmo: GizmoPrefs::default(),
            settings: WindowSettings::default(),
            wheel: Vec::new(),
            favorites: Vec::new(),
            theme: String::new(),
            hidden: Vec::new(),
            minimal: false,
            one_open: true,
            quick: Vec::new(),
            quick_menu: 0,
        }
    }
}

impl Layout {
    /// Folds the section `id` shut, or opens it if it is folded.
    pub fn fold(&mut self, id: &str) {
        if self.is_folded(id) {
            self.folded.retain(|f| f != id);
        } else {
            self.folded.push(id.into());
        }
    }

    /// Whether the section `id` is folded shut.
    pub fn is_folded(&self, id: &str) -> bool {
        self.folded.iter().any(|f| f == id)
    }

    /// `bar`, one of the window's own bars, unless minimal chrome hides it.
    pub fn chrome<T>(&self, bar: Option<T>) -> Option<T> {
        bar.filter(|_| !self.minimal)
    }

    /// Whether the panel `id` shows: not hidden, and not under minimal chrome.
    pub fn shows(&self, id: &str) -> bool {
        !self.minimal && !self.hidden.iter().any(|h| h == id)
    }

    /// Shows or hides the panel `id`.
    pub fn show(&mut self, id: &str, shown: bool) {
        self.hidden.retain(|h| h != id);
        if !shown {
            self.hidden.push(id.into());
        }
    }

    /// Shows the layout `current` as at first (`ui.layout` `restore`): its sections folded as
    /// they start, its trays at their own widths, every panel showing and one section open at
    /// a time. The hand, the scale, the theme and the window's other settings stay.
    pub fn restore(&mut self, current: &str) {
        let first = Self::default();
        self.current = current.into();
        (self.folded, self.widths, self.hidden) = (first.folded, first.widths, first.hidden);
        (self.minimal, self.one_open) = (first.minimal, first.one_open);
    }

    /// What this layout holds of the settings kept in `preferences.toml`, by their
    /// `session.state` names, where it differs from the defaults: what an older `layout.toml`
    /// hands over at start (decision 0052 as amended by step Prefs-c). A new file holds none.
    pub fn moved(&self) -> Vec<(String, Value)> {
        let named = |l: &Self| {
            let mut all = vec![
                ("layout.scale".to_string(), json!(l.ui_scale)),
                ("layout.theme".to_string(), json!(l.theme)),
            ];
            let navigation = fields(json!(l.navigation)).into_iter();
            all.extend(navigation.map(|(k, v)| (format!("navigation.{k}"), v)));
            let settings = fields(json!(l.settings)).into_iter();
            let listed = settings.filter(|(k, _)| !UNLISTED.contains(&k.as_str()));
            all.extend(listed.map(|(k, v)| (format!("settings.{k}"), v)));
            all
        };
        let first = named(&Self::default());
        named(self)
            .into_iter()
            .filter(|n| !first.contains(n))
            .collect()
    }
}

/// The layout and the file it lives in.
#[derive(Clone, Debug)]
pub struct LayoutFile {
    pub layout: Layout,
    path: Option<PathBuf>,
    /// What the file holds now.
    saved: Layout,
}

impl LayoutFile {
    /// Reads `path`; a missing file means the defaults, written there.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut file = Self::read(path)?;
        file.write()?;
        Ok(file)
    }

    /// Reads `path` without writing it back; a missing file means the defaults. The window's
    /// start hands what moved to `preferences.toml` before it writes this file (`app::shell`).
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let mut layout: Layout = match std::fs::read_to_string(path) {
            Ok(text) => parse(&path.display().to_string(), &text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Layout::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        // A favorite written by hand without arguments has none, which TOML cannot write back.
        for f in layout.favorites.iter_mut().filter(|f| f.args.is_null()) {
            f.args = serde_json::json!({});
        }
        Ok(Self {
            layout: layout.clone(),
            path: Some(path.to_path_buf()),
            saved: layout,
        })
    }

    /// A layout kept in memory only.
    pub fn in_memory(layout: Layout) -> Self {
        Self {
            saved: layout.clone(),
            layout,
            path: None,
        }
    }

    /// Writes the file if the layout changed since it was last written.
    pub fn save_if_changed(&mut self) -> Result<(), String> {
        if self.layout == self.saved {
            return Ok(());
        }
        self.write()
    }

    /// Writes the file as the layout is now.
    pub(crate) fn write(&mut self) -> Result<(), String> {
        if let Some(path) = &self.path {
            let text = toml::to_string_pretty(&self.layout).map_err(|e| e.to_string())?;
            write_file(
                path,
                &format!("# GigaSculpt layout, saved automatically.\n\n{text}"),
            )?;
        }
        self.saved = self.layout.clone();
        Ok(())
    }
}
