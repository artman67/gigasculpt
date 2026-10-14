//! The brush reference gives hand-computed answers on a flat grid, refuses what it does not
//! model, and agrees with gs-core's stroke pipeline on every shipped preset (so a broken
//! reference cannot pass unnoticed either way).

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::reference::stroke::{After, Dab, dab};

/// The grid: 21 by 21 vertices 0.1 apart on z = 0, from -1 to 1, quads facing +z.
fn grid() -> PlainMesh {
    let at = |i: u32, j: u32| i * 21 + j;
    let mut m = PlainMesh {
        positions: Vec::new(),
        faces: Vec::new(),
        attributes: BTreeMap::new(),
    };
    for i in 0..21 {
        for j in 0..21 {
            let c = |k: u32| ((f64::from(k) - 10.0) * 0.1) as f32;
            m.positions.push([c(i), c(j), 0.0]);
        }
    }
    for i in 0..20 {
        for j in 0..20 {
            m.faces
                .push([at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1)]);
        }
    }
    m
}

/// The grid vertex at `(x, y)`, in tenths.
fn v(x: i32, y: i32) -> usize {
    ((x + 10) * 21 + y + 10) as usize
}

fn with_mask(mut m: PlainMesh, f: impl Fn(usize) -> f32) -> PlainMesh {
    let values = (0..m.positions.len()).map(f).collect();
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(values),
    };
    m.attributes.insert("mask".into(), mask);
    m
}

/// A preset: strength 0.5, the kernel table, then any other tables.
fn preset(kernel: &str, rest: &str) -> String {
    format!(
        "name = \"T\"\nid = \"t\"\nfamily = \"Build\"\nstrength = 0.5\n[kernel]\n{kernel}\n{rest}\n"
    )
}

/// A press at the grid's center, radius 0.5, full pressure, camera looking down.
fn press() -> Dab<'static> {
    Dab {
        alpha: None,
        at: [0.0; 3],
        to: None,
        radius: 0.5,
        pressure: 1.0,
        view: [0.0, 0.0, -1.0],
        strength: None,
        invert: false,
    }
}

fn run(text: &str, mesh: &PlainMesh, d: Dab) -> After {
    dab(text, mesh, &d).unwrap_or_else(|e| panic!("{e}"))
}

/// The smooth falloff profile.
fn bell(d: f64) -> f64 {
    if d < 1.0 {
        3.0 * d * d * d * d - 4.0 * d * d * d + 1.0
    } else {
        0.0
    }
}

/// The weight of grid vertex `(x, y)` (tenths) for a radius `r` press at the center.
fn w(x: i32, y: i32, r: f64) -> f64 {
    bell(0.1 * f64::from(x * x + y * y).sqrt() / r)
}

fn close(got: f64, want: f64, what: &str) {
    assert!((got - want).abs() <= 1e-7, "{what}: got {got}, want {want}");
}

/// How far vertex `i` moved.
fn moved(a: &After, mesh: &PlainMesh, i: usize) -> [f64; 3] {
    std::array::from_fn(|k| a.positions[i][k] - f64::from(mesh.positions[i][k]))
}

const DRAW: &str = "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 2.0 }]";

#[test]
fn an_offset_dab_moves_by_gain_strength_tap_falloff_and_radius() {
    let g = grid();
    // amount = strength 0.5 * tap 0.5 = 0.25; move = gain 2 * w * 0.25 * radius 0.5 = 0.25 w.
    let a = run(&preset(DRAW, ""), &g, press());
    close(
        moved(&a, &g, v(2, 1))[2],
        0.25 * w(2, 1, 0.5),
        "up along the area normal",
    );
    close(moved(&a, &g, v(2, 1))[0], 0.0, "not sideways");
    close(moved(&a, &g, v(6, 0))[2], 0.0, "outside the radius");
    let inverted = run(
        &preset(DRAW, ""),
        &g,
        Dab {
            invert: true,
            ..press()
        },
    );
    close(
        moved(&inverted, &g, v(2, 1))[2],
        -0.25 * w(2, 1, 0.5),
        "inverted",
    );
    let light = run(
        &preset(DRAW, ""),
        &g,
        Dab {
            pressure: 0.5,
            ..press()
        },
    );
    close(
        moved(&light, &g, v(2, 1))[2],
        0.125 * w(2, 1, 0.5),
        "half pressure, half strength",
    );
    let small = preset(DRAW, "[pressure]\nsize = [[0, 0.5], [1, 0.5]]");
    let a = run(&small, &g, press());
    close(
        moved(&a, &g, v(2, 1))[2],
        0.125 * w(2, 1, 0.25),
        "the size curve halves the radius",
    );
    let strong = run(
        &preset(DRAW, ""),
        &g,
        Dab {
            strength: Some(1.0),
            ..press()
        },
    );
    close(
        moved(&strong, &g, v(2, 1))[2],
        0.5 * w(2, 1, 0.5),
        "the stroke's strength wins",
    );
    let half = with_mask(g.clone(), |i| if i == v(2, 1) { 0.5 } else { 1.5 });
    let a = run(&preset(DRAW, ""), &half, press());
    close(
        moved(&a, &half, v(2, 1))[2],
        0.125 * w(2, 1, 0.5),
        "half masked",
    );
    close(
        moved(&a, &half, v(1, 1))[2],
        0.0,
        "fully masked (clamped to 1)",
    );
}

#[test]
fn offset_directions_exponents_and_the_back_facing_fallback() {
    let g = grid();
    let term = |t: &str| preset(&format!("type = \"offset\"\nterms = [{{ {t} }}]"), "");
    let k = 0.25 * w(2, 1, 0.5);
    let x = run(&term("dir = \"x\", gain = 2.0"), &g, press());
    close(moved(&x, &g, v(2, 1))[0], k, "along x");
    let y = run(&term("dir = \"y\", gain = 2.0"), &g, press());
    close(moved(&y, &g, v(2, 1))[1], k, "along y");
    let z = run(&term("dir = \"z\", gain = 2.0"), &g, press());
    close(moved(&z, &g, v(2, 1))[2], k, "along z");
    let view = run(&term("dir = \"view\", gain = 2.0"), &g, press());
    close(moved(&view, &g, v(2, 1))[2], k, "toward the camera");
    let vn = run(&term("dir = \"vertex_normal\", gain = 2.0"), &g, press());
    close(moved(&vn, &g, v(2, 1))[2], k, "along the vertex normal");
    let sq = run(
        &term("dir = \"area_normal\", gain = 2.0, exponent = 2.0"),
        &g,
        press(),
    );
    close(
        moved(&sq, &g, v(2, 1))[2],
        0.25 * w(2, 1, 0.5) * w(2, 1, 0.5),
        "squared influence",
    );
    let below = run(
        &preset(DRAW, ""),
        &g,
        Dab {
            view: [0.0, 0.0, 1.0],
            ..press()
        },
    );
    close(
        moved(&below, &g, v(2, 1))[2],
        -k,
        "seen from behind: the normal is -view",
    );
    let all = preset(DRAW, "[sampling]\nfront_only = false");
    let a = run(
        &all,
        &g,
        Dab {
            view: [0.0, 0.0, 1.0],
            ..press()
        },
    );
    close(
        moved(&a, &g, v(2, 1))[2],
        k,
        "front_only off samples the back too",
    );
    let pinch = run(
        &term("dir = \"to_center\", gain = 2.0, tangent = true"),
        &g,
        press(),
    );
    let f = 1.0 - libm::exp(-0.5 * w(2, 1, 0.5));
    close(
        moved(&pinch, &g, v(2, 1))[0],
        -0.2 * f,
        "pulled toward the center",
    );
    close(
        moved(&pinch, &g, v(2, 1))[1],
        -0.1 * f,
        "pulled toward the center",
    );
}

#[test]
fn a_plane_dab_fills_up_to_its_offset_plane_on_the_chosen_side() {
    let g = grid();
    let plane = |more: &str| {
        preset(
            &format!("type = \"plane\"\ngain = 4.0\n{more}"),
            "[depth]\nimbed = 0.2",
        )
    };
    // The plane sits 0.2 * 0.5 = 0.1 above the grid; frac = 1 - exp(-4 * w * 0.25).
    let f = 1.0 - libm::exp(-w(2, 1, 0.5));
    let a = run(&plane("side = \"below\""), &g, press());
    close(
        moved(&a, &g, v(2, 1))[2],
        0.1 * f,
        "filled toward the plane",
    );
    let a = run(&plane("side = \"above\""), &g, press());
    close(moved(&a, &g, v(2, 1))[2], 0.0, "nothing above the plane");
    let a = run(&plane("depth = 0.1"), &g, press());
    close(moved(&a, &g, v(2, 1))[2], 0.0, "deeper than the cap");
    let a = run(&plane("height = 0.1\ndepth = 0.3"), &g, press());
    close(moved(&a, &g, v(2, 1))[2], 0.1 * f, "within the caps");
    let a = run(
        &plane("side = \"below\""),
        &g,
        Dab {
            invert: true,
            ..press()
        },
    );
    close(moved(&a, &g, v(2, 1))[2], -0.1 * f, "inverted, it carves");
}

/// The grid with its center raised by 0.1.
fn bump() -> PlainMesh {
    let mut g = grid();
    g.positions[v(0, 0)][2] = 0.1;
    g
}

#[test]
fn smooth_modes_pull_toward_their_targets() {
    let b = bump();
    let smooth = |mode: &str| {
        preset(
            &format!("type = \"smooth\"\nmode = \"{mode}\"\ngain = 2.0"),
            "",
        )
    };
    // frac = 1 - exp(-2 * w * 0.25).
    let f = |w: f64| 1.0 - libm::exp(-0.5 * w);
    // The bump sits 0.1 above the press: its weight is bell(0.2).
    let top = bell(0.2);
    let a = run(&smooth("uniform"), &b, press());
    close(
        moved(&a, &b, v(0, 0))[2],
        -0.1 * f(top),
        "the bump goes to its neighbours' mean",
    );
    close(
        moved(&a, &b, v(1, 0))[2],
        0.025 * f(w(1, 0, 0.5)),
        "a neighbour rises a quarter",
    );
    let a = run(&smooth("stronger"), &b, press());
    close(
        moved(&a, &b, v(0, 0))[2],
        -0.075 * f(top),
        "two rings: the target is 0.025",
    );
    let a = run(&smooth("tangent"), &b, press());
    close(moved(&a, &b, v(0, 0))[2], 0.0, "only along the surface");
    let inv = run(
        &smooth("uniform"),
        &b,
        Dab {
            invert: true,
            ..press()
        },
    );
    close(
        moved(&inv, &b, v(0, 0))[2],
        0.1 * f(top),
        "inverted, it sharpens",
    );
    let mut off = grid();
    off.positions[v(0, 0)][0] = 0.03;
    let a = run(&smooth("relax"), &off, press());
    let lens = [0.07, 0.13, (0.03f64 * 0.03 + 0.01).sqrt()];
    let l = (lens[0] + lens[1] + 2.0 * lens[2]) / 4.0;
    let sum = 0.07 - 0.13 - 0.06 - l * (1.0 - 1.0 - 2.0 * 0.03 / lens[2]);
    let w0 = bell(0.03 / 0.5);
    close(
        moved(&a, &off, v(0, 0))[0],
        sum / 4.0 * f(w0),
        "relax evens the edges",
    );
    let mut edge = grid();
    edge.positions[v(-10, 0)][0] = -0.97;
    let at_edge = Dab {
        at: [-1.0, 0.0, 0.0],
        ..press()
    };
    let a = run(&smooth("uniform"), &edge, at_edge);
    let w0 = bell(0.03 / 0.5);
    close(
        moved(&a, &edge, v(-10, 0))[0],
        -0.03 * f(w0),
        "a boundary vertex follows the boundary",
    );
}

/// The grid and, apart from it, one small square floating near its center.
fn island() -> PlainMesh {
    let mut m = grid();
    let base = m.positions.len() as u32;
    m.positions.extend([
        [0.05, 0.05, 0.05],
        [0.08, 0.05, 0.05],
        [0.08, 0.08, 0.05],
        [0.05, 0.08, 0.05],
    ]);
    m.faces.push([base, base + 1, base + 2, base + 3]);
    m
}

#[test]
fn move_selections_ball_connected_and_elastic() {
    let m = island();
    let grab = |select: &str| {
        preset(
            &format!("type = \"move\"\nselect = \"{select}\"\npoisson = 0.5"),
            "",
        )
    };
    let to = Dab {
        to: Some([0.1, 0.0, 0.1]),
        ..press()
    };
    // Strength 0.5 at full pressure.
    let a = run(&grab("ball"), &m, to);
    close(
        moved(&a, &m, v(2, 1))[0],
        0.05 * w(2, 1, 0.5),
        "dragged with the pointer",
    );
    close(
        moved(&a, &m, v(2, 1))[2],
        0.05 * w(2, 1, 0.5),
        "dragged with the pointer",
    );
    let island_vertex = m.positions.len() - 1;
    assert!(
        moved(&a, &m, island_vertex)[0] > 0.0,
        "the ball takes the island too"
    );
    let a = run(&grab("ball"), &m, Dab { invert: true, ..to });
    close(
        moved(&a, &m, v(2, 1))[0],
        0.0,
        "inverted: only along the normal",
    );
    close(
        moved(&a, &m, v(2, 1))[2],
        0.05 * w(2, 1, 0.5),
        "inverted: only along the normal",
    );
    let a = run(&grab("connected"), &m, to);
    close(
        moved(&a, &m, v(2, 1))[0],
        0.05 * bell(0.3 / 0.5),
        "weighed along the edges",
    );
    close(moved(&a, &m, v(3, 2))[0], 0.0, "too far along the edges");
    close(
        moved(&a, &m, island_vertex)[0],
        0.0,
        "not joined to the grab",
    );
    let pull = Dab {
        to: Some([0.1, 0.0, 0.0]),
        ..press()
    };
    let a = run(&grab("elastic"), &m, pull);
    close(
        moved(&a, &m, v(0, 0))[0],
        0.05,
        "the grab moves exactly with the pointer",
    );
    let fade = (15.0f64 / 16.0) * (15.0 / 16.0);
    close(
        moved(&a, &m, v(5, 0))[0],
        0.05 * fade / 2f64.sqrt(),
        "Kelvinlet along the pull",
    );
    close(
        moved(&a, &m, v(0, 5))[0],
        0.05 * fade * 0.75 / 2f64.sqrt(),
        "Kelvinlet across it",
    );
}

#[test]
fn a_hook_dab_moves_by_half_the_step_and_pinches_toward_the_motion() {
    let g = grid();
    let hook = preset("type = \"hook\"\npinch = 0.5", "");
    let a = run(
        &hook,
        &g,
        Dab {
            to: Some([0.04, 0.0, 0.0]),
            ..press()
        },
    );
    // The dab sits mid-cell: center at the press, step 0.02, strength 0.5.
    close(moved(&a, &g, v(0, 0))[0], 0.01, "the grab follows the step");
    close(
        moved(&a, &g, v(0, 1))[0],
        0.01 * w(0, 1, 0.5),
        "a neighbour follows less",
    );
    close(
        moved(&a, &g, v(0, 1))[1],
        -0.025 * w(0, 1, 0.5),
        "and is pinched toward the line",
    );
}

#[test]
fn the_mask_brush_paints_blurs_and_sharpens_and_ignores_the_mask() {
    let paint = |op: &str| {
        preset(
            &format!(
                "type = \"attribute\"\ncolumn = \"mask\"\nop = \"{op}\"\nvalue = 1.0\ngain = 4.0"
            ),
            "",
        )
    };
    // frac = 1 - exp(-4 * w * 0.25).
    let f = |w: f64| 1.0 - libm::exp(-w);
    let g = grid();
    let a = run(&paint("set"), &g, press());
    close(a.mask[v(2, 1)], f(w(2, 1, 0.5)), "painted toward 1");
    close(a.mask[v(6, 0)], 0.0, "outside the radius");
    let held = with_mask(g.clone(), |_| 0.6);
    let a = run(
        &paint("set"),
        &held,
        Dab {
            invert: true,
            ..press()
        },
    );
    close(
        a.mask[v(2, 1)],
        0.6 * (1.0 - f(w(2, 1, 0.5))),
        "inverted: toward 0, not held back",
    );
    let dot = with_mask(g.clone(), |i| if i == v(0, 0) { 1.0 } else { 0.0 });
    let a = run(&paint("blur"), &dot, press());
    close(
        a.mask[v(0, 0)],
        1.0 - f(1.0),
        "blur takes the neighbours' mean",
    );
    close(a.mask[v(1, 0)], 0.25 * f(w(1, 0, 0.5)), "blur spreads");
    let soft = with_mask(g, |i| if i == v(0, 0) { 0.5 } else { 0.25 });
    let a = run(&paint("sharpen"), &soft, press());
    close(
        a.mask[v(0, 0)],
        0.5 + 0.25 * f(1.0),
        "sharpen pushes away from the mean",
    );
}

#[test]
fn post_effects_gravity_and_auto_smooth() {
    let none = "type = \"offset\"\nterms = [{ dir = \"area_normal\", gain = 0.0 }]";
    let g = grid();
    let a = run(
        &preset(none, "[post]\ngravity = 0.5\ngravity_dir = [0, 0, 2]"),
        &g,
        press(),
    );
    close(
        moved(&a, &g, v(2, 1))[2],
        0.0625 * w(2, 1, 0.5),
        "gravity pulls along its direction",
    );
    let a = run(&preset(none, "[post]\ngravity = 0.5"), &g, press());
    close(
        moved(&a, &g, v(2, 1))[1],
        -0.0625 * w(2, 1, 0.5),
        "down by default",
    );
    let b = bump();
    let a = run(&preset(none, "[post]\nauto_smooth = 2.0"), &b, press());
    close(
        moved(&a, &b, v(0, 0))[2],
        -0.1 * (1.0 - libm::exp(-0.5 * bell(0.2))),
        "auto-smooth after the kernel",
    );
}

#[test]
fn tips_change_the_falloff() {
    let g = grid();
    let tip = |t: &str| run(&preset(DRAW, &format!("[tip]\n{t}")), &g, press());
    let d = 0.1 * 5f64.sqrt() / 0.5;
    let cases: [(&str, f64); 7] = [
        ("profile = \"sphere\"", (1.0 - d * d).sqrt()),
        ("profile = \"sharp\"", (1.0 - d) * (1.0 - d)),
        ("profile = \"linear\"", 1.0 - d),
        ("profile = \"constant\"", 1.0),
        (
            "profile = \"custom\"\npoints = [[0, 1], [0.5, 0.5], [1, 0]]",
            1.0 - d,
        ),
        ("hardness = 0.5", 1.0),
        ("focal = 50", bell(d.sqrt())),
    ];
    for (t, want) in cases {
        close(moved(&tip(t), &g, v(2, 1))[2], 0.25 * want, t);
    }
}

#[test]
fn what_the_reference_does_not_model_is_refused() {
    let g = grid();
    let bad = [
        preset(DRAW, "[automask]\nbackface = 0.5"),
        // An alpha with no levels given to read.
        preset(DRAW, "[tip.alpha]\nimage = \"grain\""),
        preset("type = \"attribute\"\ncolumn = \"color\"\ngain = 1.0", ""),
        preset("type = \"nope\"", ""),
        "this is = = not toml".to_string(),
    ];
    for text in bad {
        assert!(dab(&text, &g, &press()).is_err(), "refused: {text}");
    }
}

/// The shipped alpha `a` names, through its tone settings, as the engine resolves it.
fn shipped_alpha(a: &gs_core::brush::AlphaSpec) -> gs_core::alpha::Pyramid {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../assets/alphas/{}.toml", a.image));
    let text = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{e}"));
    let recipe = gs_core::alpha::Recipe::from_toml(&text).unwrap_or_else(|e| panic!("{e}"));
    let picture = recipe.draw().unwrap_or_else(|e| panic!("{e}"));
    gs_core::alpha::Pyramid::build(&picture, a)
}

#[test]
fn the_reference_agrees_with_the_stroke_pipeline_on_every_shipped_preset() {
    use gs_core::{BrushPreset, MeshStore, NodeConfig, StrokeParams, StrokeSample, Vec3, View};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brushes");
    let mesh = with_mask(gs_testkit::fixtures::icosphere(3), |i| (i % 5) as f32 * 0.2);
    let cfg = NodeConfig::new(6, 40).unwrap_or_else(|e| panic!("{e}"));
    let g = Vec3::new(0.3, 0.4, 0.86);
    let to = g + Vec3::new(-0.03, 0.04, 0.0);
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}")) {
        let text = std::fs::read_to_string(entry.unwrap_or_else(|e| panic!("{e}")).path())
            .unwrap_or_else(|e| panic!("{e}"));
        let p = BrushPreset::from_toml(&text).unwrap_or_else(|e| panic!("{e}"));
        if !p.claims.iter().any(|c| c == "reference") {
            continue;
        }
        let hook = p.name.starts_with("Pull Out");
        let two = matches!(p.stroke, gs_core::brush::StrokeSpec::Drag { .. }) || hook;
        let sample = |pos| StrokeSample {
            pos,
            pressure: 0.7,
            tilt: [0.0; 2],
            radius: 0.4,
        };
        // A hook's second sample stays in its first cell, half a step (0.04) from the press.
        let to = g + (to - g) * if hook { 0.5 } else { 1.0 };
        let samples = if two {
            vec![sample(g), sample(to)]
        } else {
            vec![sample(g)]
        };
        let mut s = MeshStore::from_plain(&mesh, cfg).unwrap_or_else(|e| panic!("{e}"));
        let before = s.to_plain();
        let view = View {
            dir: Vec3::new(0.0, 0.0, -1.0),
            eye: None,
        };
        let mut params = StrokeParams::new(view);
        params.alpha = p
            .tip
            .alpha
            .as_ref()
            .map(|a| std::sync::Arc::new(shipped_alpha(a)));
        // The pyramid's levels, as plain data for the reference.
        let levels = params.alpha.as_deref().map(|a| {
            gs_testkit::reference::alpha::read(
                a.levels(),
                |k| a.size(k),
                |k, x, y| a.texel(k, x, y),
            )
        });
        gs_core::stroke(&mut s, &p, params, &samples).unwrap_or_else(|e| panic!("{e}"));
        let after = s.to_plain();
        let f = |v: Vec3| v.to_array().map(f64::from);
        let d = Dab {
            alpha: levels.as_deref(),
            at: f(g),
            to: two.then(|| f(to)),
            radius: 0.4,
            pressure: 0.7,
            ..press()
        };
        let want = run(&text, &before, d);
        for (got, want) in after.positions.iter().zip(&want.positions) {
            for k in 0..3 {
                assert!(
                    (f64::from(got[k]) - want[k]).abs() < 3e-6,
                    "{}: {got:?} vs {want:?}",
                    p.name
                );
            }
        }
        if let Some(AttrValues::F32(mask)) = after.attributes.get("mask").map(|a| &a.values) {
            for (got, want) in mask.iter().zip(&want.mask) {
                assert!(
                    (f64::from(*got) - want).abs() < 3e-6,
                    "{}: mask {got} vs {want}",
                    p.name
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 17, "every preset that claims the reference");
}

#[test]
fn edges_of_the_rules_rim_caps_fallbacks_and_a_still_hook() {
    let g = grid();
    // A constant tip stops exactly at the rim: (5, 0) is exactly one radius out.
    let constant = run(&preset(DRAW, "[tip]\nprofile = \"constant\""), &g, press());
    close(moved(&constant, &g, v(4, 0))[2], 0.25, "inside the rim");
    close(moved(&constant, &g, v(5, 0))[2], 0.0, "on the rim");
    // Sampling one vertex makes the plane sit exactly at the cap (0.2 radii): it still moves.
    let one = "[sampling]\nnormal_radius = 0.01\nposition_radius = 0.01";
    let capped = |side: &str, imbed: f64| {
        let kernel = format!("type = \"plane\"\ngain = 4.0\n{side}");
        let rest = format!("{one}\n[depth]\nimbed = {imbed}");
        run(&preset(&kernel, &rest), &g, press())
    };
    let f = 1.0 - libm::exp(-w(2, 1, 0.5));
    close(
        moved(&capped("depth = 0.2", 0.2), &g, v(2, 1))[2],
        0.1 * f,
        "at the depth cap",
    );
    close(
        moved(&capped("height = 0.2", -0.2), &g, v(2, 1))[2],
        -0.1 * f,
        "at the height cap",
    );
    // Seen from behind nothing is sampled: the plane goes through the press, along -view.
    let behind = Dab {
        view: [0.0, 0.0, 1.0],
        ..press()
    };
    let plane = run(
        &preset("type = \"plane\"\ngain = 4.0", "[depth]\nimbed = 0.2"),
        &g,
        behind,
    );
    close(
        moved(&plane, &g, v(2, 1))[2],
        -0.1 * f,
        "a plane 0.1 below, facing -z",
    );
    // A hook that does not move pulls nothing.
    let still = Dab {
        to: Some([0.0; 3]),
        ..press()
    };
    let hook = run(&preset("type = \"hook\"\npinch = 0.5", ""), &g, still);
    close(moved(&hook, &g, v(0, 1))[1], 0.0, "no step, no pull");
}

#[test]
fn curves_with_a_bend_are_read_segment_by_segment() {
    let g = grid();
    let d = 0.1 * 5f64.sqrt() / 0.5;
    // A profile bent at 0.25: 1 down to 0.9, then straight down to 0 at the rim.
    let bent = "[tip]\nprofile = \"custom\"\npoints = [[0, 1], [0.25, 0.9], [1, 0]]";
    let want = 0.9 - 0.9 * (d - 0.25) / 0.75;
    close(
        moved(&run(&preset(DRAW, bent), &g, press()), &g, v(2, 1))[2],
        0.25 * want,
        "bent profile",
    );
    let near = run(&preset(DRAW, bent), &g, press());
    close(
        moved(&near, &g, v(1, 0))[2],
        0.25 * (1.0 - 0.1 * (0.2 / 0.25)),
        "first segment",
    );
    // Pressure through a bent strength curve: 0.8 at half pressure, so 0.4 at a quarter.
    let curve = "[pressure]\nstrength = [[0, 0], [0.5, 0.8], [1, 1]]";
    let quarter = run(
        &preset(DRAW, curve),
        &g,
        Dab {
            pressure: 0.25,
            ..press()
        },
    );
    close(
        moved(&quarter, &g, v(2, 1))[2],
        0.25 * 0.4 * w(2, 1, 0.5),
        "bent pressure curve",
    );
}

#[test]
fn sides_caps_and_the_view_direction_each_change_the_answer() {
    let g = grid();
    let plane = |more: &str| {
        let kernel = format!("type = \"plane\"\ngain = 4.0\n{more}");
        preset(&kernel, "[depth]\nimbed = -0.2")
    };
    let f = 1.0 - libm::exp(-w(2, 1, 0.5));
    // The plane 0.1 below the grid: everything is above it.
    let both = run(&plane(""), &g, press());
    close(
        moved(&both, &g, v(2, 1))[2],
        -0.1 * f,
        "both sides: pulled down",
    );
    let below = run(&plane("side = \"below\""), &g, press());
    close(
        moved(&below, &g, v(2, 1))[2],
        0.0,
        "only what is below moves",
    );
    let capped = run(&plane("height = 0.1"), &g, press());
    close(moved(&capped, &g, v(2, 1))[2], 0.0, "higher than the cap");
    // A slanted camera: the view term pushes back toward it.
    let view = [0.6, 0.0, -0.8];
    let slanted = Dab { view, ..press() };
    let toward = run(
        &preset(
            "type = \"offset\"\nterms = [{ dir = \"view\", gain = 2.0 }]",
            "",
        ),
        &g,
        slanted,
    );
    let k = 0.25 * w(2, 1, 0.5);
    close(
        moved(&toward, &g, v(2, 1))[0],
        -0.6 * k,
        "toward the camera, across",
    );
    close(
        moved(&toward, &g, v(2, 1))[2],
        0.8 * k,
        "toward the camera, up",
    );
}
