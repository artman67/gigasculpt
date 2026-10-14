//! The Alpha palette in Brush tray > Alpha (catalog `alpha.palette`, decision 0060 section 6):
//! hovering any alpha's picture says its name, size and bit depth, then its hint; the alphas used
//! lately are the engine's list, which grows with each new alpha, lets one picked right after a
//! slot take that slot, and `alpha.pick` `restore` cuts back to four; the large picture opens
//! the Library's Alphas; `alpha.pick` picks any alpha by its number. Under the picture and the
//! recent alphas come +, None and Alpha's ⋯, then Turn and Follow.

use gs_protocol::ErrorCode;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Ui, center};

/// The Workbench, where Brush tray > Alpha shows, the tray scrolled to it (it ends past the
/// window's foot).
fn workbench() -> Ui {
    let mut ui = Ui::new();
    ui.ok("ui.layout", json!({ "name": "workbench" }));
    ui.scroll_to("brush", "Follow");
    ui
}

/// Decision 0060, section 11's hints for the large picture and a recent one.
const PICTURE: &str = "The image the tip stamps. Tap for every alpha in the Library.";
const RECENT: &str = "Tap to use it again.";
/// Alpha's ⋯'s hint, which tells it from the brush's ⋯ in the tray's header.
const MORE: &str = "Hold turn and Map detail.";

/// Every alpha `alpha.list` lists, in order.
fn listed(ui: &mut Ui) -> Vec<Value> {
    let list = ui.ok("alpha.list", json!({}));
    list["alphas"].as_array().cloned().unwrap_or_default()
}

/// What hovering `alpha` (as `alpha.list` lists it) should say: its title, size and bits.
fn says(alpha: &Value) -> String {
    let (w, h, bits) = (&alpha["width"], &alpha["height"], &alpha["bits"]);
    format!(
        "{}, {w} x {h}, {bits}-bit",
        alpha["title"].as_str().unwrap_or_default()
    )
}

/// Panel `panel`'s widgets running `command`, with `arg` among their arguments.
fn running(ui: &mut Ui, panel: &str, command: &str, arg: &str) -> Vec<Value> {
    (ui.widgets(panel).into_iter())
        .filter(|w| w["command"]["command"] == command && w["command"]["args"].get(arg).is_some())
        .collect()
}

/// A tap of the pen on `widget`.
fn tap(ui: &mut Ui, widget: &Value) {
    let [x, y] = center(&widget["rect"]);
    ui.tap_at(x, y);
}

/// The alphas used lately, by name, as `session.state` says.
fn recent(ui: &mut Ui) -> Vec<String> {
    serde_json::from_value(ui.state()["alpha.recent"].clone()).unwrap_or_default()
}

#[test]
fn hovering_any_alpha_says_its_name_size_and_16_bits() {
    let mut ui = workbench();
    let alphas = listed(&mut ui);
    assert!(!alphas.is_empty(), "alpha.list lists the shipped alphas");
    let of = |name: &str| (alphas.iter()).find(|a| a["name"] == name).map(says);
    // The palette's large picture is the brush's alpha: none at first, which stamps as white.
    let large = running(&mut ui, "brush", "ui.library", "tab");
    assert_eq!(large.len(), 1, "one large picture");
    let white = of("white").map(|s| format!("{s}. {PICTURE}"));
    assert_eq!(large[0]["hint"].as_str(), white.as_deref());
    // Each recent picture, and each tile of the Library's Alphas.
    for w in running(&mut ui, "brush", "alpha.pick", "slot") {
        let hint = w["hint"].as_str().unwrap_or_default();
        let said = hint.contains(", 16-bit. ") && hint.ends_with(RECENT);
        assert!(said, "{} says {hint:?}", w["label"]);
    }
    ui.ok("ui.library", json!({ "tab": "alphas" }));
    let tiles = running(&mut ui, "library", "brush.set", "alpha");
    assert_eq!(tiles.len(), alphas.len(), "a tile per alpha");
    for a in &alphas {
        let tile = (tiles.iter()).find(|t| t["command"]["args"]["alpha"] == a["name"]);
        let hint = tile.and_then(|t| t["hint"].as_str());
        assert_eq!(hint, Some(says(a).as_str()), "{}", a["name"]);
        assert!(says(a).ends_with(", 16-bit"));
    }
}

#[test]
fn the_recent_alphas_grow_a_slot_pick_lends_its_slot_and_restore_cuts_them_back() {
    let mut ui = workbench();
    let names: Vec<String> = (listed(&mut ui).iter())
        .filter_map(|a| a["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.len() >= 6, "six shipped alphas: {names:?}");
    let first: Vec<String> = names[..4].to_vec();
    assert_eq!(recent(&mut ui), first, "the first four shipped, at first");
    let pictures = running(&mut ui, "brush", "alpha.pick", "slot");
    assert_eq!(pictures.len(), 4, "a picture per slot");
    // An alpha picked in the Library joins the front.
    ui.ok("ui.library", json!({ "tab": "alphas" }));
    let tiles = running(&mut ui, "library", "brush.set", "alpha");
    let fifth = (tiles.iter()).find(|t| t["command"]["args"]["alpha"] == names[4].as_str());
    tap(
        &mut ui,
        fifth.unwrap_or_else(|| panic!("a tile for {}", names[4])),
    );
    assert_eq!(ui.state()["brush.alpha"], json!(names[4]));
    let grown = [&names[4..5], &first[..]].concat();
    assert_eq!(recent(&mut ui), grown, "the fifth at the front");
    // A tap on slot 2 picks its alpha; the next new alpha takes that slot.
    let slot = running(&mut ui, "brush", "alpha.pick", "slot");
    tap(&mut ui, &slot[2]);
    assert_eq!(
        ui.state()["brush.alpha"],
        json!(names[1]),
        "slot 2 holds the second"
    );
    assert_eq!(recent(&mut ui), grown, "a slot pick moves nothing");
    let lit: Vec<Value> = (running(&mut ui, "brush", "alpha.pick", "slot").into_iter())
        .map(|w| w["selected"].clone())
        .collect();
    assert_eq!(
        lit[2],
        json!(true),
        "the slot of the brush's alpha is lit: {lit:?}"
    );
    ui.ok("brush.set", json!({ "alpha": names[5] }));
    let swapped = [&names[4..5], &names[0..1], &names[5..6], &names[2..4]].concat();
    assert_eq!(recent(&mut ui), swapped, "the sixth took slot 2");
    let list = ui.ok("alpha.list", json!({}))["recent"].clone();
    assert_eq!(list, json!(swapped), "alpha.list says the same");
    // The large picture opens the Library on its Alphas.
    ui.ok("ui.library", json!({ "open": false }));
    let large = running(&mut ui, "brush", "ui.library", "tab");
    tap(&mut ui, &large[0]);
    let library = ui.ok("ui.library", json!({}));
    assert_eq!(
        (library["open"].clone(), library["tab"].clone()),
        (json!(true), json!("alphas"))
    );
    // `restore`, which left the tray for the palette, cuts the list back to four slots.
    ui.ok("alpha.pick", json!({ "restore": true }));
    assert_eq!(recent(&mut ui), swapped[..4].to_vec());
    assert_eq!(running(&mut ui, "brush", "alpha.pick", "slot").len(), 4);
    let refused = ui.err("alpha.pick", json!({ "slot": 4 }));
    assert_eq!(refused.0, ErrorCode::NotFound, "{}", refused.1);
    let both = ui.err("alpha.pick", json!({ "slot": 0, "index": 0 }));
    assert_eq!(both.0, ErrorCode::ArgsInvalid, "{}", both.1);
}

/// Alpha's ⋯, told from the brush's ⋯ in the tray's header by its hint.
fn more(ui: &mut Ui) -> Value {
    let mut all = ui.widgets("brush").into_iter();
    let more = all.rfind(|w| w["label"] == "···" && w["hint"] == MORE);
    more.unwrap_or_else(|| panic!("Alpha's ⋯ shows"))
}

/// Whether the brush tray shows a widget labelled `label`.
fn shows(ui: &mut Ui, label: &str) -> bool {
    ui.labels("brush").iter().any(|l| l == label)
}

/// `alpha.pick` picks any alpha by its number, which the index slider did in the tray.
#[test]
fn alpha_pick_picks_any_alpha_by_its_number() {
    let mut ui = workbench();
    let names: Vec<Value> = (listed(&mut ui).iter())
        .map(|a| a["name"].clone())
        .collect();
    for (i, name) in names.iter().enumerate() {
        ui.ok("alpha.pick", json!({ "index": i }));
        assert_eq!(&ui.state()["brush.alpha"], name, "number {i}");
    }
    let (code, _) = ui.err("alpha.pick", json!({ "index": names.len() }));
    assert_eq!(code, ErrorCode::NotFound, "past the last");
}

/// Decision 0060, section 6, item 5: under the picker row (the alpha's picture and the recent
/// ones) come + (waiting for user alphas), None and Alpha's ⋯ on a row, and Turn and Follow
/// under that; the index slider, Restore Layout and the old names are gone. None leaves a plain
/// round tip, and the ⋯ holds Hold turn and Map detail.
#[test]
fn the_alpha_row_holds_plus_none_and_its_menu_then_turn_and_follow() {
    let mut ui = workbench();
    let gone = [
        "Alpha index",
        "Restore Layout",
        "Tilt Brush",
        "Align To Path",
    ];
    for label in gone {
        assert!(!shows(&mut ui, label), "{label} leaves the tray");
    }
    let widgets = ui.widgets("brush");
    let plus = (widgets.iter()).find(|w| w["label"] == "+");
    let plus = plus.unwrap_or_else(|| panic!("+ shows: {widgets:?}"));
    assert_eq!(plus["enabled"], false, "+ waits for user alphas");
    // It draws no "later" beside it: what it waits for is in its hint on hover (decision 0025,
    // section 7, amended for UI-finish), so it is narrower than None.
    let labels = ui.labels("brush");
    let later: Vec<&String> = labels.iter().filter(|l| l.contains("later")).collect();
    assert!(later.is_empty(), "{later:?}");
    let waits = "Loads an image file as an alpha. Waits for: user alphas, phase B4";
    assert_eq!(plus["hint"], waits);
    let width = |r: &Value| r[2].as_f64().unwrap_or_default() - r[0].as_f64().unwrap_or_default();
    let none = ui.rect("brush > None");
    assert!(
        width(&plus["rect"]) < none[2] - none[0],
        "{plus}, None {none:?}"
    );
    let slot = running(&mut ui, "brush", "alpha.pick", "slot");
    let large = running(&mut ui, "brush", "ui.library", "tab");
    let picker = center(&large[0]["rect"])[1];
    for s in &slot {
        let y = center(&s["rect"])[1];
        assert!(
            (y - picker).abs() < 10.0,
            "the recent alphas beside the picture"
        );
    }
    let none = ui.rect("brush > None");
    let row = middle(none)[1];
    assert!(
        none[1] > slot[0]["rect"][3].as_f64().unwrap_or_default(),
        "under the picker"
    );
    let ys = [
        ("+", center(&plus["rect"])[1]),
        ("···", center(&more(&mut ui)["rect"])[1]),
    ];
    for (name, y) in ys {
        assert!((y - row).abs() < 10.0, "{name} on None's row");
    }
    let turn = ui.rect("brush > Turn");
    assert!(turn[1] > none[3], "Turn under the row");
    assert!(ui.rect("brush > Follow")[1] > turn[1], "Follow under Turn");
    let hint = |ui: &mut Ui, label: &str| {
        let all = ui.widgets("brush").into_iter();
        all.filter(|w| w["label"] == label)
            .map(|w| w["hint"].clone())
            .next()
    };
    let turns = "Turns the alpha, in degrees.";
    assert_eq!(hint(&mut ui, "Turn"), Some(json!(turns)));
    // None: back to white, the alpha that leaves the tip as it is.
    ui.ok("alpha.pick", json!({ "index": 1 }));
    ui.press("brush > None");
    assert_eq!(ui.state()["brush.alpha"], "white");
    // The ⋯: Hold turn and Map detail.
    let [x, y] = center(&more(&mut ui)["rect"]);
    ui.tap_at(x, y);
    assert!(shows(&mut ui, "Hold turn"), "the ⋯ opens");
    assert!(shows(&mut ui, "Map detail"), "the ⋯ holds Map detail");
}
