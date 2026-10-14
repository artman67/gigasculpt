//! A part's transform (decision 0029) maps between scene and part space consistently: out and
//! back is a round trip, a chain of two is their composition, its matrix maps as it does, and
//! the part camera sees the part exactly where the scene camera sees the scene, ray for ray.
//! The identity passes every bit through. Differential checks in `f64` (ARCHITECTURE 10.1).

use gs_core::camera::Projection;
use gs_core::transform::Similarity;
use gs_core::{Aabb, Quat, Vec3};
use gs_testkit::SplitMix64;
use gs_testkit::rng::signed_unit;

use crate::camera::random_camera;
use crate::{point, wide};

const SIZE: [f32; 2] = [640.0, 400.0];

/// A random turn, a scale from 0.25 to 4 and a move of up to 3.
fn random(rng: &mut SplitMix64) -> Similarity {
    let mut u = || signed_unit(rng.next_u64());
    let axis = Vec3::new(u(), u(), u()).normalized();
    let rotation = Quat::about(axis, 3.1 * u());
    let scale = libm::exp2f(2.0 * u());
    let translation = Vec3::new(3.0 * u(), 3.0 * u(), 3.0 * u());
    Similarity {
        translation,
        rotation,
        scale,
    }
}

/// How far apart two points are, in `f64`.
fn gap(a: Vec3, b: Vec3) -> f64 {
    let (a, b) = (wide(a), wide(b));
    (0..3)
        .map(|i| (a[i] - b[i]) * (a[i] - b[i]))
        .sum::<f64>()
        .sqrt()
}

/// The scene point of `p` through the 4 by 4 matrix, in `f64`.
fn through(m: &gs_core::camera::M4, p: Vec3) -> Vec3 {
    let q = [p.x, p.y, p.z, 1.0].map(f64::from);
    let mut out = [0.0f64; 4];
    for (column, weight) in m.iter().zip(q) {
        for (o, e) in out.iter_mut().zip(column) {
            *o += f64::from(*e) * weight;
        }
    }
    Vec3::new(out[0] as f32, out[1] as f32, out[2] as f32)
}

#[test]
fn the_identity_passes_every_bit_through() {
    let id = Similarity::IDENTITY;
    let p = Vec3::new(-0.0, 1.5e-38, -3.25);
    assert_eq!(id.point(p).bits(), p.bits());
    assert_eq!(id.part_point(p).bits(), p.bits());
    assert_eq!(id.direction(p).bits(), p.bits());
    assert_eq!(id.length(0.1).to_bits(), 0.1f32.to_bits());
    let mut rng = SplitMix64::new(3);
    let camera = random_camera(&mut rng, Projection::Perspective);
    assert_eq!(id.part_camera(&camera), camera);
    let s = random(&mut rng);
    assert_eq!(s.then(&id), s);
    assert_eq!(id.then(&s), s);
    assert!(id.is_identity() && !s.is_identity());
}

#[test]
fn out_and_back_is_a_round_trip_and_a_chain_is_the_composition() {
    let mut rng = SplitMix64::new(29);
    for _ in 0..300 {
        let (a, b) = (random(&mut rng), random(&mut rng));
        let p = point(&mut rng, 2.0);
        let scene = a.point(p);
        let size = 1.0 + gap(scene, Vec3::ZERO) + gap(p, Vec3::ZERO);
        assert!(
            gap(a.part_point(scene), p) < 2e-6 * size * 8.0,
            "{a:?} {p:?}"
        );
        assert!(gap(a.inverse().point(scene), p) < 2e-6 * size * 8.0);
        let chained = b.point(a.point(p));
        let size = size + gap(chained, Vec3::ZERO) * 4.0;
        assert!(
            gap(a.then(&b).point(p), chained) < 4e-6 * size * 8.0,
            "{a:?} {b:?}"
        );
        assert!(gap(through(&a.matrix(), p), scene) < 2e-6 * size * 8.0);
        let d = point(&mut rng, 1.0).normalized();
        assert!((gap(a.direction(d), Vec3::ZERO) - 1.0).abs() < 1e-6);
        assert!(gap(a.part_direction(a.direction(d)), d) < 1e-6);
        assert!((f64::from(a.part_length(a.length(0.7))) - 0.7).abs() < 1e-6);
    }
}

#[test]
fn a_part_box_mapped_holds_every_point_inside_it_mapped() {
    let mut rng = SplitMix64::new(41);
    for _ in 0..100 {
        let s = random(&mut rng);
        let (p, q) = (point(&mut rng, 2.0), point(&mut rng, 2.0));
        let b = Aabb {
            lo: p.min(q),
            hi: p.max(q),
        };
        let out = s.bounds(b);
        for _ in 0..20 {
            let mut u = || f32::midpoint(signed_unit(rng.next_u64()), 1.0);
            let inside = b.lo + Vec3::new(u() * (b.hi.x - b.lo.x), u() * (b.hi.y - b.lo.y), 0.0);
            let inside = Vec3::new(inside.x, inside.y, b.lo.z + u() * (b.hi.z - b.lo.z));
            let m = s.point(inside);
            let slack = 1e-5 * (1.0 + gap(m, Vec3::ZERO)) as f32;
            assert!(out.dist2(m) <= slack * slack, "{m:?} outside {out:?}");
        }
    }
}

#[test]
fn the_part_camera_sees_the_part_where_the_scene_camera_sees_the_scene() {
    let mut rng = SplitMix64::new(53);
    for k in 0..200 {
        let projection = [Projection::Perspective, Projection::Orthographic][k % 2];
        let camera = random_camera(&mut rng, projection);
        let s = random(&mut rng);
        let local = s.part_camera(&camera);
        let (x, y) = (
            640.0 * f32::midpoint(signed_unit(rng.next_u64()), 1.0),
            200.0,
        );
        let (o, d) = camera.ray(x, y, SIZE);
        let (lo, ld) = local.ray(x, y, SIZE);
        let (mo, md) = s.part_ray(o, d);
        // The two rays are one line: the same direction, and each origin on the other's line.
        assert!(gap(ld, md) < 1e-5, "{s:?} {camera:?}");
        let along = (wide(lo).iter().zip(wide(mo)).zip(wide(md)))
            .map(|((a, b), c)| (a - b) * c)
            .sum::<f64>();
        let off = gap(lo, mo + md * along as f32);
        let size = 1.0 + gap(mo, Vec3::ZERO);
        assert!(off < 1e-4 * size, "{off} {s:?}");
        let p = Vec3::from_array(camera.target) + crate::point(&mut rng, 1.0);
        let (seen, part_seen) = (
            camera.project(p.to_array(), SIZE),
            local.project(s.part_point(p).to_array(), SIZE),
        );
        match (seen, part_seen) {
            (Some(a), Some(b)) => {
                let off = libm::hypotf(a[0] - b[0], a[1] - b[1]);
                assert!(off < 0.02, "drawn {off} pixels apart");
            }
            (None, None) => {}
            (a, b) => panic!("one camera sees the point and the other does not: {a:?} {b:?}"),
        }
    }
}
