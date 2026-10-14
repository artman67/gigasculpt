//! The icons (decision 0025, section 13): only the icons a control shows ship, each listed and
//! named in `icons.toml`; every entry there names a control that exists, so none goes stale
//! when a control set changes; and each icon draws as a white mask, so a tint gives it the
//! label's color.

use gs_ui::{Directions, ICONS, IconEntry, icon_entries, icon_mask};

/// Whether `e` names a control of the shipped sets, a header options button, or one of the
/// Library's two buttons not in a set (Close and Pin).
fn names_something(e: &IconEntry, all: &Directions) -> bool {
    let library = [("open", toml::Value::from(false)), ("pin", "@tile".into())];
    match (&e.command, &e.options) {
        (Some(command), None) => {
            let controls = all.controls.sets.iter().flat_map(|s| &s.controls);
            let mut same = controls.filter(|c| c.args == e.args);
            let own = |(name, value): &(&str, toml::Value)| {
                e.args == toml::Table::from_iter([(name.to_string(), value.clone())])
            };
            same.any(|c| c.command.as_ref() == Some(command))
                || (command == "ui.library" && library.iter().any(own))
        }
        (None, Some(id)) => (all.list.iter())
            .filter_map(|d| d.header.as_ref())
            .any(|s| s.options.iter().any(|o| o.id == *id)),
        _ => false,
    }
}

#[test]
fn only_the_icons_shown_ship_and_each_draws_as_a_white_mask() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons");
    let files = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}"));
    let mut svgs: Vec<String> = (files.flatten())
        .filter_map(|f| Some(f.file_name().to_str()?.strip_suffix(".svg")?.to_string()))
        .collect();
    svgs.sort();
    let listed: Vec<&str> = ICONS.iter().map(|(name, _)| *name).collect();
    assert_eq!(svgs, listed, "the list in icons.rs is the folder's SVGs");

    let entries = icon_entries().unwrap_or_else(|e| panic!("{e}"));
    let all = Directions::shipped().unwrap_or_else(|e| panic!("{e}"));
    for e in entries {
        assert!(listed.contains(&e.icon.as_str()), "{e:?}: no such icon");
        assert!(names_something(e, &all), "{e:?} names no control");
    }
    for name in &listed {
        assert!(
            entries.iter().any(|e| e.icon == *name),
            "{name} ships but no control shows it"
        );
        let mask = icon_mask(name, 24).unwrap_or_else(|| panic!("{name} does not draw"));
        let drawn = mask.pixels.iter().filter(|p| p.a() > 0).count();
        assert!(drawn > 0 && drawn < 24 * 24, "{name}: {drawn} pixels drawn");
        let white = |p: &&egui::Color32| p.r() == p.a() && p.g() == p.a() && p.b() == p.a();
        assert!(mask.pixels.iter().all(|p| white(&p)), "{name} is not white");
    }
}
