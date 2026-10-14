//! Oracle tests: known-bad files are rejected with the error that names what is wrong and where.

use gs_io::zbp::{ZbpError, read};

use crate::sample;
use crate::writer::{
    Brush, SIGNATURE, brush_object, file, header, near_signature, object, planar,
    planar_from_strips, preamble, setting, signature, strip,
};

fn read_err(bytes: &[u8]) -> ZbpError {
    match read(bytes) {
        Ok(_) => panic!("the file should be rejected"),
        Err(e) => e,
    }
}

/// Where the brush object of `sample::bytes_of` starts, and where its body starts.
fn brush_at() -> (usize, usize) {
    let header = sample::header_1007();
    let whole = file(&header, &[]);
    (whole.len(), whole.len() + 10)
}

/// The brush with these records (the default brush has no alpha and no meshes, so records
/// start right after the 5-byte mesh area plus the 2-byte field of version 0x5A).
fn with_records(records: Vec<u8>) -> (Vec<u8>, usize) {
    let brush = Brush {
        alpha: None,
        meshes: vec![],
        records,
        ..sample::brush()
    };
    let records_at = brush_at().1 + 20 + 4 + 2 + 1;
    (sample::bytes_of(&brush), records_at)
}

fn with_alpha(planar_body: &[u8]) -> ZbpError {
    let brush = Brush {
        alpha: Some(sample::alpha_with(planar_body)),
        ..sample::brush()
    };
    read_err(&sample::bytes_of(&brush))
}

/// Offset of the planar array object inside an alpha written by `sample::alpha_with`, from the
/// start of the brush body (after the 20-byte head).
fn alpha_planar_at() -> usize {
    let (_, body) = brush_at();
    let name = 14 + 2 + "~BrushAlpha".len() + 1;
    let rect = 14 + 34;
    body + 20 + name + rect + 10 + 38
}

fn bad_image(reason: &'static str) -> ZbpError {
    ZbpError::BadImage {
        offset: alpha_planar_at(),
        reason,
    }
}

#[test]
fn not_a_brush_file() {
    assert_eq!(read_err(b""), ZbpError::NotZbp);
    assert_eq!(
        read_err(&near_signature(" and more text")),
        ZbpError::NotZbp
    );
    let mut wrong_tag = sample::full();
    wrong_tag[76] = b'Q';
    assert_eq!(read_err(&wrong_tag), ZbpError::NotZbp);
}

#[test]
fn short_preamble() {
    assert_eq!(
        read_err(&SIGNATURE),
        ZbpError::Truncated {
            offset: 0,
            what: "preamble"
        }
    );
    let mut cut = preamble(&signature(), 1008, *b"12345678");
    cut.truncate(88);
    assert_eq!(
        read_err(&cut),
        ZbpError::Truncated {
            offset: 84,
            what: "preamble"
        }
    );
}

#[test]
fn every_cut_off_file_is_rejected() {
    let full = sample::full();
    for len in 0..full.len() {
        assert!(
            read(&full[..len]).is_err(),
            "a file cut to {len} bytes must not read"
        );
    }
}

#[test]
fn broken_objects() {
    let mut bytes = sample::full();
    bytes[92 + 6..92 + 10].copy_from_slice(&13u32.to_le_bytes());
    let reason = "length shorter than the object header";
    assert_eq!(read_err(&bytes), ZbpError::BadObject { offset: 92, reason });
    let mut bytes = sample::full();
    let last = bytes.len() - 1;
    bytes[last] = 7;
    let (brush, _) = brush_at();
    assert_eq!(
        read_err(&bytes),
        ZbpError::BadObject {
            offset: brush,
            reason: "end marker missing"
        }
    );
}

#[test]
fn parts_out_of_place() {
    let mut no_icon = preamble(&signature(), 1008, *b"4.7\x00\x00\x00\x00\x00");
    no_icon.extend([0u8; 40]);
    assert_eq!(
        read_err(&no_icon),
        ZbpError::BadLayout {
            offset: 92,
            reason: "icon"
        }
    );
    let (brush, _) = brush_at();
    let header_at = brush - sample::header_1007().len();
    let reason = "expected the file header, then the brush object up to the end";
    let trailing = [sample::full(), vec![1, 2, 3]].concat();
    assert_eq!(
        read_err(&trailing),
        ZbpError::BadLayout {
            offset: header_at,
            reason
        }
    );
    let no_header = file(&[], &brush_object(&sample::brush()));
    assert_eq!(
        read_err(&no_header),
        ZbpError::BadLayout {
            offset: header_at,
            reason
        }
    );
    let short = file(
        &header(1007, 0, 0.0, 0.0, &[])[..12],
        &brush_object(&sample::brush()),
    );
    assert_eq!(
        read_err(&short),
        ZbpError::Truncated {
            offset: header_at + 12,
            what: "file header"
        }
    );
}

#[test]
fn bad_array_headers() {
    let data = [0u8; 8];
    let zero_element = planar_from_strips(8, 0, 4, &[strip(&data, 1)], &[]);
    assert_eq!(
        with_alpha(&zero_element),
        bad_image("zero element or row size")
    );
    assert_eq!(
        with_alpha(&planar(2, 0, &data, 8, &[])),
        bad_image("zero element or row size")
    );
    let rows = "size is not a whole number of rows";
    assert_eq!(with_alpha(&planar(2, 3, &[0; 6], 2, &[])), bad_image(rows));
    assert_eq!(with_alpha(&planar(2, 4, &[0; 6], 2, &[])), bad_image(rows));
}

#[test]
fn bad_strips() {
    let short = planar_from_strips(4, 1, 4, &[vec![1, 0]], &[]);
    assert_eq!(
        with_alpha(&short),
        bad_image("strip shorter than its header")
    );
    let encoding = planar_from_strips(4, 1, 4, &[vec![2, 0, 0, 0, 4, 9]], &[]);
    assert_eq!(with_alpha(&encoding), bad_image("unknown strip encoding"));
    let cut = "run-length data cut off";
    assert_eq!(
        with_alpha(&planar_from_strips(4, 1, 4, &[vec![1, 0, 0, 0, 4]], &[])),
        bad_image(cut)
    );
    assert_eq!(
        with_alpha(&planar_from_strips(
            4,
            1,
            4,
            &[vec![1, 0, 0, 0, 0xFC, 1, 2]],
            &[]
        )),
        bad_image(cut)
    );
    let odd = planar_from_strips(4, 2, 4, &[vec![1, 0, 0, 0, 3, 9]], &[]);
    assert_eq!(
        with_alpha(&odd),
        bad_image("strip is not a whole number of elements")
    );
    let mut extra = planar(1, 4, &[5; 8], 8, &[]);
    extra.push(0);
    assert_eq!(with_alpha(&extra), bad_image("data after the last strip"));
    let wrong_size = planar_from_strips(12, 1, 4, &[strip(&[5; 8], 1)], &[]);
    assert_eq!(
        with_alpha(&wrong_size),
        bad_image("decoded size differs from the stated size")
    );
}

#[test]
fn a_picture_without_pixels() {
    let brush = Brush {
        alpha: Some(sample::empty_picture()),
        ..sample::brush()
    };
    let (_, body) = brush_at();
    let container_at = body + 20 + (14 + 2 + 12) + (14 + 34);
    let reason = "image container holds no pixels";
    assert_eq!(
        read_err(&sample::bytes_of(&brush)),
        ZbpError::BadImage {
            offset: container_at,
            reason
        }
    );
}

#[test]
fn a_name_without_its_flag_bytes() {
    let mut alpha = object(12, &[0]);
    alpha.extend(sample::empty_picture().split_off(14 + 2 + 12));
    let brush = Brush {
        alpha: Some(alpha),
        ..sample::brush()
    };
    let name_at = brush_at().1 + 20;
    assert_eq!(
        read_err(&sample::bytes_of(&brush)),
        ZbpError::Truncated {
            offset: name_at,
            what: "name"
        }
    );
}

/// A setting record with `body` as its whole body.
fn record(body: &[u8]) -> Vec<u8> {
    object(5, body)
}

fn values() -> Vec<u8> {
    [
        object(1, &1f32.to_le_bytes()),
        object(1, &0f32.to_le_bytes()),
    ]
    .concat()
}

#[test]
fn bad_setting_records() {
    let head = [0x23u8, 1, 0, 0, 0, 0, 0];
    let cases: [(Vec<u8>, &str); 3] = [
        (
            [&head[..], &values(), &[0, 0], b"Name\0"].concat(),
            "record has no name",
        ),
        (
            [&head[..], &values(), &[0, 1], b"Name"].concat(),
            "name is not zero-terminated",
        ),
        (
            [&head[..], &object(1, &[0; 8]), &values(), &[0, 1, 0]].concat(),
            "value is not 4 bytes",
        ),
    ];
    for (body, reason) in cases {
        let (bytes, at) = with_records(record(&body));
        let offset = if reason.starts_with("value") {
            at + 10 + 7
        } else {
            at
        };
        assert_eq!(
            read_err(&bytes),
            ZbpError::BadSetting { offset, reason },
            "{reason}"
        );
    }
}

#[test]
fn curves_shorter_than_their_points() {
    let reason = "curve shorter than its points";
    let mut huge = vec![0u8; 24];
    huge.extend(u32::MAX.to_le_bytes());
    huge.extend([0; 32]);
    let mut overlap = vec![0u8; 24];
    overlap.extend(1u32.to_le_bytes());
    overlap.extend([0; 8]);
    for body in [huge, overlap] {
        let c = object(10, &body);
        let (bytes, at) = with_records(setting(0x29, 0, 0.0, "Size", Some(&c)));
        let curve_at = at + 10 + 7 + 18 + 18 + 1;
        assert_eq!(
            read_err(&bytes),
            ZbpError::BadSetting {
                offset: curve_at,
                reason
            }
        );
    }
}

#[test]
fn messages_name_the_problem_and_the_place() {
    let cases = [
        (
            ZbpError::NotZbp,
            "not a brush file: the signature is missing",
        ),
        (
            ZbpError::Truncated {
                offset: 5,
                what: "x",
            },
            "file ends at byte 5 in the middle of the x",
        ),
        (
            ZbpError::BadObject {
                offset: 6,
                reason: "y",
            },
            "broken object at byte 6: y",
        ),
        (
            ZbpError::BadLayout {
                offset: 7,
                reason: "z",
            },
            "unexpected layout at byte 7: z",
        ),
        (
            ZbpError::BadImage {
                offset: 8,
                reason: "w",
            },
            "bad image at byte 8: w",
        ),
        (
            ZbpError::BadSetting {
                offset: 9,
                reason: "v",
            },
            "bad setting at byte 9: v",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
    }
}
