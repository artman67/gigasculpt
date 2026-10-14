//! `pick.color`, the unshaded pick (decision 0039 section 11, amended for P8b): the stored color
//! where a ray first meets a part, inside a triangle the corners' colors mixed in linear light
//! as the GPU draws them, white where nothing is painted, and nothing off every part.

use std::collections::BTreeMap;

use base64::Engine as _;
use gs_io::mesh::column;
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_render::Image;
use gs_testkit::Session;
use serde_json::{Value, json};

use crate::support::agent;

/// The nine colors of [`sheet`]'s points, in order.
const SHEET: [[u8; 3]; 9] = [
    [0x12, 0x34, 0x56],
    [0xff, 0x00, 0x00],
    [0x00, 0xff, 0x00],
    [0x00, 0x00, 0xff],
    [0xab, 0xcd, 0xef],
    [0x80, 0x80, 0x80],
    [0xff, 0xff, 0x00],
    [0x01, 0x02, 0x03],
    [0xfe, 0xdc, 0xba],
];

/// The side of the deciding test's picture, in pixels.
const SIZE: u32 = 64;

/// A color as the color column holds it: red in the low byte, opaque.
fn word([r, g, b]: [u8; 3]) -> u32 {
    u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16 | 0xff00_0000
}

/// `#rrggbb`.
fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The bytes of `#rrggbb`.
fn bytes(hex: &str) -> [u8; 3] {
    let channel = |i: usize| u8::from_str_radix(&hex[1 + 2 * i..3 + 2 * i], 16).unwrap_or(0);
    [channel(0), channel(1), channel(2)]
}

/// A flat mesh facing +Z, each point painted its color.
fn painted(positions: Vec<[f32; 3]>, faces: Vec<[u32; 4]>, colors: &[[u8; 3]]) -> PlainMesh {
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(colors.iter().map(|&c| word(c)).collect()),
    };
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::from([("color".into(), column)]),
    }
}

/// Nine points one apart, from (-1, -1) to (1, 1), in four quads, painted [`SHEET`].
fn sheet() -> PlainMesh {
    let positions = (0..9)
        .map(|v| [(v % 3) as f32 - 1.0, (v / 3) as f32 - 1.0, 0.0])
        .collect();
    let quad = |i: u32, j: u32| {
        let v = |i: u32, j: u32| j * 3 + i;
        [v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1)]
    };
    let faces = vec![quad(0, 0), quad(1, 0), quad(0, 1), quad(1, 1)];
    painted(positions, faces, &SHEET)
}

/// What `pick.color` answers for a ray straight down onto the plane at `(x, y)`.
fn down(s: &mut Session, [x, y]: [f32; 2]) -> Value {
    let ray = json!({ "origin": [x, y, 5.0], "direction": [0.0, 0.0, -1.0] });
    s.query("pick.color", ray)
}

/// At each painted corner it answers that corner's bytes exactly, and names the part.
#[test]
fn a_painted_corner_answers_its_stored_bytes_and_its_part() {
    let mut s = agent();
    s.add_part("sheet", &sheet());
    for (v, color) in SHEET.into_iter().enumerate() {
        let [x, y] = [(v % 3) as f32 - 1.0, (v / 3) as f32 - 1.0];
        // A hundred-thousandth inside the sheet, so the ray never grazes its rim.
        let at = [x - 1e-5 * x.signum(), y - 1e-5 * y.signum()];
        let answer = down(&mut s, at);
        assert_eq!(
            answer,
            json!({ "color": hex(color), "part": "sheet" }),
            "point {v}"
        );
    }
}

/// The deciding test (decision 0039 section 11, amended for P8b): one large triangle with a
/// red, a green and a blue corner, drawn face on in Flat Color, which shows paint exactly. At
/// every pixel well inside it, `pick.color` through that pixel's center answers what was drawn
/// there within one level. The nearest corner's bytes would answer only pure red, green or
/// blue, where the middle is drawn near gray.
#[test]
fn inside_a_triangle_it_answers_the_color_flat_color_draws_there() {
    let corners = vec![[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
    let rgb = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
    let mut s = agent();
    let renderer = gs_ui::Offscreen::host(None).unwrap_or_else(|e| panic!("{e}"));
    s.engine.attach_renderer(renderer);
    s.add_part(
        "triangle",
        &painted(corners, vec![[0, 1, 2, u32::MAX]], &rgb),
    );
    s.ok("material.select", json!({ "name": "flat-color" }));
    let shot = s.query(
        "view.snapshot",
        json!({ "view": "front", "size": SIZE, "antialias": false }),
    );
    let png = (base64::engine::general_purpose::STANDARD)
        .decode(shot["png_base64"].as_str().unwrap_or_default())
        .unwrap_or_else(|e| panic!("{e}"));
    let image = Image::from_png(&png).unwrap_or_else(|e| panic!("{e}"));
    let picked: Vec<Option<[u8; 3]>> = (0..SIZE * SIZE)
        .map(|i| {
            let at = [(i % SIZE) as f32 + 0.5, (i / SIZE) as f32 + 0.5];
            let args = json!({ "camera": "front", "viewport": [SIZE, SIZE], "at": at });
            let answer = s.query("pick.color", args);
            answer["color"].as_str().map(bytes)
        })
        .collect();
    let at = |x: u32, y: u32| picked[(y * SIZE + x) as usize];
    let mut compared = Vec::new();
    for (x, y) in (1..SIZE - 1).flat_map(|y| (1..SIZE - 1).map(move |x| (x, y))) {
        let inside = [(x, y), (x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
        let (Some(answer), true) = (at(x, y), inside.iter().all(|&(x, y)| at(x, y).is_some()))
        else {
            continue;
        };
        let drawn = image.pixel(x, y);
        let off = (0..3).map(|k| answer[k].abs_diff(drawn[k])).max();
        assert!(
            off <= Some(1),
            "pixel ({x}, {y}): picked {answer:?}, drawn {drawn:?}"
        );
        compared.push(answer);
    }
    assert!(compared.len() > 300, "{} pixels inside", compared.len());
    let mixed = compared.iter().any(|c| c.iter().all(|&v| v >= 100));
    assert!(mixed, "the middle is a mix, not a corner's bytes");
}

/// Off every part it answers no color and no part, so the swatch keeps its color.
#[test]
fn off_every_part_it_answers_nothing() {
    let mut s = agent();
    s.add_part("sheet", &sheet());
    assert_eq!(down(&mut s, [5.0, 5.0]), json!({}));
}

/// A part never painted answers white, the empty value.
#[test]
fn a_part_never_painted_answers_white() {
    let mut s = agent();
    s.ok("primitive.sphere", json!({ "subdivisions": 3 }));
    let answer = down(&mut s, [0.1, 0.2]);
    assert_eq!(answer, json!({ "color": "#ffffff", "part": "sphere" }));
}

/// Paint hidden by Show Paint still answers its stored color: the switch hides paint and
/// keeps it.
#[test]
fn hidden_paint_answers_its_stored_color() {
    let mut s = agent();
    s.add_part("sheet", &sheet());
    s.ok("part.set_paint_visible", json!({ "visible": false }));
    let answer = down(&mut s, [1e-5, 1e-5]);
    assert_eq!(answer["color"], json!(hex(SHEET[4])));
}

/// On a part with detail levels it answers from the level shown: at the top, the color its
/// middle point stores; one level down, where that point is not, the mix of the base's corners
/// along the diagonal the middle lies on, which is no gray.
#[test]
fn a_part_with_detail_levels_answers_the_level_shown() {
    let corners = vec![
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let rgbw = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]];
    let mut s = agent();
    s.add_part("quad", &painted(corners, vec![[0, 1, 2, 3]], &rgbw));
    s.ok("level.subdivide", json!({}));
    let top = s.mesh("quad");
    let middle = (top.positions.iter())
        .position(|p| p[0].abs() < 1e-6 && p[1].abs() < 1e-6)
        .unwrap_or_else(|| panic!("no middle point: {:?}", top.positions));
    let colors: &[u32] =
        column(&top, "color", AttrDomain::Vertex).unwrap_or_else(|| panic!("no color"));
    let stored = colors[middle].to_le_bytes();
    let near = [1e-5, 5e-6];
    let answer = down(&mut s, near);
    assert_eq!(
        answer["color"],
        json!(hex([stored[0], stored[1], stored[2]]))
    );
    s.ok("level.set", json!({ "level": 1 }));
    let base = bytes(down(&mut s, near)["color"].as_str().unwrap_or("#000000"));
    // The middle lies on the diagonal from red to blue or from green to white, whichever the
    // quad is split along: half of each in linear light is 188 in sRGB.
    let close = |want: [u8; 3]| (0..3).all(|k| base[k].abs_diff(want[k]) <= 2);
    assert!(
        close([188, 0, 188]) || close([188, 255, 188]),
        "level 1 answers {base:?}"
    );
}
