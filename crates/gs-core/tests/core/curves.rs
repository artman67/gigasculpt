//! The curve's model (catalog `interface.curve-editor`, decision 0048; the pen's sensitivities,
//! decision 0036 section 3 as amended): a point reads its value, Steps make plateaus that their
//! strength hides, Tile repeats the shape, the file form keeps every bit and old forms keep
//! their bytes, Invert turns the values over, and a sensitivity scales a curve toward full.

use gs_core::curve::{Curve, Shape};

use crate::common::ok;

#[derive(serde::Deserialize, serde::Serialize)]
struct File {
    c: Curve,
}

fn read(text: &str) -> Result<Curve, String> {
    toml::from_str::<File>(text)
        .map(|f| f.c)
        .map_err(|e| e.to_string())
}

fn write(c: &Curve) -> String {
    ok(toml::to_string(&File { c: c.clone() }))
}

fn shaped(c: Curve, steps: u32, step_strength: f32, tile: u32) -> Curve {
    ok(c.shaped(Shape {
        steps,
        step_strength,
        tile,
    }))
}

/// The curve's values at 1001 even inputs.
fn values(c: &Curve) -> Vec<f32> {
    (0..=1000).map(|k| c.eval(k as f32 / 1000.0)).collect()
}

#[test]
fn a_point_at_half_reads_its_value_there_sharp_or_smooth() {
    for smooth in [false, true] {
        let c = ok(Curve::with_kinds(
            vec![[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]],
            vec![smooth; 3],
        ));
        assert!(
            (c.eval(0.5) - 0.8).abs() < 1e-6,
            "smooth {smooth}: {}",
            c.eval(0.5)
        );
    }
}

#[test]
fn steps_make_exactly_their_count_of_plateaus_and_their_strength_hides_them() {
    for base in [
        Curve::linear(),
        ok(Curve::with_kinds(
            vec![[0.0, 0.2], [0.3, 0.5], [1.0, 0.6]],
            vec![true; 3],
        )),
    ] {
        let mut plateaus = values(&shaped(base.clone(), 4, 1.0, 1));
        plateaus.dedup();
        assert_eq!(plateaus.len(), 4, "four plateaus: {plateaus:?}");
        let ends = [base.eval(0.0), base.eval(1.0)];
        assert_eq!(
            [plateaus[0], plateaus[3]],
            ends,
            "from its start to its end"
        );
        assert_eq!(
            values(&shaped(base.clone(), 4, 0.0, 1)),
            values(&base),
            "strength 0 hides them"
        );
        let half = shaped(base.clone(), 4, 0.5, 1);
        let x = 0.1;
        let want = base.eval(x) + 0.5 * (base.eval(0.0) - base.eval(x));
        assert!((half.eval(x) - want).abs() < 1e-6, "half way at {x}");
    }
    let mut eight = values(&shaped(Curve::linear(), 8, 1.0, 1));
    eight.dedup();
    assert_eq!(eight.len(), 8);
}

#[test]
fn tile_3_runs_one_rise_and_fall_three_times() {
    let bump = ok(Curve::with_kinds(
        vec![[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]],
        vec![true; 3],
    ));
    let tiled = values(&shaped(bump.clone(), 1, 1.0, 3));
    let peaks = (1..tiled.len() - 1)
        .filter(|&k| tiled[k] > tiled[k - 1] && tiled[k] >= tiled[k + 1])
        .count();
    assert_eq!(peaks, 3, "three bumps across the range");
    let tiled = shaped(bump.clone(), 1, 1.0, 3);
    for x in [1.0 / 6.0, 0.5, 5.0 / 6.0] {
        assert!(tiled.eval(x) > 0.999, "a top at {x}: {}", tiled.eval(x));
    }
    assert_eq!(
        tiled.eval(1.0),
        bump.eval(1.0),
        "it ends where the shape ends"
    );
}

#[test]
fn the_file_form_keeps_every_bit_and_old_forms_keep_their_bytes() {
    let pts = vec![
        [0.0, 0.1],
        [0.13, 0.37],
        [0.4, 0.29],
        [0.71, 0.93],
        [1.0, 1.0],
    ];
    let kinds = vec![true, true, false, true, true];
    let five = shaped(ok(Curve::with_kinds(pts.clone(), kinds)), 4, 0.5, 2);
    let text = write(&five);
    for key in [
        "steps = 4",
        "step_strength = 0.5",
        "tile = 2",
        "sharp = [2]",
    ] {
        assert!(text.contains(key), "{key} in {text}");
    }
    let back = ok(read(&text));
    assert_eq!(back, five, "{text}");
    let bits = |c: &Curve| {
        c.points()
            .iter()
            .map(|p| p.map(f32::to_bits))
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&back), bits(&five));
    assert_eq!(values(&back), values(&five));
    // A sharp curve with a setting says so.
    let sharp = shaped(ok(Curve::new(pts.clone())), 1, 1.0, 3);
    let text = write(&sharp);
    assert!(text.contains("smooth = false"), "{text}");
    assert_eq!(ok(read(&text)), sharp);
    // Without settings, nothing new is written.
    for plain in [
        ok(Curve::new(pts.clone())),
        ok(back.shaped(Shape::default())),
    ] {
        let text = write(&plain);
        for key in ["steps", "step_strength", "tile"] {
            assert!(!text.contains(key), "{key} in {text}");
        }
        assert_eq!(ok(read(&text)), plain);
    }
    for bad in [
        "c = { points = [[0, 0], [1, 1]], smooth = false, steps = 0 }",
        "c = { points = [[0, 0], [1, 1]], smooth = false, steps = 65 }",
        "c = { points = [[0, 0], [1, 1]], smooth = false, tile = 0 }",
        "c = { points = [[0, 0], [1, 1]], smooth = false, step_strength = 1.5 }",
    ] {
        assert!(read(bad).is_err(), "{bad} is refused");
    }
}

#[test]
fn invert_turns_the_values_over_and_keeps_kinds_and_settings() {
    let c = shaped(
        ok(Curve::with_kinds(
            vec![[0.0, 0.1], [0.4, 0.7], [1.0, 1.0]],
            vec![false, true, false],
        )),
        3,
        0.5,
        2,
    );
    let inv = c.inverted();
    let ys: Vec<f32> = inv.points().iter().map(|p| p[1]).collect();
    assert_eq!(ys, vec![1.0 - 0.1, 1.0 - 0.7, 0.0]);
    assert!(inv.is_smooth(1) && !inv.is_smooth(0));
    assert_eq!(inv.shape(), c.shape());
    let x: Vec<f32> = inv.points().iter().map(|p| p[0]).collect();
    assert_eq!(x, vec![0.0, 0.4, 1.0], "the inputs stay");
}

#[test]
fn a_sensitivity_scales_a_curve_toward_full_and_below_0_turns_it_round() {
    let smooth = ok(Curve::with_kinds(
        vec![[0.0, 0.2], [0.3, 0.35], [0.7, 0.9], [1.0, 1.0]],
        vec![true; 4],
    ));
    for c in [Curve::linear(), smooth] {
        assert_eq!(c.with_sensitivity(1.0), c, "100 is the curve itself");
        for k in [0.0_f32, 0.5, -0.5, -1.0] {
            let felt = c.with_sensitivity(k);
            for i in 0..=200 {
                let p = i as f32 / 200.0;
                let at = if k < 0.0 { 1.0 - p } else { p };
                let want = 1.0 - k.abs() * (1.0 - c.eval(at));
                let got = felt.eval(p);
                assert!((got - want).abs() < 1e-5, "{k} at {p}: {got} not {want}");
            }
        }
    }
    let half = Curve::linear().with_sensitivity(-0.5);
    assert_eq!(half.points(), [[0.0, 1.0], [1.0, 0.5]], "1 - 0.5 p");
    let flat = Curve::constant(1.0);
    for k in [0.0, 0.3, -0.7] {
        assert_eq!(
            flat.with_sensitivity(k),
            flat,
            "a flat curve at full stays flat"
        );
    }
}
