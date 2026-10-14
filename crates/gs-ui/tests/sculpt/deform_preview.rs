//! The open deformer's preview on the graphics card, in the window (decision 0041, amended
//! 2026-10-12 for `DF9a`): a pull sends the part's weights once, then only the operations' small
//! block, and Cancel and Reset send nothing; a screenshot mid-pull shows the preview, drawn from
//! the buffers of the open, as the part looks after Accept; `view.snapshot`, drawn from the
//! stores, catches up and shows the same.

use gs_ui::Render;
use serde_json::{Value, json};

use crate::rig::{Window, decode_png, with_sphere};

/// A bend of `degrees` toward x along y, about a frame off the sphere's vertices, with the box
/// the window always sends.
fn bend(degrees: f32) -> Value {
    let frame = json!({ "origin": [0.05, 0.03, 0.02], "axes": [0.0, 0.0, 0.0, 1.0] });
    let bounds = json!({ "lo": [-1.05, -1.03, -1.02], "hi": [0.95, 0.97, 0.98] });
    let op = json!({ "bend": { "axis": "y", "toward": "x", "degrees": degrees } });
    json!({ "part": "sphere", "frame": frame, "op": op, "box": bounds })
}

pub(crate) fn deformer(w: &mut Window, phase: &str, degrees: Option<f32>) {
    let mut args = json!({ "phase": phase });
    if let Some(d) = degrees {
        args["transforms"] = json!([bend(d)]);
    }
    w.ok("gizmo.deformer", args);
}

/// Bytes the window sent to the GPU after the last call.
fn uploaded(w: &Window) -> u64 {
    w.host.lock().render.as_ref().map_or(0, Render::uploaded)
}

/// How many pixels differ by more than a little in any channel.
fn differ(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "the same size");
    (a.chunks(4).zip(b.chunks(4)))
        .filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > 24))
        .count()
}

/// `view.snapshot` of the main view, one sample per pixel.
fn snapshot(w: &mut Window) -> Vec<u8> {
    let args = json!({ "view": "main", "size": [320, 200], "antialias": false });
    decode_png(
        w.ok("view.snapshot", args)["png_base64"]
            .as_str()
            .unwrap_or_default(),
    )
}

#[test]
fn a_pull_sends_the_weights_once_then_only_the_operations() {
    let (mut w, _) = with_sphere(None);
    let stats = w.ok("mesh.stats", json!({}));
    let points = stats["parts"][0]["vertices"].as_u64().unwrap_or_default();
    deformer(&mut w, "open", Some(0.0));
    let weights = uploaded(&w);
    assert!(
        weights >= 2 * points && weights <= 2 * points + 4096,
        "the open sends 2 bytes a point and the block: {weights} for {points} points"
    );
    for degrees in [20.0, 45.0, 70.0] {
        deformer(&mut w, "update", Some(degrees));
        let sent = uploaded(&w);
        assert!(sent <= 1024, "an update sends the block alone: {sent}");
    }
    deformer(&mut w, "reset", None);
    assert_eq!(uploaded(&w), 0, "Reset sends nothing");
    deformer(&mut w, "update", Some(30.0));
    assert!(uploaded(&w) <= 1024, "nor the weights again");
    deformer(&mut w, "cancel", None);
    assert_eq!(uploaded(&w), 0, "Cancel sends nothing");
}

#[test]
fn a_screenshot_mid_pull_shows_the_preview_as_accept_draws_it() {
    let (mut w, _) = with_sphere(None);
    // view.snapshot renders on the window's own device, as in the app.
    let host = w.host.clone();
    w.engine.attach_renderer(Box::new(host));
    let before = w.view_pixels();
    deformer(&mut w, "open", Some(0.0));
    deformer(&mut w, "update", Some(80.0));
    assert!(uploaded(&w) <= 1024, "drawn from the open's buffers");
    let pulled = w.view_pixels();
    let caught = snapshot(&mut w);
    deformer(&mut w, "accept", None);
    let (accepted, committed) = (w.view_pixels(), snapshot(&mut w));
    let covered = before.chunks(4).count();
    // The sphere fills a small part of the view: the bend changes about 2% of its pixels.
    assert!(
        differ(&before, &pulled) > covered / 100,
        "the screenshot shows the bend"
    );
    let off = differ(&pulled, &accepted);
    assert!(
        off <= covered / 500,
        "the preview looks as Accept leaves it: {off} pixels differ"
    );
    assert_eq!(
        differ(&caught, &committed),
        0,
        "view.snapshot mid-pull draws the committed shape"
    );
}
