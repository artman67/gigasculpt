//! Catalog `brush-settings.depth`, its gs-core Check clauses (decision 0053): Outer Depth 0
//! leaves a bump beside a hollow alone, gravity pulls straight down whatever the normal, and
//! Infinite Depth reaches points ten radii behind, along the view or along a model axis. The
//! Move clause, through a camera, is gs-engine's `sculpt::depth`.

use gs_core::{BrushPreset, StrokeParams, Vec3, View};
use gs_protocol::PlainMesh;

use crate::automask_checks::plus;
use crate::brush_kit::{after, at, grid, ico, params, preset, store};
use crate::stroke_masks::moved;

/// Why the shipped preset `stem` with `extra` appended is refused.
fn refused(stem: &str, extra: &str) -> String {
    match BrushPreset::from_toml(&format!("{}\n{extra}\n", preset(stem).text)) {
        Ok(_) => panic!("{stem} with {extra:?} was not refused"),
        Err(e) => e.message,
    }
}

fn looking(dir: Vec3) -> StrokeParams {
    let mut p = params(None);
    p.view = View { dir, eye: None };
    p
}

/// A smooth bell, 1 at `t = 0` and 0 from `t = 1`.
fn bell(t: f32) -> f32 {
    let u = (1.0 - t * t).max(0.0);
    u * u
}

/// Where the hollow and the bump sit on the plane, and how deep and high they are.
const HOLLOW: [f32; 2] = [-0.15, 0.0];
const BUMP: [f32; 2] = [0.15, 0.0];
const RISE: f32 = 0.1;

/// A flat square of 64 by 64 quads facing +z, with a hollow 0.1 deep and, beside it, a bump
/// 0.1 high, in the store's vertex order.
fn hollow_and_bump() -> PlainMesh {
    let mut m = grid(64);
    for p in &mut m.positions {
        let d = |c: [f32; 2]| libm::hypotf(p[0] - c[0], p[1] - c[1]);
        p[2] = RISE * (bell(d(BUMP) / 0.1) - bell(d(HOLLOW) / 0.12));
    }
    store(&m).to_plain()
}

/// Whether vertex `i` of the plane is on the bump (above the flat).
fn on_bump(m: &PlainMesh, i: usize) -> bool {
    let p = m.positions[i];
    p[2] > 0.0 && p[0] > 0.0
}

/// Whether vertex `i` of the plane is in the hollow (below the flat).
fn in_hollow(m: &PlainMesh, i: usize) -> bool {
    m.positions[i][2] < 0.0
}

/// One Draw tap of radius 0.45 at the bottom of the hollow (or, with `hollow` false, the top
/// of the bump) with `extra` appended to Draw.
fn tap(mesh: &PlainMesh, hollow: bool, extra: &str) -> PlainMesh {
    let (on, z) = if hollow {
        (HOLLOW, -RISE)
    } else {
        (BUMP, RISE)
    };
    let dab = [at(Vec3::new(on[0], on[1], z), 0.45)];
    after(mesh, &plus("draw", extra), params(None), &dab)
}

/// "Outer Depth 0 leaves a raised bump untouched when the stroke starts in a hollow": Draw in
/// the hollow, its radius reaching well over the bump, raises the hollow's floor; with the
/// Depth Mask and Outer Depth 0 not one point of the bump moves, and without it the bump does.
#[test]
fn outer_depth_zero_leaves_a_bump_beside_a_hollow_untouched() {
    let mesh = hollow_and_bump();
    let plain = tap(&mesh, true, "");
    let masked = tap(&mesh, true, "[depth]\nmask = true\nouter = 0.0");
    let n = mesh.positions.len();
    let bump: Vec<usize> = (0..n).filter(|&i| on_bump(&mesh, i)).collect();
    let most =
        |m: &PlainMesh, set: &[usize]| set.iter().map(|&i| moved(&mesh, m, i)).fold(0.0, f32::max);
    assert!(bump.len() > 30, "the bump has {} points", bump.len());
    assert!(
        most(&plain, &bump) > 1e-3,
        "without the mask the tap reaches the bump"
    );
    for &i in &bump {
        assert_eq!(
            masked.positions[i], mesh.positions[i],
            "bump point {i} moved under Outer Depth 0"
        );
    }
    let hollow: Vec<usize> = (0..n).filter(|&i| in_hollow(&mesh, i)).collect();
    let filled = most(&masked, &hollow);
    assert!(filled > 1e-3, "the hollow's floor rises ({filled})");
}

/// Inner Depth 0 is the other side of the band: a tap on the bump's top with it leaves the
/// hollow beside it alone, and without the mask moves it.
#[test]
fn inner_depth_zero_leaves_a_hollow_beside_a_bump_untouched() {
    let mesh = hollow_and_bump();
    let plain = tap(&mesh, false, "");
    let masked = tap(&mesh, false, "[depth]\nmask = true\ninner = 0.0");
    let hollow: Vec<usize> = (0..mesh.positions.len())
        .filter(|&i| in_hollow(&mesh, i))
        .collect();
    assert!(hollow.iter().any(|&i| moved(&mesh, &plain, i) > 1e-3));
    for &i in &hollow {
        assert_eq!(masked.positions[i], mesh.positions[i], "hollow point {i}");
    }
}

/// The Depth Curve fades the band from the surface point out: in a band a quarter radius each
/// way, a curve falling from 1 to 0 moves no point more than the flat curve does, keeps most
/// of the move of points near the surface point and halves it far from it along the normal.
#[test]
fn the_depth_curve_fades_the_band_toward_its_edges() {
    let mesh = hollow_and_bump();
    let band = "[depth]\nmask = true\ninner = -0.25\nouter = 0.25";
    let flat = tap(&mesh, true, band);
    let fade = tap(&mesh, true, &format!("{band}\ncurve = [[0, 1], [1, 0]]"));
    let (mut kept, mut halved) = (0, 0);
    for i in 0..mesh.positions.len() {
        let (a, b) = (moved(&mesh, &flat, i), moved(&mesh, &fade, i));
        assert!(b <= a, "point {i} moved {b} with the fade, {a} without");
        kept += usize::from(a > 1e-4 && b > 0.9 * a);
        halved += usize::from(a > 1e-4 && b < 0.5 * a);
    }
    assert!(
        kept > 3 && halved > 10,
        "the fade keeps 90% of the move at {kept} points and halves it at {halved}"
    );
}

/// "Gravity pulls added material down whatever the normal": one Draw tap with gravity on, on
/// faces whose normals point up, sideways and down, differs from the same tap without it by a
/// pull straight down (the preset's direction, -y) at every point it moved.
#[test]
fn gravity_pulls_straight_down_whatever_the_normal() {
    let mesh = ico(4);
    let with = plus("draw", "[post]\ngravity = 1.0");
    let without = preset("draw").preset;
    for (name, normal) in [
        ("up", Vec3::new(0.0, 1.0, 0.0)),
        ("sideways", Vec3::new(1.0, 0.0, 0.0)),
        ("down", Vec3::new(0.0, -1.0, 0.0)),
    ] {
        let dab = [at(normal, 0.4)];
        let view = looking(-normal);
        let a = after(&mesh, &with, view.clone(), &dab);
        let b = after(&mesh, &without, view, &dab);
        let mut most = 0.0f32;
        for i in 0..mesh.positions.len() {
            let pull = Vec3::from_array(a.positions[i]) - Vec3::from_array(b.positions[i]);
            let size = pull.len2().sqrt();
            if size == 0.0 {
                continue;
            }
            let [x, y, z] = pull.to_array();
            assert!(
                y < 0.0 && x.abs() <= 1e-3 * size + 1e-6 && z.abs() <= 1e-3 * size + 1e-6,
                "{name}: point {i} pulled by {pull:?}, not down"
            );
            most = most.max(size);
        }
        assert!(most > 0.01, "{name}: the pull is {most}");
    }
}

/// The brush radius of the Infinite Depth taps, and how far behind the front the back sheet
/// lies: ten radii.
const R: f32 = 0.2;
const BEHIND: f32 = 10.0 * R;

/// Two flat squares of 64 by 64 quads facing +z: the front at z = 0 and the back ten radii
/// behind it.
fn two_sheets() -> PlainMesh {
    let front = grid(64);
    let mut m = front.clone();
    let base = front.positions.len() as u32;
    m.positions
        .extend(front.positions.iter().map(|p| [p[0], p[1], -BEHIND]));
    m.faces
        .extend(front.faces.iter().map(|f| f.map(|v| v + base)));
    store(&m).to_plain()
}

/// The back sheet's points that a Draw tap at the front's center, seen along `view`, with
/// `extra` appended to Draw, moves; and those it leaves.
fn behind(view: Vec3, extra: &str) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mesh = two_sheets();
    let dab = [at(Vec3::ZERO, R)];
    let out = after(&mesh, &plus("draw", extra), looking(view), &dab);
    let back = (0..mesh.positions.len()).filter(|&i| mesh.positions[i][2] < 0.0);
    let (m, l): (Vec<usize>, Vec<usize>) = back.partition(|&i| moved(&mesh, &out, i) > 0.0);
    let points = |v: Vec<usize>| v.into_iter().map(|i| mesh.positions[i]).collect();
    (points(m), points(l))
}

/// Distance across z from the point `(x, y)`.
fn across_z(p: [f32; 3], x: f32, y: f32) -> f32 {
    libm::hypotf(p[0] - x, p[1] - y)
}

/// "Infinite Depth moves vertices 10 radii behind": a tap on the front sheet moves the back
/// sheet's points inside the circle behind it and no others; with Infinite Depth off it moves
/// none of them.
#[test]
fn infinite_depth_moves_points_ten_radii_behind() {
    let front = Vec3::new(0.0, 0.0, -1.0);
    let (none, _) = behind(front, "");
    assert!(
        none.is_empty(),
        "{} back points moved without it",
        none.len()
    );
    let (moved, left) = behind(front, "[depth]\ninfinite = true");
    assert!(
        left.iter().all(|&p| across_z(p, 0.0, 0.0) >= 0.5 * R),
        "a back point near the axis stayed"
    );
    assert!(
        moved.len() > 20 && moved.iter().all(|&p| across_z(p, 0.0, 0.0) < 1.001 * R),
        "{} back points moved, all inside the circle",
        moved.len()
    );
}

/// With the X, Y, Z switches off the reach runs along the view, so a tilted view reaches the
/// back sheet off to the side; with Z on it runs along the model's z whatever the view.
#[test]
fn infinite_depth_runs_along_the_view_or_along_a_model_axis() {
    let tilted = Vec3::new(0.3, 0.0, -1.0).normalized();
    let off = 0.3 * BEHIND;
    let (moved, _) = behind(tilted, "[depth]\ninfinite = true");
    assert!(!moved.is_empty() && moved.iter().all(|&p| across_z(p, off, 0.0) < 1.1 * R));
    let (moved, left) = behind(
        tilted,
        "[depth]\ninfinite = true\naxes = [false, false, true]",
    );
    assert!(moved.len() > 20 && moved.iter().all(|&p| across_z(p, 0.0, 0.0) < 1.001 * R));
    assert!(left.iter().all(|&p| across_z(p, 0.0, 0.0) >= 0.5 * R));
}

/// The settings' ranges, a Depth Mask kept while it is off, and the combinations the depth
/// settings cannot run with (decision 0053, sections 1, 4 and 7).
#[test]
fn depth_settings_are_checked() {
    for (extra, key) in [
        ("[depth]\ninner = 0.5", "depth.inner"),
        ("[depth]\ninner = -1.5", "depth.inner"),
        ("[depth]\nouter = -0.1", "depth.outer"),
        ("[depth]\nouter = 1.5", "depth.outer"),
        ("[depth]\ncurve = [[0, 1], [1, 2]]", "depth.curve"),
    ] {
        let why = refused("draw", extra);
        assert!(why.contains(key), "{extra}: {why}");
    }
    let kept = plus(
        "draw",
        "[depth]\nouter = 0.0\ninfinite = false\naxes = [true, false, false]",
    );
    for (stem, extra) in [
        ("clip-curve", "[depth]\nmask = true"),
        ("clip-curve", "[depth]\ninfinite = true"),
        ("move-connected", "[depth]\ninfinite = true"),
        ("mask-pen", "[depth]\ninfinite = true"),
        ("paint-draw", "[depth]\ninfinite = true"),
    ] {
        let why = refused(stem, extra);
        assert!(why.contains("depth."), "{stem} with {extra:?}: {why}");
    }
    let elastic = (preset("move").text).replace("select = \"ball\"", "select = \"elastic\"");
    match BrushPreset::from_toml(&format!("{elastic}\n[depth]\ninfinite = true\n")) {
        Ok(_) => panic!("Move's elastic select took Infinite Depth"),
        Err(e) => assert!(e.message.contains("depth.infinite"), "{}", e.message),
    }
    // The shipped Move Elastic is a ball Move with a relax pass: it takes Infinite Depth.
    plus("move-elastic", "[depth]\ninfinite = true");
    // Every shipped preset writes none of them, so no preset's hash changed.
    let text = kept.canonical();
    assert!(
        text.contains("outer = 0") && text.contains("axes"),
        "{text}"
    );
    let text = preset("move").preset.canonical();
    let new = ["\nmask", "inner", "outer", "curve", "infinite", "axes"];
    assert!(new.iter().all(|k| !text.contains(k)), "{text}");
}
