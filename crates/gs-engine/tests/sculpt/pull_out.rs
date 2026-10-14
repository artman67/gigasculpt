//! Pull Out's Checks through `brush.stroke` and a camera (catalog `brush.pull-out`, decision
//! 0049, section 7). A plane is seen tilted, 40 degrees off its normal, and a drag runs from the
//! middle of the view straight down the screen, the way the plane's normal points on it: Pull
//! Out pulls parallel to the view plane, Pull Out Along Normal out along the normal where the
//! stroke started, the tip follows the pointer either way, a pull of ten radii is a roughly
//! cylindrical extrusion, and the Brush Modifier blends one direction into the other.

use gs_engine::EngineConfig;
use gs_protocol::{PlainMesh, Role};
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::with;

/// The brush radius, in scene units.
const R: f64 = 0.08;

/// A flat square of 128 by 128 quads on z = 0 from -1 to 1, facing +z.
fn plane() -> PlainMesh {
    let n = 128u32;
    let c = |k: u32| (f64::from(k) * 2.0 / f64::from(n) - 1.0) as f32;
    let at = |i: u32, j: u32| i * (n + 1) + j;
    let mut m = PlainMesh::default();
    for i in 0..=n {
        for j in 0..=n {
            m.positions.push([c(i), c(j), 0.0]);
        }
    }
    for i in 0..n {
        for j in 0..n {
            m.faces
                .push([at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    m
}

/// A camera 4 units from the origin, 40 degrees above the plane's front: the middle pixel shows
/// the origin, and the plane's normal points straight down the screen.
fn camera() -> Value {
    json!({ "azimuth_deg": 0.0, "elevation_deg": 40.0, "target": [0.0, 0.0, 0.0], "distance": 4.0 })
}

/// The drag: 51 samples from the middle of the view 300 pixels straight down, about twelve
/// radii across the screen at the plane's depth.
fn drag() -> Vec<[f32; 2]> {
    (0..=50).map(|i| [512.0, 512.0 + 6.0 * i as f32]).collect()
}

/// A session holding the plane, and the plane as the session holds it.
fn session() -> (Session, PlainMesh) {
    let mut s = Session::with(EngineConfig::default(), Role::Agent);
    s.add_part("plane", &plane());
    let before = s.mesh("plane");
    (s, before)
}

/// The plane after the drag with `brush` and the arguments `extra`, in `s`.
fn pull(s: &mut Session, brush: &str, extra: &Value) -> PlainMesh {
    let samples: Vec<Value> = drag().iter().map(|p| json!({ "position": p })).collect();
    let args = json!({ "brush": brush, "camera": camera(), "size": R, "samples": samples });
    let done = s.ok("brush.stroke", with(args, extra));
    assert_eq!(
        done.result["samples_missed"],
        json!(0),
        "{brush}: {}",
        done.result
    );
    s.mesh("plane")
}

type V = [f64; 3];

fn sub(a: V, b: V) -> V {
    std::array::from_fn(|k| a[k] - b[k])
}

fn dot(a: V, b: V) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}

fn len(a: V) -> f64 {
    dot(a, a).sqrt()
}

fn wide(p: [f32; 3]) -> V {
    p.map(f64::from)
}

/// The ray through the pixel `at`: its origin and direction.
fn ray(s: &mut Session, at: [f32; 2]) -> (V, V) {
    let r = s.query("pick.ray", json!({ "camera": camera(), "at": at }));
    let v = |k: &str| std::array::from_fn(|i| r[k][i].as_f64().unwrap_or(f64::NAN));
    (v("origin"), v("direction"))
}

/// Where the ray through the drag's last pixel meets the plane through the origin (where the
/// drag grabbed) with normal `n`: where the pointer leaves the tip.
fn pointer(s: &mut Session, n: V) -> V {
    let (o, d) = ray(s, drag()[50]);
    let t = -dot(o, n) / dot(d, n);
    std::array::from_fn(|k| o[k] + d[k] * t)
}

/// The view direction at the middle of the view, and the normal of the plane holding the
/// plane's normal +z that faces the camera most (decision 0049, section 7, at a blend of 1).
fn planes(s: &mut Session) -> (V, V) {
    let (_, view) = ray(s, [512.0, 512.0]);
    let n = [0.0, 0.0, 1.0];
    let u = sub(view, n.map(|x| x * dot(n, view)));
    (view, u.map(|x| x / len(u)))
}

/// The vertex that moved most: where it is now, and how far it moved.
fn tip(before: &PlainMesh, after: &PlainMesh) -> (V, f64) {
    let moved = |i: &usize| len(sub(wide(after.positions[*i]), wide(before.positions[*i])));
    let i = (0..before.positions.len())
        .max_by(|a, b| moved(a).total_cmp(&moved(b)))
        .unwrap_or_default();
    (wide(after.positions[i]), moved(&i))
}

/// The Check, screen-plane mode: from the tilted plane Pull Out pulls parallel to the view
/// plane, every point moving across the view and none along it, and the tip ends at the
/// pointer, ten radii and more from where it grabbed.
#[test]
fn pull_out_moves_parallel_to_the_view_plane_and_its_tip_follows_the_pointer() {
    let (mut s, before) = session();
    let after = pull(&mut s, "Pull Out", &json!({}));
    let (view, _) = planes(&mut s);
    let want = pointer(&mut s, view);
    let (end, far) = tip(&before, &after);
    let deepest = (before.positions.iter().zip(&after.positions))
        .map(|(a, b)| dot(sub(wide(*b), wide(*a)), view).abs())
        .fold(0.0, f64::max);
    let miss = len(sub(end, want));
    eprintln!(
        "Pull Out: the tip moved {far}, ends {miss} from the pointer; the most any point moved \
         along the view is {deepest}"
    );
    assert!(far > 10.0 * R, "pulled {far}");
    assert!(miss < R, "the tip ends {miss} from the pointer");
    assert!(deepest < 1e-4, "a point moved {deepest} along the view");
}

/// The Check, surface-normal mode: from the same tilted plane Pull Out Along Normal pulls every
/// point that moves more than a radius within 5 degrees of the plane's normal, and the tip
/// ends at the pointer.
#[test]
fn pull_out_along_normal_pulls_within_five_degrees_of_the_start_normal() {
    let (mut s, before) = session();
    let after = pull(&mut s, "Pull Out Along Normal", &json!({}));
    let (_, held) = planes(&mut s);
    let want = pointer(&mut s, held);
    let (end, far) = tip(&before, &after);
    let mut worst = 0.0f64;
    for (a, b) in before.positions.iter().zip(&after.positions) {
        let d = sub(wide(*b), wide(*a));
        if len(d) > R {
            worst = worst.max(libm::acos((d[2] / len(d)).clamp(-1.0, 1.0)).to_degrees());
        }
    }
    let miss = len(sub(end, want));
    eprintln!(
        "Pull Out Along Normal: the tip moved {far}, ends {miss} from the pointer; the points \
         that moved over a radius moved up to {worst} degrees off the normal"
    );
    assert!(far > 10.0 * R, "pulled {far}");
    assert!(miss < R, "the tip ends {miss} from the pointer");
    assert!(worst < 5.0, "{worst} degrees off the normal");
}

/// Mean distances from the extrusion's axis (from the origin, where the drag grabbed, to
/// `end`) of the points off the plane, in slabs across the axis at a quarter, half and three
/// quarters of its length, each slab a tenth of the length thick.
fn slabs(m: &PlainMesh, end: V) -> Vec<(usize, f64)> {
    let length = len(end);
    let axis = end.map(|x| x / length);
    [0.25, 0.5, 0.75]
        .map(|at| {
            let mut found = (0, 0.0);
            for p in &m.positions {
                let p = wide(*p);
                let t = dot(p, axis);
                if p[2] > R / 2.0 && (t - at * length).abs() < 0.05 * length {
                    found.0 += 1;
                    found.1 += len(sub(p, axis.map(|x| x * t)));
                }
            }
            (found.0, found.1 / found.0.max(1) as f64)
        })
        .to_vec()
}

/// The Check: pulling ten radii from a flat plane makes a roughly cylindrical extrusion. With
/// Live Detail on, as the brush works best, both directions pull a tube whose thickness at a
/// quarter, half and three quarters of its length stays within a factor of 2.5. The falloff
/// tapers it (research 10): over this drag and drags up to two pixels to either side it narrows
/// by 1.7 to 2.3 across those slabs, where a cone pulled to a point narrows by 3 (decision 0049,
/// "Measured").
#[test]
fn a_pull_of_ten_radii_is_a_roughly_cylindrical_extrusion() {
    let detail = json!({ "live_detail": { "mode": "constant", "size": 0.25 * R } });
    for brush in ["Pull Out", "Pull Out Along Normal"] {
        let (mut s, _) = session();
        let after = pull(&mut s, brush, &detail);
        // Both pull up and away from the plane, so the highest point is the tip.
        let end = (after.positions.iter().map(|p| wide(*p)))
            .max_by(|a, b| a[2].total_cmp(&b[2]))
            .unwrap_or_default();
        let found = slabs(&after, end);
        eprintln!(
            "{brush}: {} points, the tip {} from the grab; slabs (points, mean radius) {found:?}",
            after.positions.len(),
            len(end)
        );
        assert!(len(end) > 10.0 * R, "{brush}: pulled {}", len(end));
        let radii: Vec<f64> = found.iter().map(|f| f.1).collect();
        let thin = radii.iter().copied().fold(f64::INFINITY, f64::min);
        let thick = radii.iter().copied().fold(0.0, f64::max);
        assert!(
            found.iter().all(|f| f.0 >= 6),
            "{brush}: every slab holds points: {found:?}"
        );
        assert!(thick < 2.5 * thin, "{brush}: from {thin} to {thick} thick");
    }
}

/// The blend: the Brush Modifier at 100 makes Pull Out pull as Pull Out Along Normal does, and at
/// -100 the other way round, to the bit.
#[test]
fn the_brush_modifier_blends_one_direction_into_the_other() {
    let drag = |brush: &str, modifier: f64| {
        let (mut s, _) = session();
        pull(&mut s, brush, &json!({ "modifier": modifier })).positions
    };
    let (screen, normal) = (drag("Pull Out", 0.0), drag("Pull Out Along Normal", 0.0));
    assert_ne!(screen, normal, "the two directions differ");
    assert_eq!(drag("Pull Out", 100.0), normal, "Pull Out at 100");
    assert_eq!(
        drag("Pull Out Along Normal", -100.0),
        screen,
        "Along Normal at -100"
    );
}
