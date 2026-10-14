//! Live Boolean in the window (decision 0057, section 9, as amended for step LB7): a settled
//! group's result drawn in place of its operands, its walls in its operation's tint and the
//! rest in the part's own material, a Difference's or an Intersect's cutter as an outline
//! only; a refused group and the switch off as if never on; the active part's mark and the mask
//! on the result; Show Issues and Show Coplanar marking faces; a landing drawing a new frame
//! and a camera turn uploading nothing; and a cutter dragged with the gizmo drawing its hole
//! through the peel until its run lands (as amended for LB8). The pictures are compared with each other, so nothing
//! hangs on a shading's exact bytes but the outline's color, which is written as it is.

use gs_engine::Tint;
use gs_protocol::PlainMesh;
use gs_render::BACKGROUND;
use gs_ui::{Render, Themes};
use serde_json::json;

use crate::rig::Window;

/// The 3D view's pixels, as presented, its size, and where the corner figure and the Zoom and
/// Pan buttons draw over it while they show (in a wide enough view), a little grown, in its own
/// pixels.
struct Picture {
    rgba: Vec<u8>,
    width: i64,
    height: i64,
    over: Vec<[i64; 4]>,
}

impl Picture {
    fn of(w: &mut Window) -> Picture {
        let r = w.view_rect().map(|v| v.round() as i64);
        let d = w.ok("ui.describe", json!({}));
        let panels = d["panels"].as_array().cloned().unwrap_or_default();
        let grown = |p: &serde_json::Value| -> [i64; 4] {
            let o: [i64; 4] = std::array::from_fn(|i| p["rect"][i].as_f64().unwrap_or(0.0) as i64);
            [
                o[0] - r[0] - 3,
                o[1] - r[1] - 3,
                o[2] - r[0] + 3,
                o[3] - r[1] + 3,
            ]
        };
        let overlays = panels
            .iter()
            .filter(|p| p["id"] == "figure" || p["id"] == "pads");
        let (rgba, width) = (w.view_pixels(), r[2] - r[0]);
        Picture {
            over: overlays.map(grown).collect(),
            height: i64::try_from(rgba.len() / 4).unwrap_or(0) / width,
            rgba,
            width,
        }
    }

    fn at(&self, x: i64, y: i64) -> Option<[u8; 4]> {
        let inside = (0..self.width).contains(&x) && (0..self.height).contains(&y);
        let i = || 4 * (y * self.width + x) as usize;
        inside.then(|| std::array::from_fn(|k| self.rgba[i() + k]))
    }

    /// Every pixel the interface draws nothing over, as (x, y).
    fn all(&self) -> impl Iterator<Item = (i64, i64)> + use<> {
        let (width, height) = (self.width, self.height);
        let over = self.over.clone();
        let clear = move |&(x, y): &(i64, i64)| {
            !(over.iter()).any(|o| x >= o[0] && x < o[2] && y >= o[1] && y < o[3])
        };
        (0..height)
            .flat_map(move |y| (0..width).map(move |x| (x, y)))
            .filter(clear)
    }

    fn part(&self, x: i64, y: i64) -> bool {
        self.at(x, y).is_some_and(|p| p != BACKGROUND)
    }

    /// Pixels where `self` and `other` differ, in any channel by more than a little.
    fn differ(&self, other: &Picture) -> usize {
        (self.rgba.chunks(4).zip(other.rgba.chunks(4)))
            .filter(|(p, q)| p.iter().zip(*q).any(|(a, b)| a.abs_diff(*b) > 24))
            .count()
    }
}

/// The window's theme's color for `tint`, as the outline writes it.
fn color(tint: Tint) -> [u8; 4] {
    let dark = Themes::default().find("dark");
    let dark = dark.unwrap_or_else(|| panic!("the dark theme ships"));
    dark.colors.live(tint).to_array()
}

/// Whether `p` has `tint`'s hue, however lit: their colors less their own grays point the same
/// way, and `p` is not a gray.
fn hued(p: [u8; 4], tint: Tint) -> bool {
    let chroma = |c: [u8; 4]| {
        let gray = (0..3).map(|k| f32::from(c[k])).sum::<f32>() / 3.0;
        [0, 1, 2].map(|k| f32::from(c[k]) - gray)
    };
    let (a, b) = (chroma(p), chroma(color(tint)));
    let dot = |u: [f32; 3], v: [f32; 3]| (0..3).map(|k| u[k] * v[k]).sum::<f32>();
    let length = dot(a, a).sqrt();
    length > 12.0 && dot(a, b) > 0.95 * length * dot(b, b).sqrt()
}

/// A sphere and a cube of `size` at `center` marked `operation`, the sphere active, seen from
/// the front, with the active part's outline off.
fn pair(operation: &str, size: f32, center: [f32; 3]) -> Window {
    let mut w = Window::new(None);
    w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    let cube = json!({ "name": "cutter", "size": size, "center": center });
    w.ok("primitive.cube", cube);
    let marked = json!({ "part": "cutter", "operation": operation });
    w.ok("part.set_operation", marked);
    w.ok("part.select", json!({ "part": "sphere" }));
    w.ok("view.display", json!({ "active_outline": false }));
    w.ok("view.set", json!({ "view": "main", "camera": "front" }));
    w
}

/// The pair's pictures: the sphere alone, the cube alone, both with the switch off, and both
/// with it on once the group's run landed. The active part always shows, so each is made
/// active to be seen alone.
fn pictures(w: &mut Window) -> [Picture; 4] {
    let mut alone = |active: &str, hidden: &str| {
        w.ok("part.select", json!({ "part": active }));
        w.ok(
            "part.set_visible",
            json!({ "parts": [hidden], "visible": false }),
        );
        let picture = Picture::of(w);
        w.ok(
            "part.set_visible",
            json!({ "parts": [hidden], "visible": true }),
        );
        picture
    };
    let (cut, body) = (alone("cutter", "sphere"), alone("sphere", "cutter"));
    let off = Picture::of(w);
    w.ok("view.display", json!({ "live_boolean": true }));
    let _ = w.engine.tick();
    [body, cut, off, Picture::of(w)]
}

/// The cube over the sphere's front right, past its edge: x 0.35 to 1.15, z 0.4 to 1.2.
const OVER: [f32; 3] = [0.75, 0.0, 0.8];

/// Pixels of `p`, away from the cube's (`cut`'s) by more than the seam's band of new normals,
/// on which `p` and `q` differ by more than a little; and how many there are.
fn rest_changed(p: &Picture, q: &Picture, cut: &Picture) -> (usize, usize) {
    let far = |x: i64, y: i64| (-12..=12).all(|d| !cut.part(x + d, y) && !cut.part(x, y + d));
    let rest: Vec<_> = (p.all())
        .filter(|&(x, y)| p.part(x, y) && far(x, y))
        .collect();
    let differ = |&&(x, y): &&(i64, i64)| {
        let (a, b) = (
            p.at(x, y).unwrap_or_default(),
            q.at(x, y).unwrap_or_default(),
        );
        a.iter().zip(b).any(|(s, t)| s.abs_diff(t) > 24)
    };
    (rest.iter().filter(differ).count(), rest.len())
}

/// Pixels exactly `tint`'s color.
fn ring(p: &Picture, tint: Tint) -> usize {
    let c = Some(color(tint));
    (p.all()).filter(|&(x, y)| p.at(x, y) == c).count()
}

#[test]
fn a_settled_difference_draws_the_hole_in_its_tint_and_the_cutter_as_an_outline_only() {
    let mut w = pair("difference", 0.8, OVER);
    let [body, cut, _, live] = pictures(&mut w);
    let (mut outside, mut hole, mut tinted) = (0, 0, 0);
    for (x, y) in (live.all()).filter(|&(x, y)| cut.part(x, y)) {
        if body.part(x, y) {
            hole += 1;
            tinted += usize::from(live.at(x, y).is_some_and(|p| hued(p, Tint::Difference)));
        } else {
            outside += 1;
            assert_eq!(
                live.at(x, y),
                Some(BACKGROUND),
                "({x}, {y}): no cutter drawn"
            );
        }
    }
    assert!(
        outside > 100,
        "the cutter passes the sphere's edge: {outside} pixels"
    );
    assert!(
        tinted > hole / 3,
        "the walls show in the tint: {tinted} of {hole} pixels"
    );
    let outlined = ring(&live, Tint::Difference);
    assert!(outlined > 100, "the cutter is outlined: {outlined} pixels");
    let (changed, rest) = rest_changed(&body, &live, &cut);
    assert!(
        changed <= rest / 100,
        "the rest is the sphere's own: {changed} of {rest}"
    );
}

#[test]
fn an_intersect_keeps_the_overlap_and_an_add_merges_without_an_outline() {
    let mut w = pair("intersect", 0.8, OVER);
    let [body, cut, _, live] = pictures(&mut w);
    let outline = Some(color(Tint::Intersect));
    for (x, y) in live.all().filter(|&(x, y)| !cut.part(x, y)) {
        let p = live.at(x, y);
        let near = (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)));
        let beside = (near.filter(|(dx, dy)| dx * dx + dy * dy <= 4))
            .any(|(dx, dy)| cut.part(x + dx, y + dy));
        let want = if beside { outline } else { Some(BACKGROUND) };
        assert_eq!(
            p, want,
            "({x}, {y}): only the overlap and the cutter's outline"
        );
    }
    let kept = (live.all()).filter(|&(x, y)| cut.part(x, y) && body.part(x, y) && live.part(x, y));
    assert!(kept.count() > 100, "the overlap shows");
    let mut w = pair("union", 0.8, OVER);
    let [_, cut, off, live] = pictures(&mut w);
    for tint in [Tint::Union, Tint::Difference, Tint::Intersect] {
        assert_eq!(ring(&live, tint), 0, "an Add draws no outline");
    }
    let (changed, rest) = rest_changed(&off, &live, &cut);
    assert!(
        changed <= rest / 100,
        "an Add draws as its parts did: {changed} of {rest}"
    );
}

#[test]
fn a_refused_group_and_the_switch_off_draw_byte_for_byte_as_never_on() {
    // A cutter around the whole sphere leaves nothing: the group is refused.
    let mut w = pair("difference", 3.0, [0.0; 3]);
    let [_, _, off, live] = pictures(&mut w);
    let state = w.ok("session.state", json!({}));
    let group = &state["boolean.live"]["groups"][0];
    assert_eq!(group["state"], json!("refused"), "the cut leaves nothing");
    assert!(
        off.rgba == live.rgba,
        "a refused group draws its parts plain"
    );
    let mut w = pair("difference", 0.8, OVER);
    let [_, _, off, live] = pictures(&mut w);
    assert!(off.differ(&live) > 100, "on, the result draws");
    w.ok("view.display", json!({ "live_boolean": false }));
    assert!(
        off.rgba == Picture::of(&mut w).rgba,
        "off again, as never on"
    );
}

fn orange(p: Option<[u8; 4]>) -> bool {
    let full = [255, 170, 64];
    p.is_some_and(|p| (0..3).all(|k| p[k].abs_diff(full[k]) <= 1))
}

#[test]
fn the_results_mark_and_its_mask_still_show_and_an_active_cutter_is_outlined_in_orange() {
    let mut w = pair("difference", 0.8, OVER);
    let [_, cut, _, live] = pictures(&mut w);
    w.ok("view.display", json!({ "active_outline": true }));
    let marked = Picture::of(&mut w);
    let mut outlined = 0;
    for (x, y) in marked.all().filter(|&(x, y)| orange(marked.at(x, y))) {
        outlined += 1;
        assert!(
            !live.part(x, y),
            "({x}, {y}): outside the result, never on its walls"
        );
    }
    assert!(outlined > 100, "the result is outlined: {outlined} pixels");
    w.ok("part.select", json!({ "part": "cutter" }));
    let active = Picture::of(&mut w);
    let oranges = (active.all())
        .filter(|&(x, y)| orange(active.at(x, y)) && !cut.part(x, y))
        .count();
    assert_eq!(
        ring(&active, Tint::Difference),
        0,
        "the active cutter is not in its tint"
    );
    assert!(
        oranges > 100,
        "but the active part's orange: {oranges} pixels"
    );
    w.ok("part.select", json!({ "part": "sphere" }));
    w.ok("view.display", json!({ "active_outline": false }));
    w.ok("mask.set", json!({ "part": "sphere", "value": 1.0 }));
    let _ = w.engine.tick();
    let masked = Picture::of(&mut w);
    let luma = |p: Option<[u8; 4]>| p.map_or(0, |p| p[..3].iter().map(|&v| u32::from(v)).sum());
    let result = |x, y| live.part(x, y) && live.at(x, y) != Some(color(Tint::Difference));
    let covered = live.all().filter(|&(x, y)| result(x, y)).count();
    let darker = (live.all())
        .filter(|&(x, y)| result(x, y) && luma(masked.at(x, y)) < luma(live.at(x, y)))
        .count();
    assert!(
        darker > covered * 9 / 10,
        "the mask darkens the result: {darker} of {covered}"
    );
}

/// A cube over -1 to 1 with its -x side's faces taken off, and the points only they used: open
/// along that side's edges.
fn open_cube() -> PlainMesh {
    let mut mesh = gs_testkit::fixtures::cube(4);
    let left = |f: &[u32; 4]| f.iter().all(|&v| mesh.positions[v as usize][0] < -0.999);
    let kept: Vec<[u32; 4]> = mesh.faces.iter().copied().filter(|f| !left(f)).collect();
    let mut used: Vec<u32> = kept.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    let at = |v: u32| used.binary_search(&v).unwrap_or_default() as u32;
    mesh.faces = kept.iter().map(|f| f.map(at)).collect();
    mesh.positions = used.iter().map(|&v| mesh.positions[v as usize]).collect();
    mesh
}

/// Pixels of `tint`'s hue, as the marks are drawn lit.
fn tinted(p: &Picture, tint: Tint) -> usize {
    (p.all())
        .filter(|&(x, y)| p.at(x, y).is_some_and(|q| hued(q, tint)))
        .count()
}

#[test]
fn the_issue_marks_show_an_open_edge_and_a_coplanar_pair_and_nothing_on_a_clean_part() {
    let marks = json!({ "show_issues": true, "show_coplanar": true, "live_boolean": true });
    let mut w = pair("difference", 0.8, OVER);
    w.ok("view.display", marks.clone());
    w.ok("view.set", json!({ "view": "main", "camera": "iso" }));
    let _ = w.engine.tick();
    let clean = Picture::of(&mut w);
    let mut w = Window::new(None);
    w.ok("part.create", json!({ "name": "box", "mesh": open_cube() }));
    // Its top face on the box's: the two are coplanar there.
    let top = json!({ "name": "cutter", "size": 1.0, "center": [0.0, 0.5, 0.0] });
    w.ok("primitive.cube", top);
    let difference = json!({ "part": "cutter", "operation": "difference" });
    w.ok("part.set_operation", difference);
    w.ok("view.set", json!({ "view": "main", "camera": "iso" }));
    w.ok("view.display", json!({ "active_outline": false }));
    w.ok("view.display", marks);
    let _ = w.engine.tick();
    let flagged = Picture::of(&mut w);
    for tint in [Tint::Issue, Tint::Coplanar] {
        assert_eq!(tinted(&clean, tint), 0, "{tint:?}: nothing on a clean part");
        let shown = tinted(&flagged, tint);
        assert!(shown > 100, "{tint:?}: {shown} pixels marked");
    }
}

/// Bytes the window sent to the GPU after the last call.
fn uploaded(w: &Window) -> u64 {
    w.host.lock().render.as_ref().map_or(0, Render::uploaded)
}

#[test]
fn a_landing_draws_a_new_frame_and_a_camera_turn_uploads_nothing() {
    let mut w = pair("difference", 0.8, OVER);
    w.ok("view.display", json!({ "live_boolean": true }));
    w.host.lock().dirty = false;
    let _ = w.engine.tick();
    assert!(w.host.lock().dirty, "the landing asks for a frame");
    assert!(uploaded(&w) > 0, "and sends the result once");
    w.ok("view.orbit", json!({ "view": "main", "right_deg": 10 }));
    assert_eq!(uploaded(&w), 0, "a turn sends nothing");
}

/// The drag stays open, so no command is logged and no run starts until it ends. Measured on
/// lavapipe: 8,563 pixels moved from the last result, and 859 apart from the landed one, at the
/// result's seams, whose normals the CPU's cut makes anew.
#[test]
fn a_cutter_dragged_with_the_gizmo_draws_its_hole_through_the_peel_until_its_run_lands() {
    let mut w = pair("difference", 0.8, OVER);
    let [_, _, _, was] = pictures(&mut w);
    let down = json!({ "part": "cutter", "op": { "move": [-0.5, -0.4, 0.0] } });
    w.ok(
        "gizmo.drag",
        json!({ "phase": "update", "transform": down }),
    );
    let peeled = Picture::of(&mut w);
    w.ok("gizmo.drag", json!({ "phase": "end" }));
    let _ = w.engine.tick();
    let landed = Picture::of(&mut w);
    let (moved, kept) = (peeled.differ(&was), peeled.differ(&landed));
    assert!(
        moved > 4000,
        "the peel draws the hole where the cutter is now: {moved}"
    );
    assert!(kept < 1700, "as the result draws it once it lands: {kept}");
}
