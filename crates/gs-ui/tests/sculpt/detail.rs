//! Live Detail's cursor (`geometry.live-detail-mode`): while Live Detail is on, the brush's rings
//! and lines are red over a part it cannot refine, with why beside them, purple where a stroke
//! refines, and orange where a smooth brush thins as it refines. Every kind of block has its
//! words, named and worded as before (decision 0021, section 3, amended 2026-10-12).

use egui::Color32;
use gs_engine::Blocked;
use gs_platform::Modifiers;
use gs_protocol::PenSample;
use gs_ui::Themes;
use serde_json::{Value, json};

use crate::rig::{Window, across, pen_at, with_sphere};

/// The color most of the Size ring's pixels are nearest, of `colors`, with the pen hovering at
/// (`cx`, `cy`) after `ms` milliseconds: the ring about 64 pixels out (it lies on the surface),
/// left of the pen (its lines are on the right) and beyond the Falloff ring at half the size.
fn ring_color(
    w: &mut Window,
    (cx, cy, ms): (f32, f32, u64),
    colors: &[Color32],
) -> Option<Color32> {
    w.pen(pen_at(ms, cx, cy, 0.0));
    let rect = w.view_rect();
    let width = (rect[2] - rect[0]).round() as usize;
    let mut counts = vec![0; colors.len()];
    for (i, p) in w.view_pixels().chunks(4).enumerate() {
        let x = rect[0].round() + (i % width) as f32 + 0.5 - cx;
        let y = rect[1].round() + (i / width) as f32 + 0.5 - cy;
        let d = (x * x + y * y).sqrt();
        if x >= 0.0 || !(44.0..90.0).contains(&d) {
            continue;
        }
        let far = |c: &Color32| {
            let d = |a: u8, b: u8| {
                let x = f32::from(a) - f32::from(b);
                x * x
            };
            d(p[0], c.r()) + d(p[1], c.g()) + d(p[2], c.b())
        };
        let near = (colors.iter().enumerate()).min_by(|a, b| far(a.1).total_cmp(&far(b.1)));
        if let Some((k, _)) = near.filter(|(_, c)| far(c) < 40.0 * 40.0) {
            counts[k] += 1;
        }
    }
    let (most, count) = (counts.iter().enumerate()).max_by_key(|c| c.1)?;
    (*count > 30).then_some(colors[most])
}

/// The active part's points and triangles.
fn counts(w: &mut Window) -> (u64, u64) {
    let stats = w.ok("mesh.stats", json!({}));
    let part = &stats["parts"][0];
    let n = |k: &str| part[k].as_u64().unwrap_or_default();
    (n("vertices"), n("triangles"))
}

/// A stroke across the middle of `rect`, its samples timed after `ms` milliseconds, and the
/// frames the window draws after the lift: the held keys take their slot from the one after.
fn stroke(w: &mut Window, rect: [f32; 4], ms: u64) {
    for s in across(rect, 12) {
        w.pen(PenSample {
            t_ns: s.t_ns + ms * 1_000_000,
            ..s
        });
    }
    w.wait(0.05);
}

#[test]
fn the_cursor_says_whether_live_detail_refines_under_it() {
    let (mut w, rect) = with_sphere(None);
    let c = Themes::default().list()[0].colors.clone();
    let (purple, orange, red) = (c.detail(false), c.detail(true), c.subtract);
    let colors = [purple, orange, red, c.brush, c.smooth, c.mask];
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    let at = |ms| (cx, cy, ms);
    let off = ring_color(&mut w, at(0), &colors);
    assert_eq!(off, Some(c.brush), "Live Detail off");
    w.ok("level.subdivide", json!({}));
    // Edges a twentieth of the brush radius: finer than the sphere's, so a stroke splits them.
    w.ok("live_detail.set", json!({ "on": true, "size": 0.05 }));
    // Detail levels keep their topology: red, saying why, and a stroke keeps the count.
    assert_eq!(
        ring_color(&mut w, at(50), &colors),
        Some(red),
        "detail levels"
    );
    let shown = w.cursor_lines();
    assert!(
        shown.iter().any(|l| l == "No Live Detail: detail levels"),
        "{shown:?}"
    );
    let before = counts(&mut w);
    stroke(&mut w, rect, 100);
    assert_eq!(counts(&mut w).0, before.0, "a stroke keeps the point count");
    // With the levels deleted the part refines: purple, and the same stroke adds triangles.
    w.ok("level.delete_lower", json!({}));
    assert_eq!(
        ring_color(&mut w, at(200), &colors),
        Some(purple),
        "refines"
    );
    assert!(w.cursor_lines().iter().any(|l| l == "Live Detail"));
    let before = counts(&mut w);
    stroke(&mut w, rect, 300);
    let after = counts(&mut w);
    assert!(
        after.1 > before.1,
        "the stroke adds triangles: {before:?} to {after:?}"
    );
    // Shift holds the Smooth slot, which thins as it smooths: orange, not Smooth's blue.
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    w.input(&[], shift);
    assert_eq!(ring_color(&mut w, at(400), &colors), Some(orange), "Smooth");
    let shown = w.cursor_lines();
    assert!(
        shown.iter().any(|l| l == "Live Detail: smooths and thins"),
        "{shown:?}"
    );
    // Ctrl's Mask Pen changes no shape: it keeps the mask's yellow, and says nothing of it.
    let ctrl = Modifiers {
        ctrl: true,
        ..Modifiers::default()
    };
    w.input(&[], ctrl);
    assert_eq!(
        ring_color(&mut w, at(500), &colors),
        Some(c.mask),
        "Mask Pen"
    );
    let shown = w.cursor_lines();
    assert!(
        !shown.iter().any(|l| l.contains("Live Detail")),
        "{shown:?}"
    );
}

/// Every part of the status line, left to right.
fn status_line(w: &mut Window) -> Vec<String> {
    let d = w.ok("ui.describe", json!({}));
    let panels = d["panels"].as_array().cloned().unwrap_or_default();
    let status = panels.iter().find(|p| p["id"] == "status").cloned();
    let widgets = status.and_then(|v| v["widgets"].as_array().cloned());
    (widgets.unwrap_or_default().iter())
        .filter_map(|x| Some(x["label"].as_str()?.to_string()))
        .collect()
}

/// A kind of block: the calls that bring it about, its name in `live_detail.blocked`, its words
/// after "No Live Detail: ", and whether the status line says them too.
type Row = (
    Blocked,
    Vec<(&'static str, Value)>,
    &'static str,
    &'static str,
    bool,
);

/// One row per kind of block, in an order each brings its own about: the brush's, then the
/// part's (a copy of the sphere with UV columns, which becomes active), then detail levels.
fn rows(w: &mut Window) -> Vec<Row> {
    let draw = |[backface, face_group, topology]: [f32; 3], enable: bool| {
        let automask =
            json!({ "backface": backface, "face_group": face_group, "topology": topology });
        let settings = json!({ "automask": automask, "live_detail": { "enable": enable } });
        (
            "brush.set",
            json!({ "brush": "Draw", "settings": settings }),
        )
    };
    let mut mapped = w.ok("part.get", json!({ "part": "sphere" }));
    let faces = mapped["faces"].as_array().map_or(0, Vec::len);
    for name in ["uv0.u", "uv0.v"] {
        let values = json!({ "f32": vec![0.0; faces] });
        mapped["attributes"][name] = json!({ "domain": "face", "values": values });
    }
    let mapped = json!({ "name": "mapped", "mesh": mapped });
    let levels = vec![
        ("part.select", json!({ "part": "sphere" })),
        ("level.subdivide", json!({})),
    ];
    vec![
        (
            Blocked::Brush,
            vec![("brush.set", json!({ "brush": "Move" }))],
            "brush",
            "this brush",
            false,
        ),
        (
            Blocked::Backface,
            vec![draw([1.0, 0.0, 0.0], true)],
            "backface",
            "backface automask",
            true,
        ),
        (
            Blocked::FaceGroup,
            vec![draw([0.0, 0.5, 0.0], true)],
            "face_group",
            "face group automask",
            true,
        ),
        (
            Blocked::Topology,
            vec![draw([0.0, 0.0, 2.0], true)],
            "topology",
            "topology automask",
            true,
        ),
        (
            Blocked::OptOut,
            vec![draw([0.0; 3], false)],
            "opt_out",
            "off for this brush",
            false,
        ),
        (
            Blocked::Uvs,
            vec![draw([0.0; 3], true), ("part.create", mapped)],
            "uvs",
            "UV columns",
            false,
        ),
        (Blocked::Levels, levels, "levels", "detail levels", false),
    ]
}

/// Every kind of block has its words beside the pen, and, for the three automasks, on the
/// status line: the same words as before, under the same name in `live_detail.blocked`.
#[test]
fn every_kind_of_live_detail_block_is_named_and_worded_as_before() {
    let (mut w, rect) = with_sphere(None);
    let (cx, cy) = (
        f32::midpoint(rect[0], rect[2]),
        f32::midpoint(rect[1], rect[3]),
    );
    w.ok("live_detail.set", json!({ "on": true }));
    let mut named = Vec::new();
    for (k, (kind, calls, name, words, status)) in (0..).zip(rows(&mut w)) {
        for (command, args) in calls {
            w.ok(command, args);
        }
        w.pen(pen_at(100 * k, cx, cy, 0.0));
        w.wait(0.05);
        let state = w.ok("session.state", json!({}));
        assert_eq!(state["live_detail.blocked"], name, "{kind:?}");
        assert_eq!(json!(kind), name, "{kind:?}");
        let line = format!("No Live Detail: {words}");
        let shown = w.cursor_lines();
        assert!(shown.contains(&line), "{kind:?} beside the pen: {shown:?}");
        let shown = status_line(&mut w);
        assert_eq!(shown.contains(&line), status, "{kind:?}: {shown:?}");
        named.push(kind);
    }
    for kind in Blocked::ALL {
        assert!(named.contains(&kind), "{kind:?} has a row here");
    }
}
