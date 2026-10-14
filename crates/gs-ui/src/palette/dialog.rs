//! The system's file dialogs (decision 0016, section 17, amended 2026-10-13): which dialog each
//! file action asks, offering its own files first and every file last, the folder it starts in
//! and the name it suggests; a name with no extension gets the action's; Open given a mesh file
//! imports it; and what a file given from outside the window runs (dropped on it, or named on
//! the command line). The window shows the dialog (`gs_platform::Dialogs`) and hands its answer
//! back as input; the palette turns it into the step, run and logged like any other.

use std::path::{Path, PathBuf};

use gs_platform::{Dialog, Filter, Pick};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::record::with_args;

/// The scene files' extension.
const SCENE: &str = "gss";
/// The mesh files Import reads and Reference from a file shows.
const MESHES_IN: &str = "obj;ply;stl;ztl;zpr";
/// The mesh files Export writes.
const MESHES_OUT: &str = "obj;ply;stl";

/// What a file action's dialog asks for: the argument it fills, the kind of dialog, and the
/// files it offers first, by name and extensions.
#[derive(Clone, Copy, Debug)]
pub(super) struct Wants {
    pub arg: &'static str,
    pub pick: Pick,
    files: (&'static str, &'static str),
}

/// The dialog `command` asks with, if it is a file action the window asks one for.
pub(super) fn wants(command: &str) -> Option<Wants> {
    let scenes = ("GigaSculpt scene", SCENE);
    let (arg, pick, files) = match command {
        "file.open" => ("path", Pick::Open, scenes),
        "file.save" | "file.save_next" => ("path", Pick::Save, scenes),
        "file.import" => ("path", Pick::Open, ("Mesh file", MESHES_IN)),
        "file.export" => ("path", Pick::Save, ("Mesh file", MESHES_OUT)),
        "brush.save" => ("path", Pick::Save, ("Brush preset", "toml")),
        "brush.load" => ("path", Pick::Open, ("Brush preset", "toml")),
        "reference.open" => ("file", Pick::Open, ("Mesh file", MESHES_IN)),
        "view.floor_save" => ("path", Pick::Save, ("Floor preset", "toml")),
        "material.folder" => ("folder", Pick::Folder, ("Folder", "")),
        _ => return None,
    };
    Some(Wants { arg, pick, files })
}

/// What the window last heard of the scene, for the name a Save dialog suggests.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// The scene's own file (`file.path`).
    pub own: Option<String>,
    /// The active part.
    pub active: Option<String>,
    /// The brush the pen strokes with.
    pub brush: String,
    /// The scene differs from its file (`file.unsaved`).
    pub unsaved: bool,
}

/// The dialog `step`'s file action asks with, starting in `folder`; a folder dialog offers no
/// files.
pub(super) fn dialog(step: &Step, wants: Wants, folder: Option<String>, scene: &Scene) -> Dialog {
    let (name, pattern) = wants.files;
    let filters = match wants.pick {
        Pick::Folder => Vec::new(),
        _ => vec![filter(name, pattern), filter("All files", "*")],
    };
    let folder = folder.map(PathBuf::from).or_else(std::env::home_dir);
    let name = (wants.pick == Pick::Save).then(|| suggested(step, scene));
    Dialog {
        pick: wants.pick,
        filters,
        folder,
        name,
    }
}

fn filter(name: &str, pattern: &str) -> Filter {
    Filter {
        name: name.into(),
        pattern: pattern.into(),
    }
}

/// The name a Save dialog suggests: for a Part file the active part's, else the scene's own
/// file's (its stem for Export), else Untitled; for a brush its name, for a floor preset Floor;
/// each with its extension.
fn suggested(step: &Step, scene: &Scene) -> String {
    let own = scene.own.as_deref().map(Path::new);
    let stem = own.and_then(Path::file_stem).and_then(|s| s.to_str());
    let parts = step.args.get("parts").is_some_and(|p| !p.is_null());
    let name = match step.command.as_str() {
        "brush.save" => Some(scene.brush.as_str()).filter(|b| !b.is_empty()),
        "view.floor_save" => Some("Floor"),
        _ if parts => scene.active.as_deref(),
        _ => stem,
    };
    extended(&step.command, name.unwrap_or("Untitled"))
}

/// `path` with the extension `command` writes added when its name has none: `.gss` for a save,
/// `.obj` for Export, `.toml` for a brush (review 0009, C6). Others, and a folder, as they are.
pub(super) fn extended(command: &str, path: &str) -> String {
    let writes = wants(command).filter(|w| w.pick == Pick::Save);
    let first = writes.and_then(|w| w.files.1.split(';').next());
    let named = Path::new(path);
    let folder = path.ends_with(['/', std::path::MAIN_SEPARATOR]) || named.is_dir();
    match first {
        Some(ext) if !folder && named.file_name().is_some() && named.extension().is_none() => {
            format!("{path}.{ext}")
        }
        _ => path.to_string(),
    }
}

/// `step` as it runs: Open given a mesh file imports it instead (review 0009, C7).
pub(super) fn routed(step: Step) -> Step {
    let path = step.args["path"].as_str();
    match path.filter(|p| step.command == "file.open" && is_mesh(Path::new(p))) {
        Some(path) => with_args("file.import", json!({ "path": path })),
        None => step,
    }
}

/// Whether `path` names a mesh file Import reads.
fn is_mesh(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    (MESHES_IN.split(';')).any(|m| m.eq_ignore_ascii_case(ext))
}

/// What a file given from outside the window runs, dropped on it or named on the command line:
/// a scene file opens, a mesh file is imported; anything else is refused, saying why.
pub fn opened(path: &Path) -> Result<Step, String> {
    let shown = path.display();
    if path.is_dir() {
        return Err(format!("\"{shown}\" is a folder: give a scene file"));
    }
    if !path.is_file() {
        return Err(format!("there is no file \"{shown}\""));
    }
    let whole = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let args = json!({ "path": whole });
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ext.eq_ignore_ascii_case(SCENE) {
        return Ok(with_args("file.open", args));
    }
    if is_mesh(path) {
        return Ok(with_args("file.import", args));
    }
    Err(format!(
        "\"{shown}\" is not a scene: GigaSculpt opens .gss scene files and imports OBJ, PLY, STL, \
         ZTL and ZPR files"
    ))
}

/// `step` with `value` for `arg`.
pub(super) fn given(step: &Step, arg: &str, value: Value) -> Step {
    let mut args = step.args.clone();
    if !args.is_object() {
        args = json!({});
    }
    if let Some(all) = args.as_object_mut() {
        all.insert(arg.into(), value);
    }
    with_args(&step.command, args)
}
