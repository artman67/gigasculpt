//! A complete synthetic brush using every part the reader knows, and the values it was built
//! from, so tests can change one part and check the rest.

use crate::writer::{
    Brush, alpha_pixels, brush_object, container, curve, file, header, le16, object, picture,
    planar, setting,
};

pub const ALPHA_W: usize = 40;
pub const ALPHA_H: usize = 30;

/// Plain fields before the first setting and between the groups.
pub const TAIL: [u8; 16] = [
    0, 0, 0, 0, 0, 0x24, 0x24, 0, 0, 0, 0x80, 0x3F, 0, 0, 0x80, 0x3E,
];
pub const GAP0: [u8; 12] = [0, 0, 0, 0, 0, 0, 0x80, 0x3F, 0, 0, 0, 0];
pub const GAP1: [u8; 4] = [1, 0, 0, 0];
pub const GAP2: [u8; 5] = [1, 0, 0, 0, 1];
pub const LAST: [u8; 8] = [2, 0, 0, 0, 5, 0, 0, 0];

pub const EDIT_CURVE: [(f32, f32, u32, f32); 3] =
    [(0.0, 0.0, 1, 1.0), (0.9, 0.81, 5, 1.0), (1.0, 1.0, 1, 0.5)];

pub fn alpha_planar() -> Vec<u8> {
    let pixels = le16(&alpha_pixels(ALPHA_W, ALPHA_H));
    planar(
        2,
        u16::try_from(ALPHA_W * 2).unwrap(),
        &pixels,
        1000,
        &[777, 0, 5],
    )
}

/// A texture three objects down inside a section, the way newer files nest it.
pub fn section_body() -> Vec<u8> {
    let texture = planar(2, 6, &le16(&[1, 2, 3, 4, 5, 6]), 1 << 17, &[]);
    let wrapper = object(4, &[&[9u8; 18][..], &object(6, &texture)].concat());
    let inner = object(
        10,
        &[&[0u8; 37][..], &object(13, &[0, 0]), &wrapper].concat(),
    );
    [&[2u8, 0, 0, 0, 0][..], &inner, &[0]].concat()
}

/// Four groups of settings with plain fields between them, and a section after the third.
pub fn records() -> Vec<u8> {
    let edit = curve(10, 32, &EDIT_CURVE);
    [
        TAIL.to_vec(),
        setting(0x23, 0x08, 0.25, "Imbed", None),
        setting(0x81, 0x09, 0.0, "Buildup", None),
        GAP0.to_vec(),
        setting(0x29, 0x00, 0.0, "Edit Curve", Some(&edit)),
        setting(0x23, 0x00, 7.0, "Intensity", None),
        GAP1.to_vec(),
        setting(0x23, 0x01, 4.0, "Smooth", None),
        GAP2.to_vec(),
        object(0x0E, &section_body()),
        setting(0x81, 0x01, 0.0, "Intensity", None),
        setting(0x81, 0x00, 0.0, "Size", None),
        setting(0x55, 0x00, -3.5, "Caf\u{e9}", None),
        LAST.to_vec(),
    ]
    .concat()
}

pub fn brush() -> Brush {
    Brush {
        version: 0x5A,
        draw_size: 64.0,
        z: 0.25,
        rgb: 0.75,
        unknown: -0.5,
        alpha_flags: 0x42,
        alpha: Some(picture("~BrushAlpha", &alpha_planar())),
        meshes: vec![
            ("first", vec![1, 2, 3], vec![4]),
            ("second", vec![], vec![5, 6]),
        ],
        records: records(),
    }
}

pub fn header_1007() -> Vec<u8> {
    header(
        1007,
        10,
        0.25,
        0.75,
        &["Someone", "", "https://example.org"],
    )
}

pub fn bytes_of(brush: &Brush) -> Vec<u8> {
    file(&header_1007(), &brush_object(brush))
}

pub fn full() -> Vec<u8> {
    bytes_of(&brush())
}

/// A picture whose container holds no planar array.
pub fn empty_picture() -> Vec<u8> {
    let empty = object(10, &[0u8; 38]);
    [
        crate::writer::name_object("~BrushAlpha"),
        crate::writer::rect_object(),
        empty,
    ]
    .concat()
}

/// A container with `planar_body` as its planar array.
pub fn alpha_with(planar_body: &[u8]) -> Vec<u8> {
    [
        crate::writer::name_object("~BrushAlpha"),
        crate::writer::rect_object(),
        container(planar_body),
    ]
    .concat()
}
