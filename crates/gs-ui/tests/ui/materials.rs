//! Materials through gs-ui's render hosts (catalog items `material.matcap-shading`,
//! `material.cavity-shading` and `material.palette`): what a look draws on a painted part, the
//! MatCap flip, and the palette's slots, files and undo as the window and agents reach them.

use gs_engine::{Engine, EngineConfig};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::fixtures::icosphere;
use gs_ui::Offscreen;
use serde_json::{Value, json};

use crate::looks::{Shot, run, snapshot};

/// A headless session on lavapipe with nothing in it.
pub fn empty() -> Engine {
    let mut e = Engine::new(EngineConfig::default()).unwrap_or_else(|e| panic!("{e}"));
    let render = Offscreen::host(Some("lavapipe")).unwrap_or_else(|e| panic!("{e}"));
    e.attach_renderer(render);
    e
}

/// A unit sphere, red (sRGB 200, 45, 45) where x is above 0.2 when `paint` is on, with no color
/// column otherwise.
fn ball(paint: bool) -> PlainMesh {
    let mut mesh = icosphere(5);
    if paint {
        let red = u32::from_le_bytes([200, 45, 45, 255]);
        let words = (mesh.positions.iter())
            .map(|p| if p[0] > 0.2 { red } else { u32::MAX })
            .collect();
        let column = Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::U32(words),
        };
        mesh.attributes.insert("color".into(), column);
    }
    mesh
}

/// The pixels where `a` and `b` differ, as `b` draws them.
fn changed(a: &Shot, b: &Shot) -> Vec<[u8; 3]> {
    (a.1.chunks(4).zip(b.1.chunks(4)))
        .filter(|(x, y)| x != y)
        .map(|(_, y)| [y[0], y[1], y[2]])
        .collect()
}

/// The mean of channel `k` over `pixels`.
fn mean(pixels: &[[u8; 3]], k: usize) -> f64 {
    let sum: f64 = pixels.iter().map(|p| f64::from(p[k])).sum();
    sum / pixels.len().max(1) as f64
}

/// The sphere seen from the front in `look` and `shading`, painted or not, after `view.display`
/// with `display`.
fn front(paint: bool, look: &str, shading: &str, display: Value) -> Shot {
    let mut e = empty();
    run(
        &mut e,
        "part.create",
        json!({ "name": "ball", "mesh": ball(paint) }),
    );
    run(&mut e, "material.select", json!({ "name": look }));
    run(&mut e, "view.display", display);
    let args: Value =
        json!({ "view": "front", "size": 128, "antialias": false, "shading": shading });
    snapshot(&mut e, args)
}

/// `shot` mirrored left to right.
fn mirror(shot: &Shot) -> Shot {
    let row = 4 * shot.0 as usize;
    let rows = shot.1.chunks(row);
    let pixels = rows.flat_map(|r| r.chunks(4).rev().flatten().copied().collect::<Vec<u8>>());
    (shot.0, pixels.collect())
}

/// The pixels of `a` and `b` some channel of which differs by more than `levels`.
fn apart(a: &Shot, b: &Shot, levels: u8) -> usize {
    let far = |(x, y): (&[u8], &[u8])| x.iter().zip(y).any(|(p, q)| p.abs_diff(*q) > levels);
    (a.1.chunks(4).zip(b.1.chunks(4)))
        .filter(|&p| far(p))
        .count()
}

/// The MatCap flip (catalog item `material.matcap-shading`, as Blender's MatCap flip): with
/// `view.display`'s `matcap_flip` on, MatCap Gray, lit from one side, draws a sphere seen head
/// on as the mirror image of the unflipped picture, within the dither's level (with occlusion
/// off: its sampling pattern is not mirrored, and moved rim pixels by up to 5 levels). Lit
/// shading reads no MatCap and draws byte for byte as before.
#[test]
fn the_matcap_flip_mirrors_the_look() {
    let shown = |shading, flip| {
        let display = json!({ "matcap_flip": flip, "occlusion_strength": 0.0 });
        front(false, "gray", shading, display)
    };
    let (plain, flipped) = (shown("matcap", false), shown("matcap", true));
    let changed = apart(&plain, &flipped, 2);
    assert!(
        changed > 1000,
        "the flip changes the picture ({changed} pixels)"
    );
    let off = apart(&mirror(&plain), &flipped, 2);
    assert!(off == 0, "flipped is the mirror image ({off} pixels off)");
    assert!(
        shown("lit", true) == shown("lit", false),
        "the flip leaves Lit shading alone"
    );
}

/// HSV saturation, 0 to 1, of an RGBA pixel.
fn saturation(p: &[u8]) -> f64 {
    let (hi, lo) = (p[..3].iter().max(), p[..3].iter().min());
    let (hi, lo) = (f64::from(*hi.unwrap_or(&0)), f64::from(*lo.unwrap_or(&0)));
    if hi > 0.0 { (hi - lo) / hi } else { 0.0 }
}

/// Cavity shading (catalog item `material.cavity-shading`): on MatCap Red Wax, with an
/// occlusion radius of half the sphere's, the pixels occlusion darkens where a box meets a
/// sphere (by more than 20 levels summed over the channels) draw more saturated than the same
/// pixels with occlusion off (0.771 against 0.713 here): the multi-bounce fit keeps more of a
/// red albedo's red, so a wax crease turns a deeper red instead of gray.
#[test]
fn a_colored_crease_is_darker_and_more_saturated() {
    let mut e = crate::display::creased(1.0);
    run(&mut e, "material.select", json!({ "name": "red-wax" }));
    run(&mut e, "view.display", json!({ "occlusion_radius": 0.5 }));
    let mut shown = |strength: f64| {
        run(
            &mut e,
            "view.display",
            json!({ "occlusion_strength": strength }),
        );
        let args = json!({ "view": "front", "size": 128, "antialias": false });
        snapshot(&mut e, args)
    };
    let (on, off) = (shown(1.0), shown(0.0));
    let luma = |p: &[u8]| p[..3].iter().map(|&c| u32::from(c)).sum::<u32>();
    let crease: Vec<(&[u8], &[u8])> = (on.1.chunks(4).zip(off.1.chunks(4)))
        .filter(|(a, b)| luma(a) + 20 < luma(b))
        .collect();
    assert!(
        crease.len() > 100,
        "occlusion darkens a crease ({} pixels)",
        crease.len()
    );
    let mean = |k: usize| {
        let each = crease
            .iter()
            .map(|p| saturation(if k == 0 { p.0 } else { p.1 }));
        each.sum::<f64>() / crease.len() as f64
    };
    let (dark, light) = (mean(0), mean(1));
    assert!(
        dark > light + 0.03,
        "the crease is more saturated ({dark:.3} against {light:.3})"
    );
}

/// A standard material's cavity Intensity (decision 0043, step (c)) through the render host,
/// in MatCap and Lit shading: at 0 the material draws its crease as with the session's
/// occlusion off, while that occlusion still darkens the crease of every other material, a
/// standard one and a MatCap look.
#[test]
fn cavity_intensity_0_takes_one_materials_crease_darkening_only() {
    let mut e = crate::display::creased(1.0);
    run(&mut e, "view.display", json!({ "occlusion_radius": 0.5 }));
    let none = json!({ "material": "basic-material", "cavity_intensity": 0.0 });
    run(&mut e, "material.set", none);
    for shading in ["matcap", "lit"] {
        for (look, darkens) in [
            ("basic-material", false),
            ("basic-material-2", true),
            ("gray", true),
        ] {
            run(&mut e, "material.select", json!({ "name": look }));
            let mut shown = |strength: f64| {
                let display = json!({ "occlusion_strength": strength });
                run(&mut e, "view.display", display);
                let args =
                    json!({ "view": "front", "size": 128, "antialias": false, "shading": shading });
                snapshot(&mut e, args)
            };
            let (on, off) = (shown(1.0), shown(0.0));
            assert_eq!(on != off, darkens, "{shading}, {look}: the crease darkens");
        }
    }
}

/// R7 (research 27): paint shows on a metal look. A metal's MatCap has no diffuse layer (all
/// its light is reflection, its base color the reflection's color), so paint that multiplied
/// only the diffuse part vanished. On MatCap Metal 1, in MatCap shading and lit live, the
/// painted half differs from the unpainted picture and is red.
#[test]
fn paint_shows_on_a_metal_look() {
    for shading in ["matcap", "lit"] {
        let plain = front(false, "metal-1", shading, json!({}));
        let painted = front(true, "metal-1", shading, json!({}));
        let patch = changed(&plain, &painted);
        assert!(
            patch.len() > 500,
            "{shading}: the paint shows on metal ({} pixels changed)",
            patch.len()
        );
        let (r, g, b) = (mean(&patch, 0), mean(&patch, 1), mean(&patch, 2));
        assert!(
            r > 1.5 * g && r > 1.5 * b,
            "{shading}: the painted metal is red ({r:.0}, {g:.0}, {b:.0})"
        );
    }
}

/// Flat Color (catalog item `material.flat-color`, decision 0043 step (c)) through the render
/// host, in MatCap and Lit shading: the painted sphere shows its paint as stored where it is
/// painted and white elsewhere (its base color is white), nothing darker than the paint, and
/// byte for byte the same with light 1 off, four times the ambient, the captured studio and
/// exposure +2.
#[test]
fn flat_color_shows_the_paint_as_stored_whatever_the_light() {
    for shading in ["matcap", "lit"] {
        let shot = front(true, "flat-color", shading, json!({}));
        let mut e = empty();
        let ball = json!({ "name": "ball", "mesh": ball(true) });
        run(&mut e, "part.create", ball);
        run(&mut e, "material.select", json!({ "name": "flat-color" }));
        run(&mut e, "view.display", json!({ "exposure": 2.0 }));
        let dark = json!({ "light": 1, "on": false, "ambient": 4.0 });
        run(&mut e, "light.set", dark);
        let captured = json!({ "name": "photo-studio-01" });
        run(&mut e, "view.environment", captured);
        let args = json!({ "view": "front", "size": 128, "antialias": false, "shading": shading });
        assert!(
            shot == snapshot(&mut e, args),
            "{shading}: the light changes no pixel"
        );
        let background = &shot.1[..4];
        let surface: Vec<&[u8]> = (shot.1.chunks(4)).filter(|p| *p != background).collect();
        let red = |p: &[u8]| p.iter().zip([200, 45, 45]).all(|(a, b)| a.abs_diff(b) <= 1);
        let (reds, whites) = (
            surface.iter().filter(|p| red(p)).count(),
            surface.iter().filter(|p| p[..3] == [255; 3]).count(),
        );
        assert!(
            reds > 1000 && whites > 1000,
            "{shading}: {reds} red and {whites} white"
        );
        let darker = surface
            .iter()
            .find(|p| p[0] < 199 || p[1] < 44 || p[2] < 44);
        assert!(
            darker.is_none(),
            "{shading}: nothing darker than the paint: {darker:?}"
        );
    }
}
