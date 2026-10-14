//! Oracle self-tests (ARCHITECTURE 10.1 kind 4): every test-only oracle in `common` rejects
//! the known-bad input it exists to catch, so a broken oracle cannot pass broken output. (The
//! Hausdorff distance, volume and manifold check come from gs-testkit, which tests its own.)

use crate::common::*;

#[test]
fn components_counts_the_pieces() {
    let ball = sphere([0.0; 3], 1.0, 4);
    assert_eq!(components(&ball), 1, "one piece");
    let two = merge(&ball, &cuboid([3.0; 3], [4.0; 3], 1));
    assert_eq!(components(&two), 2, "two pieces");
    let three = merge(&two, &torus(1.0, 0.3, 12, 8));
    assert_eq!(components(&three), 3, "three pieces");
}

#[test]
fn triangle_distance_measures_the_face_the_edges_and_the_corners() {
    let t = [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]];
    let cases = [
        ([0.5, 0.5, 3.0], 3.0),
        ([0.5, 0.5, -0.25], 0.25),
        ([1.0, -1.0, 0.0], 1.0),
        ([-3.0, -4.0, 0.0], 5.0),
        ([2.0, 2.0, 0.0], 2.0_f64.sqrt()),
        ([4.0, 0.0, 2.0], 8.0_f64.sqrt()),
    ];
    for (p, d) in cases {
        let got = triangle_distance(p, &t);
        assert!((got - d).abs() <= 1e-12, "{p:?}: {got} vs {d}");
    }
}

#[test]
fn the_nearest_query_agrees_with_checking_every_triangle() {
    // Points on, near and up to a little under 4 buckets outside a sphere: the fast query
    // must match brute force exactly, however many rings out the answer lies.
    let tris = triangles(&sphere([0.1, -0.2, 0.3], 1.0, 6));
    let brute = |p: P| {
        tris.iter()
            .map(|t| triangle_distance(p, t))
            .fold(f64::MAX, f64::min)
    };
    let near = Nearest::new(tris.clone(), 0.25);
    for i in 0..300 {
        let k = f64::from(i);
        let p = [
            1.5 * libm::sin(k * 0.37),
            1.5 * libm::cos(k * 0.61),
            0.9 * libm::sin(k * 0.23) - 0.2,
        ];
        let (fast, slow) = (near.distance(p), brute(p));
        assert!((fast - slow).abs() <= 1e-12, "{p:?}: {fast} against {slow}");
    }
    // Far away it gives a lower bound that still shows the point is far.
    for p in [[10.0, 0.0, 0.0], [0.0, -7.0, 3.0]] {
        let (fast, slow) = (near.distance(p), brute(p));
        assert!(
            fast <= slow && fast >= 0.5 * slow,
            "{p:?}: {fast} against {slow}"
        );
    }
    assert!(
        near.distance([f64::NAN, 0.0, 0.0]).is_infinite(),
        "not a point"
    );
}

#[test]
fn the_winding_number_is_one_inside_zero_outside_and_half_on_a_hole() {
    let s = triangles(&sphere([0.0; 3], 1.0, 6));
    assert!(
        (winding(&s, [0.1, -0.2, 0.3]) - 1.0).abs() <= 1e-9,
        "inside"
    );
    assert!(winding(&s, [2.0, 0.0, 0.5]).abs() <= 1e-9, "outside");
    let inverted = triangles(&flipped(&sphere([0.0; 3], 1.0, 6)));
    assert!(
        (winding(&inverted, [0.0; 3]) + 1.0).abs() <= 1e-9,
        "inside out"
    );
    // A flat square seen from just above its middle covers almost half the sphere of view.
    let square = triangles(&without(&cuboid([-1.0; 3], [1.0; 3], 1), |c| {
        !c.iter().all(|p| p[2] >= 1.0 - 1e-9)
    }));
    let w = winding(&square, [0.0, 0.0, 0.999]);
    assert!(w.abs() >= 0.45 && w.abs() <= 0.5, "half: {w}");
}

#[test]
fn the_fast_hausdorff_distance_gives_gs_testkits_answer() {
    let a = sphere([0.1, -0.2, 0.3], 1.0, 5);
    let mut b = sphere([0.0; 3], 0.9, 4);
    b.positions[7] = [1.4, 0.2, -0.1];
    let open = without(&a, |c| c.iter().all(|p| p[1] >= 0.4));
    for (from, to) in [(&a, &b), (&b, &a), (&open, &b), (&b, &open)] {
        let fast = fast_hausdorff_from(from, to, 0.1);
        let slow = gs_testkit::measure::hausdorff_from(from, to);
        assert!((fast - slow).abs() <= 1e-12, "{fast} against {slow}");
    }
    let both = fast_hausdorff(&a, &b, 0.1);
    let slow = gs_testkit::measure::hausdorff(&a, &b);
    assert!(
        (both - slow).abs() <= 1e-12,
        "two-sided: {both} against {slow}"
    );
    assert!(
        both >= 0.3,
        "non-trivial: the moved vertex is far out ({both})"
    );
}
