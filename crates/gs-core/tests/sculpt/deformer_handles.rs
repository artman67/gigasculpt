//! The open deformer's handles (decision 0041, amended 2026-10-13 for `DF7c`): a cone for each
//! argument the cone table names, at the middle of the box's face on the camera's side, a ring
//! for a twist, a dot at each cage point and each curve point, and the box repeated where a
//! splitting symmetry plane carries the deformer. One pure function makes them all.

use gs_core::camera::{Camera, Preset};
use gs_core::deform::{Curve, Deform, Lattice, Offset};
use gs_core::gizmo::{
    Deformer, DeformerHandle, DeformerHandles, Drive, Kind, Style, deformer_handles, pick_deformer,
};
use gs_core::symmetry::Symmetry;
use gs_core::transform::{Frame, Op, Similarity};
use gs_core::{Aabb, Quat, Vec3};

const SIZE: [f32; 2] = [1000.0, 800.0];

/// The box every test deforms: two long on X, one on Y, half on Z, about the frame's origin.
const BOX: Aabb = Aabb {
    lo: Vec3::new(-1.0, -0.5, -0.25),
    hi: Vec3::new(1.0, 0.5, 0.25),
};

/// A camera ten units from the origin, `azimuth` and `elevation` degrees round from the front.
fn camera(azimuth: f32, elevation: f32) -> Camera {
    Camera {
        orientation: Camera::turned(azimuth.to_radians(), elevation.to_radians()),
        distance: 10.0,
        ..Camera::preset(Preset::Front)
    }
}

/// From the front, right and above: every axis's high face is toward it.
fn high() -> Camera {
    camera(30.0, 25.0)
}

/// From behind, left and below: every axis's low face is toward it.
fn low() -> Camera {
    camera(-150.0, -25.0)
}

fn frame(origin: Vec3) -> Frame {
    Frame {
        origin,
        axes: Quat::IDENTITY,
    }
}

/// The handles of `op` about an unturned frame at `origin` on [`BOX`], seen by `camera`.
fn shown(op: &Op, origin: Vec3, symmetry: &Symmetry, camera: &Camera) -> DeformerHandles {
    let placed = Similarity::default();
    let deformer = Deformer {
        op,
        bounds: BOX,
        symmetry,
        placed: &placed,
    };
    deformer_handles(&frame(origin), &deformer, (camera, SIZE, &Style::default()))
}

fn plain(op: &Op, camera: &Camera) -> DeformerHandles {
    shown(op, Vec3::ZERO, &Symmetry::default(), camera)
}

fn near(a: Vec3, b: Vec3) -> bool {
    (a - b).len2().sqrt() < 1e-5
}

fn names(d: &DeformerHandles) -> Vec<String> {
    d.handles.iter().map(DeformerHandle::name).collect()
}

fn pointer(h: &DeformerHandle) -> &'static str {
    h.drive.at()
}

/// The number a handle drives and its rate.
fn rate(h: &DeformerHandle) -> f32 {
    match h.drive {
        Drive::Number { rate, .. } => rate,
        _ => panic!("{} drives no number", h.name()),
    }
}

#[test]
fn a_cone_per_argument_sits_on_the_middle_of_the_face_toward_the_camera() {
    let x = Vec3::new(1.0, 0.0, 0.0);
    let (y, z) = (Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
    for (camera, side) in [(high(), 1.0), (low(), -1.0)] {
        let offset = plain(&Op::Move(Vec3::ZERO), &camera);
        assert_eq!(names(&offset), ["cone_x", "cone_y", "cone_z"]);
        let faces = [x * 1.0, y * 0.5, z * 0.25];
        for (h, face) in offset.handles.iter().zip(faces) {
            assert_eq!(h.kind, Kind::Cone);
            assert_eq!(pointer(h), "/move");
            assert!(near(h.at, face * side), "{}: {:?}", h.name(), h.at);
            assert!(near(h.along, face.normalized() * side), "outward");
            assert!(h.pickable && h.fade > 0.0);
            // The tip 0.3 gizmo units out, the base 0.16 wide, as drawn and picked.
            let tip = h.at + h.along * (0.3 * offset.unit);
            let drawn = camera.project(tip.to_array(), SIZE).unwrap_or_default();
            let [dx, dy] = [h.points[0][0] - drawn[0], h.points[0][1] - drawn[1]];
            assert!(
                libm::hypotf(dx, dy) < 0.01,
                "{:?} against {drawn:?}",
                h.points
            );
            let [a, b] = [h.points[1], h.points[2]];
            let width = libm::hypotf(b[0] - a[0], b[1] - a[1]);
            assert!((width - 0.16 * offset.unit_px).abs() < 0.01, "{width}");
            let base = [f32::midpoint(a[0], b[0]), f32::midpoint(a[1], b[1])];
            let radius = Style::default().pick_pt;
            let picked = pick_deformer(&offset.handles, base, radius);
            assert_eq!(picked.map(|i| offset.handles[i].name()), Some(h.name()));
        }

        // A bend's one cone: on its axis's face, toward Toward; its arc's end follows the pen,
        // 2 span / L squared radians a scene unit, in degrees.
        let bend = Deform::Bend {
            axis: 0,
            toward: 1,
            angle: 0.0,
        };
        let bend = plain(&Op::Deform(bend), &camera);
        assert_eq!(names(&bend), ["cone_x"]);
        let h = &bend.handles[0];
        assert_eq!(pointer(h), "/bend/degrees");
        assert!(near(h.at, x * side) && near(h.along, y));
        assert!((rate(h) - 4f32.to_degrees()).abs() < 1e-3, "{}", rate(h));

        // A fold's turns half on each side: 2 / f radians a scene unit.
        let fold = Deform::Fold {
            axis: 1,
            toward: 2,
            angle: 0.0,
        };
        let fold = plain(&Op::Deform(fold), &camera);
        assert_eq!(names(&fold), ["cone_y"]);
        assert!(near(fold.handles[0].along, z));
        assert!((rate(&fold.handles[0]) - 4f32.to_degrees()).abs() < 1e-3);

        // A turn about X: one cone on the Y or Z face, along the turn.
        let rotate = Op::Rotate {
            axis: x,
            angle: 0.0,
        };
        let rotate = plain(&rotate, &camera);
        assert_eq!(rotate.handles.len(), 1, "{:?}", names(&rotate));
        let h = &rotate.handles[0];
        assert_eq!(pointer(h), "/rotate/degrees");
        let f: f32 = h.at.len2().sqrt();
        assert!(near(h.along, x.cross(h.at).normalized()), "along the turn");
        assert!((rate(h) - (1.0 / f).to_degrees()).abs() < 1e-3);

        // A taper's cone: on the high face's edge, pointing in.
        let taper = Deform::Taper {
            axis: 0,
            amount: 0.0,
            exponent: 1.0,
        };
        let taper = shown(&Op::Deform(taper), x * -1.0, &Symmetry::default(), &camera);
        let h = &taper.handles[0];
        assert_eq!((h.name().as_str(), pointer(h)), ("cone_x", "/taper/amount"));
        assert!(
            (h.at.x - 0.0).abs() < 1e-5,
            "the high face, about a frame at the low"
        );
        assert!(
            h.along.dot(h.at - Vec3::new(h.at.x, 0.0, 0.0)) < 0.0,
            "inward"
        );

        // Three for the stretch, each driving its own factor.
        let stretch = plain(&Op::Scale([1.0; 3]), &camera);
        let at: Vec<&str> = stretch.handles.iter().map(pointer).collect();
        assert_eq!(at, ["/scale/0", "/scale/1", "/scale/2"]);
        assert!((rate(&stretch.handles[0]) - 1.0).abs() < 1e-5, "1 over f");
    }
}

#[test]
fn a_twist_has_a_ring_about_its_axis_on_the_face_toward_the_camera() {
    let twist = Deform::Twist {
        axis: 1,
        angle: 0.0,
    };
    let camera = high();
    let d = plain(&Op::Deform(twist), &camera);
    assert_eq!(names(&d), ["ring_y"]);
    let h = &d.handles[0];
    assert_eq!((h.kind, pointer(h)), (Kind::Ring, "/twist/degrees"));
    let centre = Vec3::new(0.0, 0.5, 0.0);
    assert!(near(h.at, centre), "{:?}", h.at);
    // The face at its place 0.5 along the box turns half the angle: 2 degrees a degree swept.
    assert!((rate(h) - 2.0).abs() < 1e-5, "{}", rate(h));
    // A closed circle through the farthest corner's distance from the axis, starting on Z.
    assert_eq!(h.points.len(), 65);
    assert_eq!(h.points[0], h.points[64]);
    let r = (1.0f32 + 0.25 * 0.25).sqrt();
    let start = centre + Vec3::new(0.0, 0.0, r);
    let drawn = camera.project(start.to_array(), SIZE).unwrap_or_default();
    let [dx, dy] = [h.points[0][0] - drawn[0], h.points[0][1] - drawn[1]];
    assert!(
        libm::hypotf(dx, dy) < 0.01,
        "{:?} against {drawn:?}",
        h.points[0]
    );
}

#[test]
fn a_three_cubed_cage_has_27_dots_at_rest_plus_offset_and_its_lines() {
    let lattice = Lattice {
        soft: true,
        size: [3, 3, 3],
        offsets: vec![Offset {
            at: [2, 1, 0],
            by: [0.1, 0.0, 0.0],
        }],
    };
    let d = plain(&Op::Deform(Deform::Lattice(lattice)), &high());
    assert_eq!(d.handles.len(), 27);
    assert!(d.handles.iter().all(|h| h.kind == Kind::Dot));
    assert!(d.handles.iter().all(|h| pointer(h) == "/lattice/offsets"));
    let dot = |name: &str| {
        (d.handles.iter())
            .find(|h| h.name() == name)
            .unwrap_or_else(|| panic!("no {name} in {:?}", names(&d)))
    };
    assert!(near(dot("dot_0_0_0").at, BOX.lo));
    assert!(near(dot("dot_1_1_1").at, Vec3::ZERO));
    let moved = dot("dot_2_1_0");
    assert!(near(moved.at, Vec3::new(1.1, 0.0, -0.25)), "{:?}", moved.at);
    assert_eq!(
        moved.drive,
        Drive::Cage {
            at: [2, 1, 0],
            by: [0.1, 0.0, 0.0]
        }
    );
    // A line through each row of three along each axis.
    assert_eq!(d.lines.iter().filter(|l| !l.repeat).count(), 12 + 27);
    assert_eq!(d.lines.iter().filter(|l| l.points.len() == 3).count(), 27);
}

#[test]
fn a_curve_has_a_dot_at_each_point_or_at_the_ends_of_its_axis_line() {
    let curve = |points: Option<Vec<[f32; 3]>>| {
        let c = Curve {
            axis: 0,
            points,
            twist: None,
            scale: None,
        };
        plain(&Op::Deform(Deform::Curve(c)), &high())
    };
    let line = curve(None);
    assert_eq!(names(&line), ["point_0", "point_1"]);
    assert!(near(line.handles[0].at, Vec3::new(-1.0, 0.0, 0.0)));
    assert!(near(line.handles[1].at, Vec3::new(1.0, 0.0, 0.0)));
    assert_eq!(pointer(&line.handles[1]), "/curve/points");

    let points = vec![[-1.0, 0.0, 0.0], [0.0, 0.3, 0.0], [1.0, 0.0, 0.1]];
    let bent = curve(Some(points.clone()));
    assert_eq!(names(&bent), ["point_0", "point_1", "point_2"]);
    for (h, p) in bent.handles.iter().zip(&points) {
        assert!(near(h.at, Vec3::from_array(*p)));
    }
    let joined = bent.lines.iter().filter(|l| l.points.len() == 3).count();
    assert_eq!(joined, 1, "one line through the points in order");
}

#[test]
fn a_splitting_mirror_or_parallel_plane_repeats_the_box_without_handles() {
    let bend = || {
        Op::Deform(Deform::Bend {
            axis: 1,
            toward: 0,
            angle: 0.0,
        })
    };
    let camera = high();
    let off = Vec3::new(2.0, 0.0, 0.0);
    let none = shown(&bend(), off, &Symmetry::default(), &camera);
    assert_eq!(none.boxes.len(), 1);

    let mirror = Symmetry {
        mirror: [true, false, false],
        ..Symmetry::default()
    };
    let mirrored = shown(&bend(), off, &mirror, &camera);
    assert_eq!(mirrored.boxes.len(), 2, "the frame is off the X plane");
    assert_eq!(names(&mirrored), names(&none), "handles on the box itself");
    for (a, b) in mirrored.boxes[0].iter().zip(&mirrored.boxes[1]) {
        let flipped = Vec3::new(-a.x, a.y, a.z);
        assert!(mirrored.boxes[1].iter().any(|c| near(*c, flipped)), "{b:?}");
    }
    let repeats = mirrored.lines.iter().filter(|l| l.repeat).count();
    assert_eq!(repeats, 12, "the repeat's twelve edges");

    let parallel = Symmetry {
        parallel: [true, false, false],
        ..Symmetry::default()
    };
    let moved = shown(&bend(), off, &parallel, &camera);
    for (a, b) in moved.boxes[0].iter().zip(&moved.boxes[1]) {
        assert!(near(*b, *a - Vec3::new(4.0, 0.0, 0.0)), "{a:?} {b:?}");
    }

    let on_plane = shown(&bend(), Vec3::ZERO, &mirror, &camera);
    assert_eq!(
        on_plane.boxes.len(),
        1,
        "a plane through the frame splits nothing"
    );
}
