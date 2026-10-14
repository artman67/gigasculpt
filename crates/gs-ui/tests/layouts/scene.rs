//! The scene's files (decision 0016, section 17), reached from the header's File menu
//! (decision 0060, section 3): File lists every save with its key; the window asks for what a
//! scene command lacks instead of failing: Open with unsaved changes asks before it drops them,
//! a first Save asks a name and the next Save writes the same file without asking; a form's
//! path starts in the folder that fits it; a relaunch can open the last scene; and Open recent
//! lists the recent scenes by name.

use gs_platform::{Input, Modifiers};
use serde_json::{Value, json};

use crate::header::{item, items, open, shown_menu};
use crate::rig::{Ui, folder, key};

/// An open form: its command, the arguments it has, and each field it asks with the text it
/// starts with.
type Asked = (String, Value, Vec<(String, String)>);

/// The palette's form, if one is open.
fn form(ui: &Ui) -> Option<Asked> {
    let studio = ui.host.lock();
    let form = studio.shell.palette.form.as_ref()?;
    let step = &form.step;
    Some((step.command.clone(), step.args.clone(), form.fields.clone()))
}

/// Presses File's item `label` in the header, opening File first.
fn press(ui: &mut Ui, label: &str) {
    open(ui, "File");
    ui.press(&format!("menu > {label}"));
}

/// The save actions File lists, in order: each item's label, what it runs and with which
/// arguments, and its key. Ctrl+Shift+T saves the active Part; the Part's Save next has no key,
/// as Ctrl+Alt+S is the scene's.
fn saves() -> [(&'static str, &'static str, Value, Value); 7] {
    let part = json!({ "parts": ["@active"] });
    let save_as = json!({ "form": "file.save", "ask": ["path"] });
    let save_part = json!({ "form": "file.save", "args": part.clone(), "ask": ["path"] });
    [
        ("Save", "file.save", json!({}), json!("Ctrl+S")),
        ("Save as…", "ui.palette", save_as, Value::Null),
        (
            "Save next",
            "file.save_next",
            json!({}),
            json!("Ctrl+Alt+S"),
        ),
        ("Quick save", "file.quick_save", json!({}), json!("9")),
        (
            "Revert…",
            "ui.palette",
            json!({ "form": "file.revert" }),
            json!("Ctrl+Alt+R"),
        ),
        ("Save…", "ui.palette", save_part, json!("Ctrl+Shift+T")),
        ("Save next", "file.save_next", part, Value::Null),
    ]
}

/// Catalog `interface.scene-files`: "One Save button runs a configurable default action
/// (Save ...); press and hold, or right-click, lists all save actions", which decision 0060
/// (section 3) moved into the header: File lists every save action with its key, Save first
/// running `file.save`; a pick runs it and shuts the menu.
#[test]
fn file_lists_every_save_with_its_key_and_a_pick_runs_it() {
    let mut ui = Ui::new();
    let dir = folder("save-button");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": dir.join("head_001.gss") }));
    open(&mut ui, "File");
    let rows = items(&mut ui);
    let at = rows.iter().position(|w| w["label"] == "Save");
    let at = at.unwrap_or_else(|| panic!("File lists Save: {rows:?}"));
    let listed: Vec<_> = (rows[at..].iter().take(7))
        .map(|w| {
            let step = &w["command"];
            let args = match &step["args"] {
                Value::Null => json!({}),
                args => args.clone(),
            };
            (
                w["label"].clone(),
                step["command"].clone(),
                args,
                w["key"].clone(),
            )
        })
        .collect();
    let saves: Vec<_> = (saves().into_iter())
        .map(|(label, command, args, key)| (json!(label), json!(command), args, key))
        .collect();
    assert_eq!(listed, saves, "every save, with its key");
    ui.press("menu > Save next");
    assert_eq!(shown_menu(&mut ui), None, "a pick shuts the menu");
    assert!(
        dir.join("head_002.gss").exists(),
        "Save next ran: {:?}",
        files(&dir)
    );
    // Save writes the scene's own file, now head_002, which a backup keeps.
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    press(&mut ui, "Save");
    assert!(
        dir.join("head_002.backup.gss").exists(),
        "{:?}",
        files(&dir)
    );
}

/// Types `text` after what the form's first field holds and presses Enter, as a person does.
pub(crate) fn answer(ui: &mut Ui, text: &str) {
    let none = Modifiers::default();
    ui.input(&[], none);
    ui.input(&[Input::Text(text.into())], none);
    ui.input(&[key("Enter", true, none)], none);
    ui.input(&[key("Enter", false, none)], none);
}

/// How many parts the scene has.
pub(crate) fn parts(ui: &mut Ui) -> usize {
    let stats = ui.ok("mesh.stats", json!({}));
    stats["parts"].as_array().into_iter().flatten().count()
}

/// The names of the files in `dir`, sorted.
fn files(dir: &std::path::Path) -> Vec<String> {
    let listed = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}"));
    let mut names: Vec<String> = (listed.flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Catalog `interface.scene-files`: "the first Save on a new scene shows a name dialog and the
/// second writes the same file with none".
#[test]
fn the_first_save_asks_a_name_and_the_next_writes_the_same_file_without_asking() {
    let mut ui = Ui::new();
    let dir = folder("first-save");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    press(&mut ui, "Save");
    let asked = form(&ui).expect("a first Save opens the form");
    let fields = [("path".to_string(), String::new())];
    assert_eq!((asked.0.as_str(), &asked.2[..]), ("file.save", &fields[..]));
    assert!(
        files(&dir).is_empty(),
        "nothing is written before a name is given"
    );
    let path = dir.join("first.gss");
    answer(&mut ui, &path.display().to_string());
    assert!(form(&ui).is_none(), "the form closes once it runs");
    assert_eq!(files(&dir), ["first.gss"]);
    assert_eq!(ui.state()["file.path"], json!(path.display().to_string()));
    let first = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    press(&mut ui, "Save");
    assert!(form(&ui).is_none(), "the second Save asks nothing");
    assert_eq!(
        files(&dir),
        ["first.backup.gss", "first.gss"],
        "the same file, with its backup"
    );
    let second = std::fs::read(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(
        first, second,
        "the second Save wrote the cube into the same file"
    );
}

/// Catalog `interface.scene-files`: "Open asks before discarding unsaved changes", with the
/// form New's button opens; the scene stays as it was until the question is answered.
#[test]
fn open_with_unsaved_changes_asks_before_it_drops_them() {
    let mut ui = Ui::new();
    let dir = folder("open-asks");
    let path = dir.join("one.gss");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": path }));
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    press(&mut ui, "Open…");
    let (command, _, fields) = form(&ui).expect("Open asks for the file");
    assert_eq!(command, "file.open");
    let start = format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    assert_eq!(
        fields,
        [("path".to_string(), start)],
        "in the scene's folder"
    );
    answer(&mut ui, "one.gss");
    let asked = form(&ui).expect("unsaved changes: Open asks before it drops them");
    let path = path.display().to_string();
    assert_eq!(asked.0, "file.open");
    assert_eq!(
        asked.1,
        json!({ "path": path }),
        "the form keeps the file asked for"
    );
    assert_eq!(asked.2, [("discard_changes".to_string(), String::new())]);
    assert_eq!(parts(&mut ui), 2, "nothing is dropped while it asks");
    let problem = ui
        .host
        .lock()
        .shell
        .status
        .problem
        .clone()
        .unwrap_or_default();
    assert!(
        problem.contains("unsaved"),
        "the status line says why: {problem}"
    );
    ui.press("palette > Discard changes and open");
    assert!(form(&ui).is_none());
    assert_eq!(parts(&mut ui), 1, "opened once answered");
    // With no changes, Open asks only for the file.
    press(&mut ui, "Open…");
    answer(&mut ui, "one.gss");
    assert!(form(&ui).is_none(), "nothing to drop: it opens at once");
}

/// Catalog `interface.scene-files`: "after loading from folder A and importing from folder B,
/// Save As opens in A"; Import opens in B, the folder it ran with last.
#[test]
fn save_as_starts_where_the_scene_was_loaded_whatever_was_imported() {
    let mut ui = Ui::new();
    let (a, b) = (folder("folder-a"), folder("folder-b"));
    let scene = a.join("scene.gss");
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": scene }));
    ui.ok("file.new", json!({ "discard_changes": true }));
    ui.ok("file.open", json!({ "path": scene }));
    let obj = b.join("tri.obj");
    let tetra = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 0 1\nf 1 3 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n";
    std::fs::write(&obj, tetra).unwrap_or_else(|e| panic!("{e}"));
    let in_folder =
        |dir: &std::path::Path| format!("{}{}", dir.display(), std::path::MAIN_SEPARATOR);
    let start = |ui: &Ui| form(ui).map(|f| f.2[0].1.clone());
    press(&mut ui, "Import…");
    assert_eq!(
        start(&ui),
        Some(in_folder(&a)),
        "Import starts in the scene's folder at first"
    );
    // From A's folder, the way to B's: up one, then B.
    let b_name = b.file_name().map(|n| n.to_string_lossy().into_owned());
    let to_b = std::path::Path::new("..").join(b_name.unwrap_or_default());
    answer(&mut ui, &to_b.join("tri.obj").display().to_string());
    assert_eq!(parts(&mut ui), 2, "imported from B");
    press(&mut ui, "Save as…");
    assert_eq!(start(&ui), Some(in_folder(&a)), "Save As opens in A");
    ui.ok("ui.palette", json!({ "open": false }));
    press(&mut ui, "Import…");
    assert_eq!(
        start(&ui),
        Some(in_folder(&a.join(to_b))),
        "Import opens in B, where it ran last"
    );
    ui.ok("ui.palette", json!({ "open": false }));
}

/// Catalog `interface.scene-files`: "with open-last on, relaunching reopens the scene". Starts
/// of the window on one config folder and state folder, each stopped cleanly: off (the default) a
/// relaunch opens nothing; switched on in the settings window's Files set, it opens the scene
/// saved last, logged as any open; a gone file opens nothing and says nothing. Open recent's
/// first row is the old Open last (decision 0060, section 3), and asks before it drops unsaved
/// changes, as Open does.
#[test]
fn with_open_last_on_a_relaunch_reopens_the_last_scene() {
    let dir = folder("open-last");
    let state = dir.join("state");
    let launch = || {
        let mut ui = Ui::launch(&dir, &state);
        ui.settle();
        ui
    };
    let stop = |mut ui: Ui| ui.session.engine.close();
    let scene = dir.join("head.gss").display().to_string();
    let mut ui = launch();
    ui.ok("primitive.sphere", json!({ "subdivisions": 8 }));
    ui.ok("file.save", json!({ "path": scene }));
    stop(ui);
    let mut ui = launch();
    assert_eq!(parts(&mut ui), 0, "off by default: nothing opens");
    ui.ok("ui.settings", json!({ "open": true, "page": "Files" }));
    ui.settle();
    let switch = "Open last scene";
    ui.scroll_to("settings", switch);
    ui.press(&format!("settings > {switch}"));
    assert_eq!(ui.state()["settings.open_last"], json!(true), "switched on");
    stop(ui);
    let mut ui = launch();
    assert_eq!(
        ui.state()["file.path"],
        json!(scene),
        "the relaunch reopened it"
    );
    assert_eq!(parts(&mut ui), 1, "the saved sphere");
    assert_eq!(ui.logged("file.open").len(), 1, "logged as any open");
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    open_recent(&mut ui);
    let first = recent_rows(&mut ui).first().cloned();
    assert_eq!(
        first,
        Some(("head".into(), json!(1), true)),
        "the last scene first"
    );
    ui.press("submenu > head");
    let asked = form(&ui).expect("unsaved changes: Open last asks");
    let open_last = ("file.open", json!({ "recent": 1 }));
    assert_eq!(
        (asked.0.as_str(), asked.1),
        open_last,
        "the step it asks for"
    );
    let field = [("discard_changes".to_string(), String::new())];
    assert_eq!(asked.2, field, "whether to drop the changes");
    ui.ok("ui.palette", json!({ "open": false }));
    stop(ui);
    std::fs::remove_file(&scene).unwrap_or_else(|e| panic!("{e}"));
    let mut ui = launch();
    assert_eq!(parts(&mut ui), 0, "a gone file opens nothing");
    let problem = ui.host.lock().shell.status.problem.clone();
    assert_eq!(problem, None, "and says nothing");
}

/// Opens File > Open recent's submenu with a tap on it.
fn open_recent(ui: &mut Ui) {
    open(ui, "File");
    assert_eq!(
        item(ui, "Open recent")["command"],
        Value::Null,
        "it only lists"
    );
    ui.press("menu > Open recent");
}

/// The rows File > Open recent's submenu shows, in order: each scene's name, the place in the
/// list its `file.open` names, and whether it can be picked. A row says its file's path on
/// hover.
fn recent_rows(ui: &mut Ui) -> Vec<(String, Value, bool)> {
    let shown = ui.widgets("submenu");
    let scene = |h: &str| {
        std::path::Path::new(h)
            .extension()
            .is_some_and(|e| e == "gss")
    };
    let row = |w: &&Value| w["hint"].as_str().is_some_and(scene);
    (shown.iter().filter(row))
        .map(|w| {
            let name = w["label"].as_str().unwrap_or_default().to_string();
            let open = (&w["command"]["command"], &w["command"]["args"]);
            assert_eq!(open.0, "file.open", "a row opens its scene: {w}");
            (name, open.1["recent"].clone(), w["enabled"] == true)
        })
        .collect()
}

/// Catalog `interface.scene-files`' recent list (decision 0016, section 17, item 7, and decision
/// 0060, section 3): a tap on File > Open recent lists the recent scenes by name, newest first;
/// a gone file shows but cannot be picked; Escape shuts the list and runs nothing; a pick opens
/// its scene through one `file.open`, asking first while there are unsaved changes, as Open
/// last does.
#[test]
fn recent_lists_the_scenes_by_name_newest_first_and_a_pick_opens_one() {
    let dir = folder("recent-menu");
    let mut ui = Ui::launch(&dir, &dir.join("state"));
    ui.settle();
    for name in ["head_study", "torso_block", "hand_pose"] {
        ui.ok("file.new", json!({ "discard_changes": true }));
        ui.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        let path = dir.join(format!("{name}.gss"));
        ui.ok("file.save", json!({ "path": path }));
    }
    std::fs::remove_file(dir.join("torso_block.gss")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(recent_rows(&mut ui), [], "the list starts shut");
    let opens = |ui: &mut Ui| ui.logged("file.open").len();
    let before = opens(&mut ui);
    open_recent(&mut ui);
    let row = |name: &str, place: u64, can: bool| (name.to_string(), json!(place), can);
    let listed = [
        row("hand_pose", 1, true),
        row("torso_block", 2, false),
        row("head_study", 3, true),
    ];
    assert_eq!(
        recent_rows(&mut ui),
        listed,
        "newest first, the gone one dimmed"
    );
    ui.press("submenu > torso_block");
    assert_eq!(opens(&mut ui), before, "a gone file cannot be picked");
    ui.tap("Escape", Modifiers::default());
    assert_eq!(recent_rows(&mut ui), [], "Escape shuts the list");
    assert_eq!(shown_menu(&mut ui), None, "and File");
    assert_eq!(opens(&mut ui), before, "and runs nothing");
    ui.ok("primitive.cube", json!({ "subdivisions": 4 }));
    open_recent(&mut ui);
    ui.press("submenu > head_study");
    assert_eq!(recent_rows(&mut ui), [], "a pick shuts the list");
    assert_eq!(shown_menu(&mut ui), None, "and File");
    let asked = form(&ui).expect("unsaved changes: the pick asks first");
    let step = ("file.open", json!({ "recent": 3 }));
    assert_eq!((asked.0.as_str(), asked.1), step, "the pick's one step");
    ui.press("palette > Discard changes and open");
    assert_eq!(opens(&mut ui), before + 1, "opened through one file.open");
    let head = dir.join("head_study.gss").display().to_string();
    assert_eq!(ui.state()["file.path"], json!(head));
    assert_eq!(parts(&mut ui), 1, "the saved sphere");
}
