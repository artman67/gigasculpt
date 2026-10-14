//! Differential tests: files written by our own writer from the format notes, read back field
//! by field.

use gs_io::zbp::{Block, CurvePoint, Image, SettingKind, ZbpBrush, read};

use crate::sample::{self, ALPHA_H, ALPHA_W, EDIT_CURVE};
use crate::writer::{
    Brush, alpha_pixels, brush_object, copyright, curve, file, header, icon_pixels, le16, object,
    picture, planar, planar_from_strips, setting,
};

fn read_ok(bytes: &[u8]) -> ZbpBrush {
    read(bytes).unwrap_or_else(|e| panic!("synthetic file should read: {e}"))
}

#[test]
fn reads_the_preamble_icon_and_header() {
    let b = read_ok(&sample::full());
    assert_eq!(b.preamble.copyright, copyright());
    assert_eq!(b.preamble.format, 1008);
    assert_eq!(b.preamble.writer, "4.7");
    assert_eq!(b.icon.name, "Image");
    let icon = Image {
        width: 4,
        height: 4,
        bytes_per_pixel: 4,
        data: icon_pixels(),
    };
    assert_eq!(b.icon.image, icon);
    assert_eq!(b.header.version, 1007);
    assert_eq!(b.header.base_type, 10);
    assert_eq!((b.header.z_intensity, b.header.rgb_intensity), (0.25, 0.75));
    assert_eq!(b.header.credits, ["Someone", "https://example.org"]);
    assert_eq!(b.header.raw, sample::header_1007());
}

#[test]
fn reads_the_brush_head_alpha_and_meshes() {
    let b = read_ok(&sample::full());
    assert_eq!(b.head.version, 0x5A);
    assert_eq!(b.head.draw_size, Some(64.0));
    assert_eq!(
        (b.head.z_intensity, b.head.rgb_intensity, b.head.unknown),
        (0.25, 0.75, -0.5)
    );
    assert_eq!(b.head.alpha_flags, 0x42);
    let alpha = b.alpha.expect("the sample has an alpha");
    assert_eq!(alpha.name, "~BrushAlpha");
    assert_eq!(
        (
            alpha.image.width,
            alpha.image.height,
            alpha.image.bytes_per_pixel
        ),
        (40, 30, 2)
    );
    assert_eq!(
        alpha.image.gray16(),
        Some(alpha_pixels(ALPHA_W, ALPHA_H)),
        "alpha pixels round-trip"
    );
    let meshes: Vec<_> = b
        .meshes
        .iter()
        .map(|m| (m.tag, m.name.as_str(), m.mesh.version, m.companion.version))
        .collect();
    assert_eq!(
        meshes,
        [(2, "first", 0x59, 0x13), (2, "second", 0x59, 0x13)]
    );
    assert_eq!(
        (
            b.meshes[0].mesh.bytes.as_slice(),
            b.meshes[0].companion.bytes.as_slice()
        ),
        (&[1, 2, 3][..], &[4][..])
    );
    assert_eq!(
        (
            b.meshes[1].mesh.bytes.as_slice(),
            b.meshes[1].companion.bytes.as_slice()
        ),
        (&[][..], &[5, 6][..])
    );
}

#[test]
fn reads_settings_in_groups_with_their_state() {
    let b = read_ok(&sample::full());
    let got: Vec<_> = b
        .settings
        .iter()
        .map(|s| (s.group, s.name.as_str(), s.kind, s.on, s.value))
        .collect();
    let want = [
        (0, "Imbed", SettingKind::Slider, false, 0.25),
        (0, "Buildup", SettingKind::Switch, true, 0.0),
        (1, "Edit Curve", SettingKind::Curve, false, 0.0),
        (1, "Intensity", SettingKind::Slider, false, 7.0),
        (2, "Smooth", SettingKind::Slider, true, 4.0),
        (3, "Intensity", SettingKind::Switch, true, 0.0),
        (3, "Size", SettingKind::Switch, false, 0.0),
        (3, "Caf\u{e9}", SettingKind::Other(0x55), false, -3.5),
    ];
    assert_eq!(got, want);
    assert!(
        b.settings
            .iter()
            .all(|s| s.second.to_bits() == 0.5f32.to_bits()),
        "second values are kept"
    );
    assert_eq!(b.settings[0].head, [0x23, 0x01, 0x08, 0, 2, 0, 3]);
    let points: Vec<_> = EDIT_CURVE
        .iter()
        .map(|&(x, y, kind, weight)| CurvePoint { x, y, kind, weight })
        .collect();
    let edit = b.settings[2]
        .curve
        .as_ref()
        .expect("Edit Curve has a curve");
    assert_eq!(
        (edit.version, edit.head.len(), &edit.points),
        (10, 60, &points)
    );
    assert!(
        b.settings
            .iter()
            .filter(|s| s.kind != SettingKind::Curve)
            .all(|s| s.curve.is_none())
    );
}

#[test]
fn keeps_plain_fields_and_sections_in_place() {
    let b = read_ok(&sample::full());
    let blocks = [
        Block {
            after: 0,
            bytes: sample::TAIL.to_vec(),
        },
        Block {
            after: 2,
            bytes: sample::GAP0.to_vec(),
        },
        Block {
            after: 4,
            bytes: sample::GAP1.to_vec(),
        },
        Block {
            after: 5,
            bytes: sample::GAP2.to_vec(),
        },
        Block {
            after: 8,
            bytes: sample::LAST.to_vec(),
        },
    ];
    assert_eq!(b.blocks, blocks);
    assert_eq!(b.sections.len(), 1);
    let section = &b.sections[0];
    assert_eq!((section.after, section.object.version), (5, 0x0E));
    assert_eq!(section.object.bytes, sample::section_body());
    let texture = Image {
        width: 3,
        height: 2,
        bytes_per_pixel: 2,
        data: le16(&[1, 2, 3, 4, 5, 6]),
    };
    assert_eq!(section.images, [texture]);
}

/// Header versions, brush versions at the boundaries where fields appear, with and without an
/// alpha and meshes.
#[test]
fn older_and_unknown_layouts() {
    let cases = [
        (1002, 0x41, false, false, 0),
        (1005, 0x4B, true, false, 2),
        (1006, 0x4C, false, true, 2),
        (1007, 0x59, true, true, 2),
        (1099, 0x5A, true, true, 0),
    ];
    for (header_version, brush_version, alpha, meshes, credits) in cases {
        let mut brush = Brush {
            version: brush_version,
            ..sample::brush()
        };
        if !alpha {
            brush.alpha = None;
        }
        if !meshes {
            brush.meshes.clear();
        }
        let bytes = file(
            &header(header_version, 3, 0.5, 1.0, &["A", "B"]),
            &brush_object(&brush),
        );
        let b = read_ok(&bytes);
        let case = format!("header {header_version}, brush {brush_version:#x}");
        assert_eq!(
            (b.header.version, b.header.base_type, b.header.z_intensity),
            (header_version, 3, 0.5),
            "{case}"
        );
        assert_eq!(b.header.credits.len(), credits, "{case}");
        assert_eq!(b.head.draw_size.is_some(), brush_version >= 0x4C, "{case}");
        assert_eq!(
            (b.head.z_intensity, b.head.rgb_intensity),
            (0.25, 0.75),
            "{case}"
        );
        assert_eq!(
            (b.alpha.is_some(), b.meshes.len()),
            (alpha, if meshes { 2 } else { 0 }),
            "{case}"
        );
        assert_eq!(b.settings.len(), 8, "{case}");
    }
}

fn alpha_image(planar_body: &[u8]) -> Image {
    let brush = Brush {
        alpha: Some(picture("~BrushAlpha", planar_body)),
        ..sample::brush()
    };
    read_ok(&sample::bytes_of(&brush))
        .alpha
        .expect("alpha present")
        .image
}

#[test]
fn images_of_one_two_and_four_bytes_per_pixel() {
    let grey8: Vec<u8> = (0..60u8).map(|i| i / 7).collect();
    let one = alpha_image(&planar(1, 10, &grey8, 16, &[]));
    assert_eq!(
        one,
        Image {
            width: 10,
            height: 6,
            bytes_per_pixel: 1,
            data: grey8
        }
    );
    assert_eq!(one.gray16(), None);
    let rgba: Vec<u8> = (0..48u8).collect();
    let four = alpha_image(&planar(4, 8, &rgba, 16, &[3]));
    assert_eq!(
        four,
        Image {
            width: 2,
            height: 6,
            bytes_per_pixel: 4,
            data: rgba
        }
    );
    assert_eq!(four.gray16(), None);
    let two = alpha_image(&planar(2, 4, &le16(&[0x0102, 0xFFFE, 7, 0x8000]), 4, &[]));
    assert_eq!(two.gray16(), Some(vec![0x0102, 0xFFFE, 7, 0x8000]));
}

/// Run-length edges written by hand: a zero-length run, the longest run, the longest literal.
#[test]
fn run_length_edges() {
    let mut packed = vec![0x00, 0xAA, 127, 5];
    packed.push(128);
    packed.extend((0..128u8).map(|i| i ^ 0x5A));
    packed.extend([0xFF, 9]);
    let strip = [&[1, 0, 0, 0][..], &packed].concat();
    let mut want = vec![5u8; 127];
    want.extend((0..128u8).map(|i| i ^ 0x5A));
    want.push(9);
    let image = alpha_image(&planar_from_strips(256, 1, 16, &[strip], &[]));
    assert_eq!(image.data, want);
}

#[test]
fn curves_of_both_versions_and_a_bare_header() {
    let pts = [(0.0, 0.25, 0, 1.0), (1.0, 0.75, 4, 1.0)];
    for (version, middle) in [(9, 28), (10, 32), (10, 0)] {
        let c = curve(version, middle, &pts);
        let records = [setting(0x29, 0, 0.0, "Size", Some(&c)), vec![0; 4]].concat();
        let b = read_ok(&sample::bytes_of(&Brush {
            records,
            ..sample::brush()
        }));
        let got = b.settings[0].curve.clone().expect("curve");
        assert_eq!((got.version, got.head.len()), (version, 28 + middle));
        let xs: Vec<_> = got
            .points
            .iter()
            .map(|p| (p.x, p.y, p.kind, p.weight))
            .collect();
        assert_eq!(xs, pts);
    }
}

/// Images are looked for four objects deep inside a section and no deeper.
#[test]
fn images_nested_four_deep_are_found_five_are_not() {
    let texture = object(6, &planar(1, 2, &[1, 2, 3, 4], 1 << 17, &[]));
    let nest = |levels: usize| (0..levels).fold(texture.clone(), |inner, _| object(20, &inner));
    for (levels, found) in [(3, 1), (4, 0)] {
        let records = [
            vec![0; 4],
            setting(0x23, 0, 1.0, "A", None),
            object(0x0E, &nest(levels)),
        ]
        .concat();
        let b = read_ok(&sample::bytes_of(&Brush {
            records,
            ..sample::brush()
        }));
        assert_eq!(
            b.sections[0].images.len(),
            found,
            "texture inside {levels} wrappers plus the section"
        );
    }
}

#[test]
fn an_object_with_an_empty_body_is_kept() {
    let records = [setting(0x23, 0, 1.0, "A", None), object(0x30, &[])].concat();
    let b = read_ok(&sample::bytes_of(&Brush {
        records,
        ..sample::brush()
    }));
    assert_eq!(
        (
            b.sections[0].object.version,
            b.sections[0].object.bytes.len()
        ),
        (0x30, 0)
    );
    assert!(b.blocks.is_empty(), "no plain fields were written");
}
