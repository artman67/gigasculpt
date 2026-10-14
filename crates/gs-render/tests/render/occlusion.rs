//! Ambient occlusion (catalog item `render.live-ambient-occlusion`, decision 0019): it darkens
//! a crease and leaves flats and convex ridges alone, it is the same at two zooms because its
//! radius is in scene units, strength 0 draws as no occlusion and more strength darkens the
//! crease more, and an imported flat-plus-crease MatCap shows its crease image where it darkens
//! (catalog item `material.cavity-shading`). A standard material's cavity settings scale the
//! session's occlusion (decision 0043, step (c)).

use crate::common::{hardware, lavapipe, upload};
use crate::scene;
use gs_core::material::{Cavity, Color};
use gs_render::display::decode;
use gs_render::matcap::{Bake, MatCapImage, Recipe, bake};
use gs_render::shading::Material;
use gs_render::{
    Camera, Display, Fit, GpuMatCap, GpuMesh, Image, Lens, Occlusion, Preset, Projection,
    RenderSize, Renderer, Shading, View, ViewTransform,
};

/// A strip across x from -1.5 to 1.5 (y from -1 to 1), flat at z = 0 except for a V-shaped
/// valley around x = -0.6 (concave, 0.3 deep, sides at 45 degrees) and a ridge around x = 0.6
/// (convex, 0.3 high). The camera looks down -z.
fn valley_and_ridge() -> gs_protocol::PlainMesh {
    let height = |x: f32| {
        let valley = (0.3 - (x + 0.6).abs()).max(0.0);
        let ridge = (0.3 - (x - 0.6).abs()).max(0.0);
        ridge - valley
    };
    scene::height_field(
        &scene::steps(-1.5, 1.5, 150),
        &scene::steps(-1.0, 1.0, 40),
        height,
    )
}

const SIZE: u32 = 256;

/// Looking down -z in parallel at x and y from -half to half.
fn front(half: f32) -> Camera {
    Camera {
        distance: half,
        lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
        projection: Projection::Orthographic,
        ..Camera::preset(Preset::Front)
    }
}

/// A one-color 8-bit MatCap of sRGB `level`, shown as the file looks (exposure 0).
fn flat(r: &Renderer, level: u8) -> GpuMatCap {
    let v = u16::from(level) * 257;
    r.upload_matcap(&MatCapImage {
        size: 4,
        pixels: vec![[v, v, v, u16::MAX]; 16],
    })
}

fn render(
    r: &Renderer,
    mesh: &GpuMesh,
    shading: Shading,
    half: f32,
    occlusion: Occlusion,
) -> Image {
    let view = View {
        occlusion,
        display: Display {
            exposure: 0.0,
            transform: ViewTransform::Standard,
            dither: false,
        },
        ..View::new(front(half), shading)
    };
    // The picture a still window settles to: at four samples the integrated GPU's
    // half-resolution occlusion still carries noise that differs between zooms.
    let size = RenderSize {
        samples: 16,
        ..RenderSize::square(SIZE)
    };
    r.snapshot(mesh, &view, size).expect("renders")
}

/// The mean display-linear gray of the pixels on the line x = `x` for y from -0.5 to 0.5, in
/// an image of x and y from -half to half.
fn along(image: &Image, half: f32, x: f32) -> f32 {
    channel(image, half, x, 0)
}

/// As [`along`], for channel `k` (0 red, 1 green, 2 blue).
fn channel(image: &Image, half: f32, x: f32, k: usize) -> f32 {
    let px = |v: f32| ((v + half) / (2.0 * half) * SIZE as f32) as u32;
    let column = px(x);
    let rows: Vec<u32> = (px(-0.5)..px(0.5)).collect();
    let sum: f32 = (rows.iter())
        .map(|&y| {
            let [r, g, b, _] = image.pixel(column, SIZE - 1 - y);
            decode([r, g, b])[k]
        })
        .sum();
    sum / rows.len() as f32
}

const RADIUS: Occlusion = Occlusion {
    radius: Some(0.25),
    strength: 1.0,
    cavity: Cavity::NEUTRAL,
};

/// Every renderer to check on: lavapipe (occlusion at full resolution) and the real GPUs here
/// (the integrated one works it out at half resolution and scales it up).
fn renderers() -> Vec<&'static Renderer> {
    std::iter::once(lavapipe())
        .chain(hardware().iter().map(|(_, r)| r))
        .collect()
}

#[test]
fn occlusion_darkens_the_crease_and_leaves_flats_and_ridges() {
    for r in renderers() {
        let mesh = upload(r, &valley_and_ridge());
        let gray = flat(r, 128);
        let shading = Shading::MatCap(&gray);
        let on = render(r, &mesh, shading, 1.6, RADIUS);
        let off = render(r, &mesh, shading, 1.6, Occlusion::OFF);
        let lit = decode([128, 0, 0])[0];
        let name = r.gpu().adapter_name();
        for x in [-1.3f32, 0.0, 1.3] {
            let (a, b) = (along(&on, 1.6, x), along(&off, 1.6, x));
            assert!(
                (a - b).abs() < 0.002 && (b - lit).abs() < 0.002,
                "{name}: the flat at x = {x} is untouched: {a} with occlusion, {b} without"
            );
        }
        let ridge = along(&on, 1.6, 0.6);
        assert!(
            (ridge - lit).abs() < 0.002,
            "{name}: the ridge's crest is untouched: {ridge} against {lit}"
        );
        let crease = along(&on, 1.6, -0.6);
        // 19% darker at full quality; 14.5% with the integrated GPU's fewer steps at half
        // resolution, which find less of a crease narrower than an occlusion texel.
        assert!(
            crease < 0.9 * lit,
            "{name}: the valley's crease is darker: {crease} against {lit}"
        );
        assert!(
            (along(&off, 1.6, -0.6) - lit).abs() < 0.002,
            "{name}: without occlusion the crease is lit like the flats"
        );
    }
}

/// The radius is in scene units, so the crease at half the camera's reach (twice the size on
/// screen) is as dark as before, and the flats stay untouched.
#[test]
fn occlusion_is_the_same_at_two_zooms() {
    for r in renderers() {
        let mesh = upload(r, &valley_and_ridge());
        let gray = flat(r, 128);
        let shading = Shading::MatCap(&gray);
        let far = render(r, &mesh, shading, 1.6, RADIUS);
        let near = render(r, &mesh, shading, 0.8, RADIUS);
        for x in [-0.75f32, -0.6, -0.45, 0.0] {
            let (a, b) = (along(&far, 1.6, x), along(&near, 0.8, x));
            assert!(
                (a - b).abs() < 0.006,
                "{}: x = {x}: {a} from afar, {b} from twice as close",
                r.gpu().adapter_name()
            );
        }
    }
}

/// Strength 0 draws the same bytes as no occlusion, whatever the radius, in MatCap and Clay
/// (decision 0019 as amended for Q8: Clay takes occlusion).
#[test]
fn strength_0_draws_as_no_occlusion() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let gray = flat(r, 128);
    for (name, shading) in [("MatCap", Shading::MatCap(&gray)), ("Clay", Shading::Clay)] {
        let off = render(r, &mesh, shading, 1.6, Occlusion::OFF);
        let none = Occlusion {
            strength: 0.0,
            ..RADIUS
        };
        let on = render(r, &mesh, shading, 1.6, RADIUS);
        assert!(on != off, "{name}: full strength darkens the crease");
        assert!(
            render(r, &mesh, shading, 1.6, none) == off,
            "{name}: strength 0 is no occlusion"
        );
    }
}

/// Each step of strength darkens the crease by the same amount (strength scales how much
/// light occlusion takes away) and leaves the flats as they are, in MatCap and Clay. Clay's
/// crease darkens less (5% here against MatCap's 19%): occlusion takes only the environment's
/// light, and the key light, which it leaves, is most of Clay's.
#[test]
fn more_strength_darkens_the_crease_more_and_leaves_the_flats() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let gray = flat(r, 128);
    for (name, shading) in [("MatCap", Shading::MatCap(&gray)), ("Clay", Shading::Clay)] {
        let at = |strength: f32| {
            let occlusion = Occlusion { strength, ..RADIUS };
            render(r, &mesh, shading, 1.6, occlusion)
        };
        let [none, half, full] = [at(0.0), at(0.5), at(1.0)];
        for x in [-1.3f32, 0.0, 1.3] {
            let flats = [&none, &half, &full].map(|image| along(image, 1.6, x));
            assert!(
                flats.iter().all(|f| (f - flats[0]).abs() < 0.002),
                "{name}: the flat at x = {x} stays {flats:?}"
            );
        }
        let crease = [&none, &half, &full].map(|image| along(image, 1.6, -0.6));
        assert!(
            crease[0] > crease[1] && crease[1] > crease[2] && crease[2] < 0.97 * crease[0],
            "{name}: the crease darkens with strength: {crease:?}"
        );
        let middle = f32::midpoint(crease[0], crease[2]);
        assert!(
            (crease[1] - middle).abs() < 0.01 * crease[0],
            "{name}: half strength takes half the light: {crease:?}"
        );
    }
}

/// An 8-bit MatCap image of one gray `level` everywhere.
fn image(level: u8) -> MatCapImage {
    let v = u16::from(level) * 257;
    MatCapImage {
        size: 4,
        pixels: vec![[v, v, v, u16::MAX]; 16],
    }
}

/// Cavity Transition (catalog item `material.cavity-shading`; the setting comes with
/// `material.matcap-import`, phase 2): on a flat-plus-crease pair, -1 shows only the flat image
/// and +1 only the crease image, on the flat and in the crease alike.
#[test]
fn cavity_transition_leans_the_pair_to_one_image() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    for (t, level) in [(-1.0, 200), (1.0, 20)] {
        let pair = r.upload_matcap_pair(&image(200), &image(20)).transition(t);
        let shown = render(r, &mesh, Shading::MatCap(&pair), 1.6, RADIUS);
        let want = decode([level, 0, 0])[0];
        for x in [1.3, -0.6] {
            let got = along(&shown, 1.6, x);
            assert!(
                (got - want).abs() < 0.002,
                "transition {t} at x = {x}: {got} against {want}"
            );
        }
    }
}

/// An imported two-sphere MatCap read as flat plus crease shows its flat image where nothing
/// occludes and turns toward its crease image in the crease.
#[test]
fn a_crease_image_shows_where_occlusion_darkens() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let pair = r.upload_matcap_pair(&image(200), &image(20));
    let on = render(r, &mesh, Shading::MatCap(&pair), 1.6, RADIUS);
    let light = decode([200, 0, 0])[0];
    let dark = decode([20, 0, 0])[0];
    let flat_light = along(&on, 1.6, 1.3);
    assert!(
        (flat_light - light).abs() < 0.002,
        "the flat shows the flat image: {flat_light} against {light}"
    );
    let crease = along(&on, 1.6, -0.6);
    assert!(
        crease < light - 0.1 * (light - dark),
        "the crease turns toward the crease image: {crease} between {dark} and {light}"
    );
}

/// The session's occlusion `RADIUS` with a standard material's cavity settings.
fn with(cavity: Cavity) -> Occlusion {
    Occlusion { cavity, ..RADIUS }
}

/// A standard material in both shadings its cavity settings act in: MatCap, baked small from
/// `OpenPBR`'s default material under the studio, and Lit.
fn standard(r: &Renderer) -> (GpuMatCap, Material) {
    let recipe = Recipe {
        bake: Bake {
            size: 64,
            ..Bake::default()
        },
        ..Recipe::default()
    };
    let baked = bake(&recipe).expect("bakes");
    (r.upload_bake(&baked), recipe.material)
}

/// A standard material's Intensity (decision 0043, step (c)) scales the darkening the session's
/// occlusion leaves, in MatCap and Lit: at 0 the material draws as with occlusion off while the
/// same session occlusion still darkens the crease of a material with neutral settings, at 2
/// the crease is darker than at 1 and the flats stay, and at session strength 0 occlusion is
/// off whatever the Intensity.
#[test]
fn cavity_intensity_scales_the_sessions_darkening_and_strength_0_is_off_everywhere() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let (baked, material) = standard(r);
    let looks = [
        ("MatCap", Shading::MatCap(&baked)),
        ("Lit", Shading::Lit(&material)),
    ];
    let intensity = |intensity: f32| {
        with(Cavity {
            intensity,
            ..Cavity::NEUTRAL
        })
    };
    for (name, shading) in looks {
        let shown = |occlusion: Occlusion| render(r, &mesh, shading, 1.6, occlusion);
        let (off, neutral) = (shown(Occlusion::OFF), shown(RADIUS));
        assert!(
            neutral != off,
            "{name}: the session's occlusion darkens a neutral material's crease"
        );
        assert!(
            shown(intensity(0.0)) == off,
            "{name}: Intensity 0 is no occlusion"
        );
        let session_off = Occlusion {
            strength: 0.0,
            ..intensity(2.0)
        };
        assert!(
            shown(session_off) == off,
            "{name}: session strength 0 is off whatever the Intensity"
        );
        let twice = shown(intensity(2.0));
        for x in [-1.3f32, 0.0, 1.3] {
            let (a, b) = (along(&twice, 1.6, x), along(&neutral, 1.6, x));
            assert!(
                (a - b).abs() < 0.002,
                "{name}: the flat at x = {x} stays: {a} against {b}"
            );
        }
        let crease = [&off, &neutral, &twice].map(|image| along(image, 1.6, -0.6));
        assert!(
            crease[0] > crease[1] + 0.01 && crease[1] > crease[2] + 0.01,
            "{name}: Intensity 2 darkens the crease more than 1: {crease:?}"
        );
    }
}

/// A standard material's Radius (decision 0043, step (c)) multiplies the session's radius: twice
/// a radius of 0.125 draws as 0.25, which draws otherwise than 0.125.
#[test]
fn cavity_radius_multiplies_the_sessions_radius() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let (baked, _) = standard(r);
    let at = |session: f32, radius: f32| {
        let occlusion = Occlusion {
            radius: Some(session),
            ..with(Cavity {
                radius,
                ..Cavity::NEUTRAL
            })
        };
        render(r, &mesh, Shading::MatCap(&baked), 1.6, occlusion)
    };
    let quarter = at(0.25, 1.0);
    assert!(at(0.125, 1.0) != quarter, "the radius shows");
    assert!(at(0.125, 2.0) == quarter, "Radius 2 doubles the session's");
}

/// A standard material's Colorize (decision 0043, step (c)) gives back the light occlusion
/// takes, in the cavity color, and nothing else: at 1 in pure red, the crease's red is as with
/// occlusion off and its green as with no Colorize, and the flats, where nothing is occluded,
/// stay. In MatCap and Lit.
#[test]
fn cavity_colorize_tints_only_the_darkened_share() {
    let r = lavapipe();
    let mesh = upload(r, &valley_and_ridge());
    let (baked, material) = standard(r);
    let looks = [
        ("MatCap", Shading::MatCap(&baked)),
        ("Lit", Shading::Lit(&material)),
    ];
    let red = with(Cavity {
        colorize: 1.0,
        color: Color([1.0, 0.0, 0.0]),
        ..Cavity::NEUTRAL
    });
    for (name, shading) in looks {
        let [off, neutral, tinted] =
            [Occlusion::OFF, RADIUS, red].map(|o| render(r, &mesh, shading, 1.6, o));
        let crease = |image: &Image, k: usize| channel(image, 1.6, -0.6, k);
        let reds = [&off, &neutral, &tinted].map(|image| crease(image, 0));
        assert!(
            reds[1] < reds[0] - 0.01 && (reds[2] - reds[0]).abs() < 0.002,
            "{name}: the crease's red is given back: {reds:?} (off, neutral, tinted)"
        );
        let greens = [&neutral, &tinted].map(|image| crease(image, 1));
        assert!(
            (greens[1] - greens[0]).abs() < 0.002,
            "{name}: the crease's green stays darkened: {greens:?}"
        );
        for (x, k) in [-1.3f32, 0.0, 1.3]
            .into_iter()
            .flat_map(|x| [(x, 0), (x, 1)])
        {
            let (a, b) = (channel(&tinted, 1.6, x, k), channel(&neutral, 1.6, x, k));
            assert!(
                (a - b).abs() < 0.002,
                "{name}: channel {k} of the flat at x = {x} stays: {a} against {b}"
            );
        }
    }
}
