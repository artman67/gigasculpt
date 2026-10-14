//! The themes (decision 0025, sections 6 and 13): every theme gives every color role and keeps
//! its text legible on what it sits on; a theme missing a role that has no fallback, naming an
//! unknown one or with a color it cannot read is refused by name; the user's own join the list, and one with a shipped
//! theme's name is refused.

use egui::Color32;
use gs_ui::{Theme, Themes, contrast};

/// The shipped theme files, by name.
const SHIPPED: [(&str, &str); 3] = [
    ("dark", include_str!("../../../../assets/themes/dark.toml")),
    (
        "light",
        include_str!("../../../../assets/themes/light.toml"),
    ),
    (
        "high-contrast",
        include_str!("../../../../assets/themes/high-contrast.toml"),
    ),
];

#[test]
fn every_theme_keeps_its_text_legible() {
    // WCAG 2's formula, against two ratios worked by hand: black on white, and #777777 on white,
    // just under the 4.5 floor.
    let near = |a: f32, b: f32| (a - b).abs() < 0.01;
    assert!(near(contrast(Color32::BLACK, Color32::WHITE), 21.0));
    assert!(near(
        contrast(Color32::from_gray(0x77), Color32::WHITE),
        4.48
    ));
    let themes = Themes::default().list();
    let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["dark", "light", "high-contrast"]);
    // 4.5 to 1 for text on what it sits on; 3 for text_off, which WCAG exempts.
    for t in themes {
        let faint = t.too_faint();
        assert!(faint.is_empty(), "{}: {faint:?}", t.name);
    }
}

/// A scroll bar's handle reads at least 3 to 1 on its track in every theme, at rest, hovered
/// and held (WCAG 2's 1.4.11 for a control; decision 0025, rule 11, as decision 0062 amends
/// it). egui draws the handle in the widget state's text color when the scroll style asks for
/// the foreground, else in its fill (`scroll_area.rs`, egui 0.36): `control` on `edge` was 1.54
/// to 1 in the dark theme and 1.21 in high contrast.
#[test]
fn a_scroll_handle_reads_on_its_track_in_every_theme() {
    for t in Themes::default().list() {
        let mut style = egui::Style::default();
        t.apply(&mut style);
        let (v, scroll) = (&style.visuals, &style.spacing.scroll);
        for w in [&v.widgets.inactive, &v.widgets.hovered, &v.widgets.active] {
            let handle = if scroll.foreground_color {
                w.fg_stroke.color
            } else {
                w.bg_fill
            };
            let ratio = contrast(handle, v.extreme_bg_color);
            assert!(ratio >= 3.0, "{}: the handle is {ratio:.2} to 1", t.name);
        }
    }
}

#[test]
fn every_theme_gives_every_color_role_and_one_missing_or_unknown_is_refused() {
    let roles = |text: &str| {
        let table: toml::Table = text.parse().unwrap_or_else(|e| panic!("{e}"));
        let colors = table["colors"].as_table().cloned().unwrap_or_default();
        colors.keys().cloned().collect::<Vec<_>>()
    };
    let every = roles(SHIPPED[0].1);
    assert!(every.len() >= 23, "{every:?}");
    for (name, text) in SHIPPED {
        assert_eq!(roles(text), every, "{name}");
        let read = Theme::from_toml(name, text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!read.title.is_empty(), "{name} has a title");
    }
    let light = SHIPPED[1].1;
    let missing = light.replacen("panel = \"#dfdfdf\"", "", 1);
    let err = Theme::from_toml("mine", &missing).err().unwrap_or_default();
    assert!(err.contains("missing field `panel`"), "{err}");
    let unknown = light.replacen("[colors]", "[colors]\nglow = \"#ffffff\"", 1);
    let err = Theme::from_toml("mine", &unknown).err().unwrap_or_default();
    assert!(err.contains("unknown field `glow`"), "{err}");
}

/// The axis figure's letters have a role of their own, legible on every axis color in every
/// shipped theme (the test above); a theme without it still loads, its letters in whichever
/// of `on_lit` and `text` reads better on its faintest axis color, and a role that reads
/// poorly there is named.
#[test]
fn a_theme_without_on_axis_loads_with_a_legible_fallback() {
    for (name, text) in SHIPPED {
        let without = (text.lines())
            .filter(|l| !l.starts_with("on_axis") && !l.trim_start().starts_with("# or text"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(without, text, "{name} gives on_axis");
        let read = Theme::from_toml(name, &without).unwrap_or_else(|e| panic!("{name}: {e}"));
        let c = &read.colors;
        let fallback = if name == "light" { c.text } else { c.on_lit };
        assert_eq!(c.on_axis(), fallback, "{name}");
    }
    let white = SHIPPED[0]
        .1
        .replacen("on_axis = \"#000000\"", "on_axis = \"#ffffff\"", 1);
    let read = Theme::from_toml("white", &white).unwrap_or_else(|e| panic!("{e}"));
    let faint = read.too_faint();
    assert!(
        faint.iter().any(|f| f.starts_with("on_axis on axis_y")),
        "{faint:?}"
    );
}

#[test]
fn a_color_that_is_not_hex_is_refused_by_name() {
    let broken = SHIPPED[0].1.replacen("\"#282828\"", "\"#28282\"", 1);
    let err = Theme::from_toml("dark", &broken).err().unwrap_or_default();
    assert!(err.contains("\"#28282\" is not #rrggbb"), "{err}");
}

#[test]
fn the_users_themes_join_the_list_and_a_shipped_name_is_refused() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("themes-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let light = SHIPPED[1].1;
    let write = |name: &str, text: &str| {
        std::fs::write(dir.join(name), text).unwrap_or_else(|e| panic!("{e}"));
    };
    write("sepia.toml", &light.replacen("\"Light\"", "\"Sepia\"", 1));
    write("light.toml", light);
    write("broken.toml", "title = \"Broken\"\n");
    write(
        "faint.toml",
        &light.replacen("text = \"#262626\"", "text = \"#aaaaaa\"", 1),
    );
    let (themes, problems) = Themes::load(&dir);
    let themes = themes.list();
    let names: Vec<&str> = themes.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["dark", "light", "high-contrast", "faint", "sepia"]);
    // Leaving out type and spacing keeps the dark theme's.
    assert_eq!(themes[4].sizes.body, themes[0].sizes.body);
    let said = |what: &str| problems.iter().any(|p| p.contains(what));
    assert!(said("light is a shipped theme"), "{problems:?}");
    let broken = |p: &&String| p.contains("broken.toml") && p.contains("missing field `colors`");
    assert!(problems.iter().any(|p| broken(&p)), "{problems:?}");
    assert!(said("faint.toml still loads"), "{problems:?}");
    assert!(said("make text darker"), "{problems:?}");
    assert_eq!(problems.len(), 3, "{problems:?}");
}

/// A theme copied from the light or the high-contrast one before their warning color changed
/// (Refusals-notes (6)) still loads, and the line the window shows at start names its file
/// once and its warning once, with which way to change it: darker on light surfaces, lighter
/// on dark ones.
#[test]
fn a_theme_with_an_old_warning_loads_and_says_once_which_way_to_change_it() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("old-colors-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let old = [
        (
            "old-light.toml",
            SHIPPED[1].1,
            "#4f3900",
            "#735400",
            "darker",
        ),
        (
            "old-contrast.toml",
            SHIPPED[2].1,
            "#ffee55",
            "#f0d246",
            "lighter",
        ),
    ];
    for (file, text, now, then, _) in old {
        let line = |c: &str| format!("warning = \"{c}\"");
        let copied = text.replacen(&line(now), &line(then), 1);
        assert_ne!(copied, text, "{file}");
        std::fs::write(dir.join(file), copied).unwrap_or_else(|e| panic!("{e}"));
    }
    let (themes, problems) = Themes::load(&dir);
    let names: Vec<String> = themes.list().iter().map(|t| t.name.clone()).collect();
    assert!(names.contains(&"old-light".into()), "{names:?}");
    assert!(names.contains(&"old-contrast".into()), "{names:?}");
    assert_eq!(problems.len(), 2, "{problems:?}");
    for (file, .., way) in old {
        let said = problems.iter().find(|p| p.contains(file));
        let said = said.unwrap_or_else(|| panic!("{file}: {problems:?}"));
        assert_eq!(said.matches(file).count(), 1, "{said}");
        assert_eq!(said.matches("warning").count(), 1, "{said}");
        assert!(said.contains(&format!("make warning {way}")), "{said}");
        assert!(said.contains("still loads"), "{said}");
    }
}
