//! Send to Blender and Read from Blender (decision 0025, section 13; catalog
//! `interface.app-bridge`): the header's File > Blender group (decision 0060, section 3) runs
//! `exchange.write` and `exchange.read` in the session's exchange folders, found as the add-on
//! finds them. Send says why it cannot run in an empty scene; Read is held back, "Nothing new
//! from Blender", until a manifest lands in `from-blender`, and again once it has been read.

use gs_ui::Exchange;
use serde_json::{Value, json};

use crate::header::{item, open};
use crate::region::middle;
use crate::rig::{Ui, folder};

/// File > Blender's item `label`, after its heading.
fn button(ui: &mut Ui, label: &str) -> Value {
    open(ui, "File");
    let widgets = ui.widgets("menu");
    let heading = widgets
        .iter()
        .position(|w| w["label"] == "Blender" && w["role"] == "label");
    let heading = heading.unwrap_or_else(|| panic!("File heads a Blender group: {widgets:?}"));
    assert!(
        widgets[heading..].iter().any(|w| w["label"] == label),
        "{label} is under Blender: {widgets:?}"
    );
    item(ui, label)
}

/// Taps File > Blender's item `label` and runs what it asks for.
fn press(ui: &mut Ui, label: &str) {
    button(ui, label);
    let [x, y] = middle(ui.rect(&format!("menu > {label}")));
    ui.tap_at(x, y);
}

/// Copies the files of `from` into `to`, the manifest last as the add-on writes it.
fn copy(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap_or_else(|e| panic!("{e}"));
    let files = std::fs::read_dir(from).unwrap_or_else(|e| panic!("{e}"));
    let mut names: Vec<_> = files.filter_map(|f| Some(f.ok()?.file_name())).collect();
    names.sort_by_key(|n| n == "manifest.json");
    for name in names {
        std::fs::copy(from.join(&name), to.join(&name)).unwrap_or_else(|e| panic!("{e}"));
    }
}

#[test]
fn send_and_read_use_the_sessions_exchange_folders_and_read_waits_for_something_new() {
    let cache = folder("blender");
    let mut ui = Ui::new();
    ui.host.lock().shell.exchange = Exchange::at(cache.clone());
    let send = button(&mut ui, "Send");
    assert_eq!(send["enabled"], false, "{send}");
    assert_eq!(send["reason"], "the scene has no parts to send", "{send}");
    let read = button(&mut ui, "Read");
    assert_eq!(
        (&read["enabled"], &read["reason"]),
        (&json!(false), &json!("Nothing new from Blender"))
    );

    // A part to send: Send writes the session's to-blender folder, making it first.
    ui.ok("primitive.cube", json!({ "subdivisions": 2 }));
    assert_eq!(button(&mut ui, "Send")["enabled"], true);
    press(&mut ui, "Send");
    let session = ui.host.lock().shell.status.session.clone();
    let exchange = cache.join("gigasculpt/exchange").join(&session);
    let to = exchange.join("to-blender");
    assert!(to.join("manifest.json").is_file(), "{}", to.display());

    // Blender sends the cube back unchanged: Read can run, and reads it as one undo step.
    copy(&to, &exchange.join("from-blender"));
    let read = button(&mut ui, "Read");
    assert_eq!(read["enabled"], true, "{read}");
    press(&mut ui, "Read");
    let ran = ui.logged("exchange.read");
    assert_eq!(ran.len(), 1, "{ran:?}");
    let read = button(&mut ui, "Read");
    assert_eq!(
        read["reason"], "Nothing new from Blender",
        "what was read is not read twice: {read}"
    );
}
