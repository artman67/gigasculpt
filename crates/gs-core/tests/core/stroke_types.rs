//! Changing a brush's stroke (decision 0051, sections 6 and 7): a patch naming another stroke
//! class keeps only the stroke settings that class reads, beside what the patch sets; Move, Pull
//! Out and the shape brushes keep their strokes; the stroke types run on every brush whose
//! kernel can run them, and a brush picked a type shows that type.

use gs_core::brush::{BrushPreset, KernelSpec, Patch, Smoothing, StrokeSpec, StrokeType};

use crate::brush_kit::{preset, shipped};
use crate::common::ok;

fn patch(text: &str) -> Patch {
    ok(toml::from_str(text))
}

/// Clay with `stroke` as its stroke table's lines.
fn clay_with(stroke: &str) -> String {
    let text = preset("clay").text;
    let cut = text.find("[stroke]").unwrap_or(text.len());
    format!("{}[stroke]\n{stroke}\n", &text[..cut])
}

fn resolve(text: &str, stroke: &str) -> Result<StrokeSpec, String> {
    let p = BrushPreset::resolve(text, &patch(&format!("[stroke]\n{stroke}")));
    p.map(|p| p.stroke).map_err(|e| e.message)
}

#[test]
fn a_patch_naming_another_class_keeps_only_the_settings_that_class_reads() {
    let smoothing = Smoothing {
        radius: 0.5,
        factor: 0.25,
    };
    let dab = clay_with(
        "class = \"dab\"\nspacing = 0.05\ntap = 0.25\ndirectional = true\n\
         smoothing = { radius = 0.5, factor = 0.25 }",
    );
    assert_eq!(
        resolve(&dab, "class = \"anchored\""),
        Ok(StrokeSpec::Anchored {
            follow: false,
            tap: 0.25,
            smoothing
        }),
        "tap and smoothing carry over; spacing and directional go"
    );
    assert_eq!(
        resolve(&dab, "class = \"curve\""),
        Ok(StrokeSpec::Curve {
            spacing: 0.05,
            straight: false,
            smoothing
        }),
        "spacing and smoothing carry over; tap and directional go"
    );
    let follow = clay_with("class = \"anchored\"\nfollow = true\ntap = 0.25");
    assert_eq!(
        resolve(&follow, "class = \"dab\"\nspacing = 0.375"),
        Ok(StrokeSpec::Dab {
            spacing: 0.375,
            tap: 0.25,
            smoothing: Smoothing::default(),
            directional: false
        }),
        "follow goes; what the patch sets is kept"
    );
    let line = clay_with("class = \"curve\"\nspacing = 0.05\nstraight = true");
    assert_eq!(
        resolve(&line, "class = \"dab\""),
        Ok(StrokeSpec::Dab {
            spacing: 0.05,
            tap: 0.5,
            smoothing: Smoothing::default(),
            directional: false
        }),
        "straight goes, and a setting no one gave takes its default"
    );
    let clay = preset("clay").text;
    assert_eq!(
        resolve(&clay, "class = \"dab\"\nspacing = 0.2"),
        Ok(StrokeSpec::Dab {
            spacing: 0.2,
            tap: 0.5,
            smoothing: Smoothing::default(),
            directional: false
        }),
        "the same class merges key by key, as before"
    );
    let e = resolve(&clay, "class = \"anchored\"\nspacing = 0.2").err();
    assert!(
        e.as_deref().is_some_and(|m| m.contains("spacing")),
        "a setting the patch gives is never dropped: {e:?}"
    );
}

#[test]
fn move_pull_out_and_the_shape_brushes_keep_their_strokes() {
    let refused = |stem: &str, stroke: &str| resolve(&preset(stem).text, stroke).err();
    assert!(
        refused("move", "class = \"dab\"").is_some(),
        "move needs drag"
    );
    assert!(
        refused("pull-out", "class = \"anchored\"").is_some(),
        "hook needs dab"
    );
    for (stem, stroke) in [
        ("select-rect", "class = \"dab\""),
        ("clip-curve", "class = \"anchored\""),
        ("clay", "class = \"gesture\"\nshape = \"polygon\""),
    ] {
        let e = refused(stem, stroke).unwrap_or_default();
        assert!(
            e.contains("gesture"),
            "{stem} to {stroke}: a shape is a brush of its own: {e}"
        );
    }
}

#[test]
fn the_stroke_types_in_the_pickers_order_by_name_title_and_class() {
    let listed: Vec<_> = (StrokeType::ALL.iter())
        .map(|t| (t.name(), t.title(), t.class()))
        .collect();
    assert_eq!(
        listed,
        [
            ("dots", "Dots", "dab"),
            ("drag_rect", "Drag Rect", "anchored"),
            ("freehand", "Freehand", "dab"),
            ("drag_dot", "Drag Dot", "anchored"),
        ]
    );
    for t in StrokeType::ALL {
        assert_eq!(StrokeType::named(t.name()), Some(t), "{}", t.name());
    }
    assert_eq!(StrokeType::named("spray"), None, "not built yet");
    for s in shipped() {
        let shown = StrokeType::of(&s.preset.stroke);
        let dab = matches!(s.preset.stroke, StrokeSpec::Dab { .. });
        let want = dab.then_some(StrokeType::Dots);
        assert_eq!(
            shown, want,
            "{}: a dab brush shows Dots, the rest none",
            s.stem
        );
    }
}

/// The stroke settings that pick `t`: its class and the setting it names, if it gives one.
fn picking(t: StrokeType) -> Patch {
    let mut stroke = toml::Table::new();
    stroke.insert("class".into(), t.class().into());
    if let (key, Some(value)) = t.setting() {
        stroke.insert(key.into(), value);
    }
    Patch::from_iter([("stroke".to_string(), toml::Value::Table(stroke))])
}

#[test]
fn every_brush_takes_each_stroke_type_its_kernel_can_run_and_shows_it() {
    for s in shipped() {
        for t in StrokeType::ALL {
            let runs = match (&s.preset.kernel, s.preset.stroke) {
                (_, StrokeSpec::Drag { .. } | StrokeSpec::Gesture { .. }) => false,
                (KernelSpec::Hook(_), _) => t.class() == "dab",
                _ => true,
            };
            let got = BrushPreset::resolve(&s.text, &picking(t));
            let name = (s.stem.as_str(), t.name());
            assert_eq!(got.is_ok(), runs, "{name:?}: {:?}", got.as_ref().err());
            let Ok(p) = got else { continue };
            assert_eq!(
                StrokeType::of(&p.stroke),
                Some(t),
                "{name:?} shows its type"
            );
            let stroke = s.preset.stroke;
            assert!(
                BrushPreset { stroke, ..p } == s.preset,
                "{name:?}: only the stroke changes"
            );
        }
    }
}
