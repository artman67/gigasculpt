//! Saving a preset into its file's text (decision 0018, "Saving a brush"): an unchanged preset
//! saves its file back byte for byte, a changed setting is edited where it stands with every
//! other line kept, and a file in a later format is refused by its version.

use gs_core::BrushPreset;
use gs_core::brush::Patch;

use crate::brush_kit::{preset, shipped};
use crate::common::ok;

fn patch(text: &str) -> Patch {
    ok(toml::from_str(text))
}

fn refused(text: &str, want: &str) {
    match BrushPreset::from_toml(text) {
        Ok(_) => panic!("accepted:\n{text}"),
        Err(e) => assert!(e.message.contains(want), "{e} should say {want:?}"),
    }
}

#[test]
fn every_shipped_preset_saves_back_into_its_file_byte_for_byte() {
    for s in shipped() {
        let same = ok(BrushPreset::resolve(&s.text, &Patch::new()));
        assert_eq!(ok(same.write_into(&s.text, &[])), s.text, "{}", s.stem);
        // A setting changed to what the file already says changes nothing either.
        let strength = patch(&format!("strength = {}", s.preset.strength));
        let same = ok(BrushPreset::resolve(&s.text, &strength));
        assert_eq!(ok(same.write_into(&s.text, &[])), s.text, "{}", s.stem);
        // Changed, only its line changes, in every shipped file's layout.
        let other = if s.preset.strength > 0.5 { 0.25 } else { 0.75 };
        let strength = patch(&format!("strength = {other}"));
        let changed = ok(BrushPreset::resolve(&s.text, &strength));
        let out = ok(changed.write_into(&s.text, &[]));
        let differ: Vec<(&str, &str)> = (s.text.lines().zip(out.lines()))
            .filter(|(a, b)| a != b)
            .collect();
        let line = format!("strength = {other}");
        assert_eq!(out.lines().count(), s.text.lines().count(), "{}", s.stem);
        assert!(
            differ.len() == 1 && differ[0].1 == line,
            "{}: {differ:?}",
            s.stem
        );
    }
}

#[test]
fn a_changed_setting_is_written_where_it_stands_and_every_other_line_kept() {
    let text = (preset("clay").text).replace("strength = 0.25\n", "strength = 0.25 # how hard\n");
    let changes = patch(
        r#"
        strength = 0.5
        modifier = 30.0
        tip = { focal = -20.0, alpha = { image = "ring" } }
        stroke = { spacing = 0.2 }
        paint = { channel = "color", color_strength = 0.5 }
        "#,
    );
    let want = ok(BrushPreset::resolve(&text, &changes));
    let out = ok(want.write_into(&text, &[]));
    let expected = text
        .replace("0.25 # how hard", "0.5 # how hard")
        .replace("\"reference\"]\n", "\"reference\"]\nmodifier = 30.0\n")
        .replace(
            "profile = \"smooth\"\n",
            "profile = \"smooth\"\nfocal = -20.0\n",
        )
        .replace("\"grainy-round\"", "\"ring\"")
        .replace("spacing = 0.1", "spacing = 0.2")
        + "\n[paint]\nchannel = \"color\"\ncolor_strength = 0.5\n";
    assert_eq!(out, expected, "only the changed settings' lines change");
    assert_eq!(ok(BrushPreset::from_toml(&out)), want, "and it reads back");
    // The settings between the file and the brush are the patch, so a saved file loads back
    // as a patch over its shipped preset.
    let found = ok(BrushPreset::from_toml(&text)).changes(&want);
    assert!(found.removed.is_empty(), "{:?}", found.removed);
    assert_eq!(found.set, changes, "the changes are the patch");
}

#[test]
fn a_table_the_file_lacks_gets_only_the_settings_that_differ_from_its_defaults() {
    let draw = preset("draw");
    let ring = patch(r#"tip = { alpha = { image = "ring", mid = 0.0 } }"#);
    let want = ok(BrushPreset::resolve(&draw.text, &ring));
    let out = ok(want.write_into(&draw.text, &[]));
    let expected = draw.text.replace(
        "profile = \"smooth\"\n",
        "profile = \"smooth\"\n\n[tip.alpha]\nimage = \"ring\"\n",
    );
    assert_eq!(out, expected, "after its parent, defaults left out");
    // The same holds for the settings between two presets.
    let mut found = draw.preset.changes(&want);
    found.trim(|c| BrushPreset::resolve(&draw.text, &c.set).is_ok_and(|p| p == want));
    assert_eq!(found.set, patch(r#"tip = { alpha = { image = "ring" } }"#));
}

#[test]
fn a_setting_the_brush_no_longer_has_is_removed_with_its_lines() {
    let clay = preset("clay");
    let mut plain = clay.preset.clone();
    plain.tip.alpha = None;
    let out = ok(plain.write_into(&clay.text, &[]));
    let expected = clay
        .text
        .replace("\n[tip.alpha]\nimage = \"grainy-round\"\n", "");
    assert_eq!(out, expected);
    let found = clay.preset.changes(&plain);
    assert_eq!(found.removed, ["tip.alpha"], "a patch cannot say this");
}

#[test]
fn a_named_setting_is_written_even_where_it_equals_the_default() {
    // Naming a size curve keeps the session's global one off (decision 0036), so a brush whose
    // patch names its default size curve must save the name, not drop it as unchanged.
    let draw = preset("draw");
    let out = ok(draw.preset.write_into(&draw.text, &["pressure.size"]));
    assert!(out.starts_with(&draw.text), "{out}");
    let added = &out[draw.text.len()..];
    assert!(added.starts_with("\n[pressure]\nsize = [["), "{added}");
    assert_eq!(added.lines().count(), 3, "{added}");
    assert_eq!(ok(BrushPreset::from_toml(&out)), draw.preset, "same brush");
    // A file that names it already is kept byte for byte.
    let moved = preset("move");
    let same = ok(moved.preset.write_into(&moved.text, &["pressure.size"]));
    assert_eq!(same, moved.text);
}

#[test]
fn a_file_in_a_later_format_is_refused_by_its_version() {
    let draw = preset("draw").text.replace("version = 1\n", "");
    let later = "version: this file is brush preset format 2, and this build reads format 1";
    // Refused by its version before the keys it adds.
    refused(&format!("version = 2\n{draw}\n[future]\nkey = 1\n"), later);
    refused(&format!("version = 2\n{draw}"), later);
    refused(
        &format!("version = 0\n{draw}"),
        "version must be from 1 to 1, got 0",
    );
    let one = ok(BrushPreset::from_toml(&format!("version = 1\n{draw}")));
    assert_eq!(one, ok(BrushPreset::from_toml(&draw)), "left out, it is 1");
    assert_eq!(one.version, gs_core::brush::FORMAT);
    assert!(
        !one.canonical().contains("version"),
        "not in the canonical form"
    );
    let e = BrushPreset::resolve(&draw, &patch("version = 1")).err();
    let e = e.map(|e| e.message).unwrap_or_default();
    assert!(e.contains("settings cannot change version"), "{e}");
}

#[test]
fn an_unknown_field_is_refused_by_name_never_dropped_on_save() {
    let draw = preset("draw");
    let text = draw.text.replace("[tip]\n", "[tip]\nspin = 3\n");
    refused(&text, "unknown field `spin`");
    let e = draw.preset.write_into(&text, &[]).err();
    let e = e.map(|e| e.message).unwrap_or_default();
    assert!(e.contains("unknown field `spin`"), "{e}");
}

/// A preset file saved by a Windows editor ends every line in CRLF. It reads as the same brush,
/// saves back byte for byte, and each edit is the one an LF file gets, in the file's own line
/// endings: a changed line, an added key and table, a removed table, a named setting.
#[test]
fn a_file_with_windows_line_endings_is_read_and_edited_in_its_own_endings() {
    let crlf = |text: &str| text.replace('\n', "\r\n");
    let same_edit = |p: &BrushPreset, text: &str, named: &[&str], what: &str| {
        let lf = ok(p.write_into(text, named));
        assert_eq!(ok(p.write_into(&crlf(text), named)), crlf(&lf), "{what}");
    };
    for s in shipped() {
        let text = crlf(&s.text);
        assert_eq!(ok(BrushPreset::from_toml(&text)), s.preset, "{}", s.stem);
        assert_eq!(ok(s.preset.write_into(&text, &[])), text, "{}", s.stem);
        let other = if s.preset.strength > 0.5 { 0.25 } else { 0.75 };
        let strength = patch(&format!("strength = {other}"));
        let changed = ok(BrushPreset::resolve(&text, &strength));
        assert_eq!(changed, ok(BrushPreset::resolve(&s.text, &strength)));
        same_edit(&changed, &s.text, &[], &s.stem);
    }
    let clay = preset("clay");
    let changes = patch(
        r#"
        modifier = 30.0
        tip = { focal = -20.0, alpha = { image = "ring" } }
        paint = { channel = "color", color_strength = 0.5 }
        "#,
    );
    same_edit(
        &ok(BrushPreset::resolve(&clay.text, &changes)),
        &clay.text,
        &[],
        "an added key and table",
    );
    let mut plain = clay.preset.clone();
    plain.tip.alpha = None;
    same_edit(&plain, &clay.text, &[], "a removed table");
    let draw = preset("draw");
    same_edit(
        &draw.preset,
        &draw.text,
        &["pressure.size"],
        "a named setting",
    );
}
