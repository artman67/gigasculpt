//! Reading the layout and controls files: the shipped ones built in, the user's from the layouts
//! folder, each checked as it is added ([`Directions`]); and TOML read with an error that names
//! the file, line and key, and written back.

use std::path::Path;

use gs_engine::{BrushInfo, SessionState};
use serde::Serialize;
use serde_json::json;

use super::{Control, Controls, Direction, HeaderOptions, WIDGETS, split};

/// The shipped controls, a file per tray in the layouts folder's [`CONTROLS_DIR`], in the order
/// their sets come in: the shelf's, the brush tray's, the header's view buttons' (`strip.toml`,
/// once the view strip's), the scene tray's, the Reference View's bar, the settings window's and
/// the 3D view's header menus.
const CONTROLS: [(&str, &str); 7] = [
    (
        "controls/shelf.toml",
        include_str!("../../../../assets/layouts/controls/shelf.toml"),
    ),
    (
        "controls/brush-tray.toml",
        include_str!("../../../../assets/layouts/controls/brush-tray.toml"),
    ),
    (
        "controls/strip.toml",
        include_str!("../../../../assets/layouts/controls/strip.toml"),
    ),
    (
        "controls/scene-tray.toml",
        include_str!("../../../../assets/layouts/controls/scene-tray.toml"),
    ),
    (
        "controls/reference.toml",
        include_str!("../../../../assets/layouts/controls/reference.toml"),
    ),
    (
        "controls/settings.toml",
        include_str!("../../../../assets/layouts/controls/settings.toml"),
    ),
    (
        "controls/header.toml",
        include_str!("../../../../assets/layouts/controls/header.toml"),
    ),
];

/// The shipped layout.
const LAYOUTS: [(&str, &str); 1] = [(
    "workbench.toml",
    include_str!("../../../../assets/layouts/workbench.toml"),
)];

/// The folder of controls files, one per tray, in the layouts folder.
pub const CONTROLS_DIR: &str = "controls";

/// Whether `name` (without `.toml`) is a shipped layout's: a user file of that name replaces it.
pub fn is_shipped(name: &str) -> bool {
    LAYOUTS
        .iter()
        .any(|(file, _)| file.trim_end_matches(".toml") == name)
}

/// Every layout and the controls they share: the controls files' sets and the engine's
/// brushes. Empty when no files are loaded: then the window shows only the status line, the
/// parts and the 3D view.
#[derive(Clone, Debug, Default)]
pub struct Directions {
    /// Every controls file's sets, the files in [`CONTROLS`]'s order.
    pub controls: Controls,
    /// Each controls file's own sets, by its place in [`CONTROLS`].
    files: [Controls; CONTROLS.len()],
    /// Every brush, as `brush.list` gives them: by family, then by name.
    pub brushes: Vec<BrushInfo>,
    /// In `order`.
    pub list: Vec<Direction>,
}

impl Directions {
    /// No layouts yet, and the engine's brushes.
    fn empty() -> Self {
        Self {
            brushes: gs_engine::brushes(),
            ..Self::default()
        }
    }

    /// The shipped controls and layout.
    pub fn shipped() -> Result<Self, String> {
        let mut all = Self::empty();
        for (file, text) in CONTROLS.iter().chain(&LAYOUTS) {
            all.add(file, text)?;
        }
        Ok(all)
    }

    /// The shipped files, then every `.toml` file in `dir` (which may not exist) and in its
    /// `controls` folder. A file in `controls` named like a shipped tray's file replaces that
    /// tray's controls (`controls/brush-tray.toml` replaces the brush tray's sets, and the other
    /// trays keep theirs); any other name there is refused. In `dir`, a file named like a
    /// shipped layout replaces it, and any other adds a layout. Shipped files are never copied
    /// out, so a new version's layouts arrive unless the user has their own. A file that does
    /// not read is left out, and the one it would have replaced stays; the second value says
    /// why, naming the file, line and key.
    pub fn load(dir: &Path) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let controls = read_files(&dir.join(CONTROLS_DIR), "controls/", &mut problems);
        let layouts = read_files(dir, "", &mut problems);
        let shipped = |(file, text): (&str, &str)| (file.to_string(), text.to_string());
        // The controls first: the layouts are checked against them.
        let order = (CONTROLS.map(shipped).into_iter())
            .chain(controls)
            .chain(LAYOUTS.map(shipped))
            .chain(layouts);
        let mut all = Self::empty();
        for (file, text) in order {
            if let Err(e) = all.add(&file, &text) {
                problems.push(format!("{}: {e}", dir.display()));
            }
        }
        (all, problems)
    }

    /// Adds the file called `file` holding `text`: one tray's controls (`controls/<tray>.toml`),
    /// or one layout.
    pub fn add(&mut self, file: &str, text: &str) -> Result<(), String> {
        if file.starts_with("controls/") {
            return self.add_controls(file, text);
        }
        let mut direction: Direction = parse(file, text)?;
        direction.name = file.trim_end_matches(".toml").to_string();
        self.check(&direction).map_err(|e| format!("{file}: {e}"))?;
        self.list.retain(|d| d.name != direction.name);
        self.list.push(direction);
        self.list
            .sort_by(|a, b| a.order.cmp(&b.order).then(a.name.cmp(&b.name)));
        Ok(())
    }

    /// Puts `text` in place of the tray file `file`'s controls, if every set still reads and
    /// checks and no two sets share a name; else changes nothing.
    fn add_controls(&mut self, file: &str, text: &str) -> Result<(), String> {
        let Some(at) = CONTROLS.iter().position(|(f, _)| *f == file) else {
            let trays = CONTROLS.map(|(f, _)| f.trim_start_matches("controls/"));
            return Err(format!(
                "{file}: not a tray's file; a controls file is one of {}",
                trays.join(", ")
            ));
        };
        let mut files = self.files.clone();
        files[at] = parse(file, text)?;
        let sets = files.iter().flat_map(|c| c.sets.iter().cloned());
        let controls = Controls {
            sets: sets.collect(),
            families: files.iter().flat_map(|c| c.families.clone()).collect(),
        };
        // A layout names a set, whichever file holds it, so a name is used once.
        let mut seen = std::collections::BTreeSet::new();
        if let Some(set) = controls.sets.iter().find(|s| !seen.insert(&s.name)) {
            let name = &set.name;
            return Err(format!("{file}: set `{name}`: another set has this name"));
        }
        controls.check().map_err(|e| format!("{file}: {e}"))?;
        self.files = files;
        self.controls = controls;
        Ok(())
    }

    /// Each shipped tray's controls file and the sets it has now: what a user's file of that
    /// name in `layouts/controls/` replaces.
    pub fn control_files(&self) -> impl Iterator<Item = (&'static str, &Controls)> {
        CONTROLS.iter().map(|(file, _)| *file).zip(&self.files)
    }

    /// The layout called `name`.
    pub fn get(&self, name: &str) -> Option<&Direction> {
        self.list.iter().find(|d| d.name == name)
    }

    /// One control per brush, under its family, in the engine's order; each picks its brush
    /// and is lit while it is the brush. A Smooth brush goes on the Smooth slot instead, and is
    /// lit while it is there (`brush-settings.modifier-key-slots`).
    pub fn brush_controls(&self) -> Vec<Control> {
        (self.brushes.iter())
            .map(|b| {
                let (command, shows) = match b.family.as_str() {
                    "Smooth" => ("brush.slot", "brush.slot.smooth"),
                    _ => ("brush.set", "brush"),
                };
                let pick = Control {
                    group: Some(b.family.clone()),
                    ..Control::runs(&b.name, command, &json!({ "brush": b.name }))
                };
                pick.showing(shows, &b.name)
            })
            .collect()
    }

    /// Whether `piece` names something to show: a set or a group of one with controls, a
    /// family some brush has, or a shared widget.
    pub(crate) fn knows(&self, piece: &str) -> bool {
        let (name, group) = split(piece);
        if name == "brushes" {
            let brushes = self.brush_controls();
            return brushes
                .iter()
                .any(|c| group.is_none_or(|g| c.group.as_deref() == Some(g)));
        }
        let widget = group.is_none() && WIDGETS.iter().any(|w| w.0 == name);
        widget || (self.controls.piece(piece)).is_some_and(|(_, c)| !c.is_empty())
    }

    /// Every piece names a set, a group of one, a family of brushes or a shared widget; panel
    /// and Library tab ids are unique; the wheel rings a piece; the header and its options
    /// buttons, the settings window, the quick menu, the Library's tabs and each tool show
    /// pieces; an options button and a tool name state values.
    fn check(&self, d: &Direction) -> Result<(), String> {
        let unknown = |key: &str, piece: &str| {
            format!(
                "{key}: nothing called `{piece}`; give a set from {}/ ({}), `set/Group`, \
                 `brushes/Family`, or one of {}",
                CONTROLS_DIR,
                self.set_names(),
                WIDGETS.map(|w| w.0).join(", ")
            )
        };
        for (i, panel) in d.panels.iter().enumerate() {
            if d.panels[..i].iter().any(|p| p.id == panel.id) {
                return Err(format!("panel `{}`: id: used twice", panel.id));
            }
            if let Some(piece) = panel.show.iter().find(|p| !self.knows(p)) {
                return Err(format!("panel `{}`: {}", panel.id, unknown("show", piece)));
            }
        }
        // The pen end's entries are pieces but for its options buttons' ids.
        let mut header = d.header.iter().flat_map(|h| {
            let menus = h.menus.iter().map(|p| ("header: menus", p));
            let piece = move |p: &&String| !h.options.iter().any(|o| o.id == **p);
            let show = h.show.iter().filter(piece).map(|p| ("header: show", p));
            let options = h.options.iter().flat_map(|o| o.show.iter());
            menus.chain(show.chain(options.map(|p| ("header: options", p))))
        });
        if let Some((key, piece)) = header.find(|(_, p)| !self.knows(p)) {
            return Err(unknown(key, piece));
        }
        let names = SessionState::names();
        let unknown_light =
            |o: &&HeaderOptions| o.shows.as_ref().is_some_and(|v| !names.contains(v));
        let mut lights = d.header.iter().flat_map(|h| &h.options);
        if let Some(o) = lights.find(unknown_light) {
            return Err(format!(
                "header: options `{}`: shows: no state value is called that",
                o.id
            ));
        }
        if let Some(piece) = d.settings.iter().find(|p| !self.knows(p)) {
            return Err(unknown("settings", piece));
        }
        if let Some(piece) = d.quick.iter().find(|p| !self.knows(p)) {
            return Err(unknown("quick", piece));
        }
        for (i, tab) in d.library.iter().enumerate() {
            if d.library[..i].iter().any(|t| t.id == tab.id) {
                return Err(format!("library `{}`: id: used twice", tab.id));
            }
            if !self.knows(&tab.show) {
                return Err(format!(
                    "library `{}`: {}",
                    tab.id,
                    unknown("show", &tab.show)
                ));
            }
        }
        for tool in &d.tools {
            if tool.when.as_ref().is_some_and(|v| !names.contains(v)) {
                let id = &tool.id;
                return Err(format!("tool `{id}`: when: no state value is called that"));
            }
            // `brush` shows the active tool, so a tool showing it would draw itself.
            let wrong = |p: &&String| p.as_str() == "brush" || !self.knows(p);
            if let Some(piece) = tool.show.iter().find(wrong) {
                return Err(format!("tool `{}`: {}", tool.id, unknown("show", piece)));
            }
            if let Some(piece) = tool.tray.iter().find(|p| !self.knows(p)) {
                return Err(format!("tool `{}`: {}", tool.id, unknown("tray", piece)));
            }
        }
        match &d.wheel {
            Some(wheel) if !self.knows(&wheel.show) => {
                Err(format!("wheel: {}", unknown("show", &wheel.show)))
            }
            _ => Ok(()),
        }
    }

    fn set_names(&self) -> String {
        let names: Vec<&str> = self.controls.sets.iter().map(|s| s.name.as_str()).collect();
        names.join(", ")
    }
}

/// A layout or the controls as the file text that reads back the same.
pub fn to_toml<T: Serialize>(value: &T) -> Result<String, String> {
    toml::to_string_pretty(value).map_err(|e| e.to_string())
}

/// Every `.toml` file in `dir` (which may not exist), by name, as its name after `prefix` and
/// its text. One that does not read goes in `problems`.
fn read_files(dir: &Path, prefix: &str, problems: &mut Vec<String>) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    for path in entries.map(|e| e.path()) {
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => files.push((format!("{prefix}{}", file_name(&path)), text)),
            Err(e) => problems.push(format!("{}: {e}", path.display())),
        }
    }
    files.sort();
    files
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Reads `text`, from the file called `file`. An error names the file, the line and the key.
pub fn parse<T: serde::de::DeserializeOwned>(file: &str, text: &str) -> Result<T, String> {
    toml::from_str(text).map_err(|e| toml_error(file, text, &e))
}

/// A TOML error as one line: the file, the line number, the line itself (which names the key)
/// and what is wrong.
pub fn toml_error(file: &str, text: &str, e: &toml::de::Error) -> String {
    let Some(at) = e.span().map(|s| s.start) else {
        return format!("{file}: {}", e.message());
    };
    let before = &text[..at.min(text.len())];
    let number = before.matches('\n').count() + 1;
    let line = text.lines().nth(number - 1).unwrap_or_default().trim();
    format!("{file}: line {number} `{line}`: {}", e.message())
}
