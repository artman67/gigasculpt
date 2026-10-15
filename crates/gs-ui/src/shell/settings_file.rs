//! The settings file (decision 0050; catalog `interface.layout-presets`): the keymap's changes,
//! the quick menus and the arrangement in one TOML file. `ui.layout` `export` writes all three;
//! `import` reads back every part the file holds, or those named, each in place of the
//! window's, and leaves the others exactly as they were. Every part read is checked before any
//! is applied. The engine has read the file's version and the parts it holds already.

use std::collections::BTreeMap;
use std::path::Path;

use gs_engine::{Hand, LayoutPart, SETTINGS_VERSION, UiKeymap, errors, file_refused};
use gs_protocol::{CommandError, ErrorCode};
use serde::{Deserialize, Serialize};

use super::Shell;
use crate::direction::parse;
use crate::keymap::{Binding, Keymap, write_file};
use crate::layout::Layout;
use crate::quick;

/// What the file starts with.
const HEADER: &str = "# GigaSculpt settings: the keymap's changes, the quick menus and the \
                      arrangement\n# (`ui.layout` with `export`). `ui.layout` with `import` reads \
                      them back, all or the\n# parts named.\n";

/// The interface scales an arrangement may have, as `ui.layout` `scale` takes them.
const SCALES: [f32; 2] = [0.5, 3.0];

/// The file: its version, then each part.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    arrangement: Option<Arrangement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quick: Option<Quick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    keymap: Option<Keys>,
}

/// Where things are: the layout shown, for which hand, at what scale, with which sections
/// folded and trays hidden or widened.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrangement {
    layout: String,
    hand: Hand,
    scale: f32,
    folded: Vec<String>,
    hidden: Vec<String>,
    minimal: bool,
    one_open: bool,
    /// The trays' dragged widths, in points, by panel id.
    #[serde(default)]
    widths: BTreeMap<String, f32>,
}

/// Every quick menu as it shows, and the one shown, from 1.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Quick {
    shown: usize,
    menus: Vec<Vec<String>>,
}

/// The keymap's changes, as `keymap.toml` holds them.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Keys {
    #[serde(default)]
    bind: Vec<Binding>,
}

impl Shell {
    /// `ui.layout` `export`: every part, written to `path`.
    pub(super) fn export_settings(&self, path: &str) -> Result<(), String> {
        let l = &self.layout.layout;
        let menus = quick::menus(l, &self.quick_listed());
        let file = Settings {
            version: SETTINGS_VERSION,
            arrangement: Some(Arrangement {
                layout: self.shown().into(),
                hand: l.hand,
                scale: l.ui_scale,
                folded: l.folded.clone(),
                hidden: l.hidden.clone(),
                minimal: l.minimal,
                one_open: l.one_open,
                widths: l.widths.clone(),
            }),
            quick: Some(Quick {
                shown: quick::shown(l, menus.len()) + 1,
                menus,
            }),
            keymap: Some(Keys {
                bind: self.keymap.changes.clone(),
            }),
        };
        let text = toml::to_string_pretty(&file).map_err(|e| e.to_string())?;
        write_file(Path::new(path), &format!("{HEADER}\n{text}"))
    }

    /// `ui.layout` `import`: the parts of the file at `path` that `parts` names, or every part
    /// it holds, each in place of the window's. Nothing changes unless all of it reads. A
    /// refusal is coded by what went wrong: no file, a file it may not read, one it cannot read
    /// as settings (damaged), or one naming what the window does not have (not found).
    #[expect(
        clippy::result_large_err,
        reason = "the engine's wire error, as in keymap.rs"
    )]
    pub(super) fn import_settings(
        &mut self,
        path: &str,
        parts: &[LayoutPart],
    ) -> Result<(), CommandError> {
        // The engine read it to check it; one gone or changed since says so in its words.
        let read = std::fs::read_to_string(path);
        let text = read.map_err(|e| file_refused("read", Path::new(path), &e))?;
        let file: Settings = parse(path, &text).map_err(damaged)?;
        let named = |p: LayoutPart| parts.is_empty() || parts.contains(&p);
        let mut layout = self.layout.layout.clone();
        if let Some(a) = file.arrangement.filter(|_| named(LayoutPart::Arrangement)) {
            self.arrange(&mut layout, a)?;
        }
        if let Some(q) = file.quick.filter(|_| named(LayoutPart::Quick)) {
            self.quick_menus(&mut layout, q)?;
        }
        let keys = file.keymap.filter(|_| named(LayoutPart::Keymap));
        if let Some(mut keymap) = keys.map(|k| keymap(&k.bind)).transpose()? {
            // Rebuilt from its changes and written to `keymap.toml`.
            keymap.change(&UiKeymap::default(), self.keymap_file.as_deref())?;
            self.keymap = keymap;
        }
        self.layout.layout = layout;
        Ok(())
    }

    /// The layout file's quick menu list.
    fn quick_listed(&self) -> Vec<String> {
        (self.now_plain().direction).map_or_else(Vec::new, |d| d.quick.clone())
    }

    /// `a` put in `layout`, once its layout is one the window has and its scale one
    /// `ui.layout` takes.
    #[expect(
        clippy::result_large_err,
        reason = "the engine's wire error, as in keymap.rs"
    )]
    fn arrange(&self, layout: &mut Layout, a: Arrangement) -> Result<(), CommandError> {
        let names: Vec<&str> = (self.directions.list.iter())
            .map(|d| d.name.as_str())
            .collect();
        if !names.contains(&a.layout.as_str()) {
            let there = names.join(", ");
            return Err(not_there(format!(
                "no layout called {} here; there are {there}",
                a.layout
            )));
        }
        let [low, high] = SCALES;
        if !(low..=high).contains(&a.scale) {
            let message = format!("scale {} is not from {low} to {high}", a.scale);
            return Err(damaged(message));
        }
        (layout.current, layout.hand, layout.ui_scale) = (a.layout, a.hand, a.scale);
        (layout.folded, layout.hidden, layout.widths) = (a.folded, a.hidden, a.widths);
        (layout.minimal, layout.one_open) = (a.minimal, a.one_open);
        Ok(())
    }

    /// The quick menus `q` put in `layout`, once there are one to five, each holds something
    /// the window can show and the one shown is one of them.
    #[expect(
        clippy::result_large_err,
        reason = "the engine's wire error, as in keymap.rs"
    )]
    fn quick_menus(&self, layout: &mut Layout, q: Quick) -> Result<(), CommandError> {
        let count = q.menus.len();
        if !(1..=quick::MOST).contains(&count) {
            let message = format!("{count} quick menus; there are 1 to {}", quick::MOST);
            return Err(damaged(message));
        }
        if let Some(i) = q.menus.iter().position(Vec::is_empty) {
            return Err(damaged(format!("quick menu {} holds nothing", i + 1)));
        }
        if let Some(p) = (q.menus.iter().flatten()).find(|p| !self.directions.knows(p)) {
            let message = format!("nothing called `{p}` to show in a quick menu");
            return Err(not_there(message));
        }
        if !(1..=count).contains(&q.shown) {
            return Err(damaged(format!("quick menu {} shows, of {count}", q.shown)));
        }
        // Menus as the layout file lists them are no change.
        let listed = self.quick_listed();
        layout.quick = if q.menus == [listed] {
            Vec::new()
        } else {
            q.menus
        };
        layout.quick_menu = q.shown - 1;
        Ok(())
    }
}

/// The built-in keys with `changes` over them, each bound as `ui.keymap` binds; refused whole
/// when one names a command the program does not have.
#[expect(
    clippy::result_large_err,
    reason = "the engine's wire error, as in keymap.rs"
)]
fn keymap(changes: &[Binding]) -> Result<Keymap, CommandError> {
    let specs = gs_engine::registry();
    let known = |c: &str| c.is_empty() || specs.iter().any(|s| s.name == c);
    if let Some(b) = changes.iter().find(|b| !known(&b.command)) {
        return Err(not_there(format!(
            "{} runs `{}`, which is no command here; nothing was imported",
            b.key, b.command
        )));
    }
    let mut keymap = Keymap::default();
    for b in changes {
        let args = match serde_json::to_value(&b.args) {
            Ok(serde_json::Value::Object(args)) => args,
            _ => serde_json::Map::new(),
        };
        let bind = UiKeymap {
            bind: Some(b.key.clone()),
            command: Some(b.command.clone()),
            args,
            replace: true,
            ..UiKeymap::default()
        };
        let bound = keymap.change(&bind, None);
        bound.map_err(|e| damaged(format!("{}: {}", b.key, e.message)))?;
    }
    Ok(keymap)
}

/// A settings file the window cannot read as settings.
fn damaged(message: String) -> CommandError {
    errors::error(ErrorCode::Damaged, message)
}

/// A settings file naming what the window does not have.
fn not_there(message: String) -> CommandError {
    errors::error(ErrorCode::NotFound, message)
}
