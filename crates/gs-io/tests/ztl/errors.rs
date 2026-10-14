//! Oracle tests: known-bad tool and project files are rejected with the error that names what
//! is wrong and where.

use gs_io::ReadError;
use gs_io::ztl::read;

use crate::brush::{near_signature, planar_from_strips};
use crate::sample::{self, MESH, REST_11};
use crate::writer::{
    RECORD_TAIL, absent, array_slot, array_slot_from, companion_object, entry, faces, file, item,
    mesh_object, name_object, object, planar, points, project_object, record, tool_object,
    tools_section,
};

fn read_err(bytes: &[u8]) -> ReadError {
    match read(bytes) {
        Ok(_) => panic!("the file should be rejected"),
        Err(e) => e,
    }
}

/// Offset of the first copy of `part` in `bytes`.
fn at(bytes: &[u8], part: &[u8]) -> usize {
    bytes
        .windows(part.len())
        .position(|w| w == part)
        .expect("the part is in the file")
}

fn layout(offset: usize, reason: &'static str) -> ReadError {
    ReadError::BadLayout { offset, reason }
}

fn mesh_err(offset: usize, reason: &'static str) -> ReadError {
    ReadError::BadMesh { offset, reason }
}

/// A tool file whose selected part has this mesh and companion (and the usual list).
fn body_file(mesh: &[u8], companion: &[u8]) -> Vec<u8> {
    let list = sample::list(&sample::hat_record(&RECORD_TAIL));
    let rec = record(11, 1, 2, mesh, Some(companion), &list);
    file(b"LTZ\0", &sample::tool_with(&rec))
}

/// The body's companion with these slots.
fn companion(level: u32, base: u32, slots: &[Vec<u8>]) -> Vec<u8> {
    companion_object(18, level, base, slots, None)
}

/// The body's mesh with these slots.
fn mesh(slots: &[Vec<u8>]) -> Vec<u8> {
    mesh_object(0x52, 384, slots, &[])
}

fn positions(n: usize) -> Vec<u8> {
    array_slot(12, &points(&sample::stored_points(n, 0.0)), 1 << 17)
}

#[test]
fn not_a_tool_or_project_file() {
    assert_eq!(read_err(b""), ReadError::NotZtl);
    assert_eq!(read_err(&near_signature(" and more")), ReadError::NotZtl);
    let mut brush = sample::tool_file();
    brush[76..80].copy_from_slice(b"PBZ\0");
    assert_eq!(read_err(&brush), ReadError::NotZtl);
    let cut = &sample::tool_file()[..82];
    assert_eq!(
        read_err(cut),
        ReadError::Truncated {
            offset: 80,
            what: "preamble"
        }
    );
}

#[test]
fn every_cut_off_file_is_rejected() {
    for bytes in [sample::tool_file(), sample::project_file()] {
        for len in 0..bytes.len() {
            assert!(
                read(&bytes[..len]).is_err(),
                "a file cut at {len} must not read"
            );
        }
    }
}

#[test]
fn a_tool_file_needs_a_tool_object() {
    let main = object(1003, &[0; 20]);
    let bytes = file(b"LTZ\0", &main);
    let reason = "a tool file holds a tool object after the icon";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &main), reason));
}

#[test]
fn a_tool_holds_name_settings_and_one_record() {
    let short = object(1004, &[&[0u8; 54][..], &name_object("Body")].concat());
    let bytes = file(b"LTZ\0", &short);
    let reason = "a tool holds plain fields, then name, settings and record objects";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &short), reason));

    let mut long = sample::tool();
    let body_len = long.len() - 14;
    long.truncate(long.len() - 4 - 2);
    long.extend(object(1, &[0; 4]));
    long.extend([3, 0x88, 0, 0]);
    let len = u32::try_from(body_len - 2 + 18 + 14).unwrap();
    long[6..10].copy_from_slice(&len.to_le_bytes());
    let bytes = file(b"LTZ\0", &long);
    let reason = "a tool ends after its first record";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &long), reason));
}

#[test]
fn records_need_a_known_version_and_a_mesh() {
    let list = sample::list(&sample::hat_record(&RECORD_TAIL));
    let rec = record(
        12,
        1,
        2,
        &sample::body_mesh(384),
        Some(&sample::body_companion()),
        &list,
    );
    let tool = sample::tool_with(&rec);
    let bytes = file(b"LTZ\0", &tool);
    assert_eq!(
        read_err(&bytes),
        layout(at(&bytes, &tool), "unknown part record version")
    );

    let bare = object(
        11,
        &[&[0u8; 25][..], &object(13, &[0, 0]), &object(1, &[0; 34])].concat(),
    );
    let bytes = file(b"LTZ\0", &sample::tool_with(&bare));
    let reason = "a record holds plain fields, two small objects and a mesh";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &bare), reason));
}

#[test]
fn the_list_has_one_entry_without_a_record() {
    let cases: [(Vec<u8>, &str); 3] = [
        (
            [
                RECORD_TAIL.to_vec(),
                item(0, MESH),
                entry(0x11, [0; 4], "A", "", REST_11),
                item(0, MESH),
                entry(0x11, [0; 4], "B", "", REST_11),
            ]
            .concat(),
            "two list entries without a record",
        ),
        (RECORD_TAIL.to_vec(), "no list entry for the selected part"),
        (
            [
                RECORD_TAIL.to_vec(),
                item(0, MESH),
                name_object("A"),
                sample::hat_settings(),
                entry(0x11, [0; 4], "A", "", REST_11),
            ]
            .concat(),
            "a part in the list has a name, settings and record",
        ),
    ];
    for (after, reason) in cases {
        let rec = sample::body_record(384, &after);
        let bytes = file(b"LTZ\0", &sample::tool_with(&rec));
        let offset = if reason.starts_with("no list") {
            at(&bytes, &sample::tool_with(&rec))
        } else {
            at(&bytes, &rec)
        };
        assert_eq!(read_err(&bytes), layout(offset, reason), "{reason}");
    }
}

#[test]
fn only_the_selected_record_lists_other_parts() {
    let inner = [
        RECORD_TAIL.to_vec(),
        item(0, MESH),
        name_object("X"),
        sample::hat_settings(),
        sample::hat_record(&RECORD_TAIL),
        entry(0x11, [0; 4], "X", "", REST_11),
    ]
    .concat();
    let hat = sample::hat_record(&inner);
    let bytes = file(
        b"LTZ\0",
        &sample::tool_with(&sample::body_record(384, &sample::list(&hat))),
    );
    let reason = "a part's own list holds other parts";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &hat), reason));
}

#[test]
fn list_entries_cut_short() {
    let no_zero = [
        RECORD_TAIL.to_vec(),
        item(0, MESH),
        vec![0x11, 0, 0, 0, 0, 0, 0, 0, b'A'],
    ]
    .concat();
    let rec = sample::body_record(384, &no_zero);
    let bytes = file(b"LTZ\0", &sample::tool_with(&rec));
    let name_at = at(&bytes, &rec) + rec.len() - 4 - 1;
    assert_eq!(
        read_err(&bytes),
        ReadError::Truncated {
            offset: name_at,
            what: "list entry name"
        }
    );
    let short = [
        RECORD_TAIL.to_vec(),
        item(0, MESH),
        entry(0x11, [0; 4], "A", "", REST_11 - 1),
    ]
    .concat();
    let rec = sample::body_record(384, &short);
    let bytes = file(b"LTZ\0", &sample::tool_with(&rec));
    let rest_at = at(&bytes, &rec) + rec.len() - 4 - (REST_11 - 1);
    assert_eq!(
        read_err(&bytes),
        ReadError::Truncated {
            offset: rest_at,
            what: "list entry"
        }
    );
}

#[test]
fn positions_must_match_a_level() {
    let faces3 = array_slot(16, &faces(&sample::BODY_FACES), 1 << 17);
    let cases: [(Vec<u8>, Vec<u8>, &str); 5] = [
        (
            mesh(&[positions(7)]),
            companion(0, u32::MAX, std::slice::from_ref(&faces3)),
            "a stored level has fewer vertices than the base",
        ),
        (
            mesh(&[positions(8)]),
            companion(0, 7, std::slice::from_ref(&faces3)),
            "the positions match no subdivision level",
        ),
        (
            mesh(&[positions(7)]),
            companion(1, 7, std::slice::from_ref(&faces3)),
            "the positions do not match the current level",
        ),
        (
            mesh(&[positions(7), absent(), absent(), positions(7)]),
            companion(0, 7, std::slice::from_ref(&faces3)),
            "the positions match no subdivision level",
        ),
        (
            mesh(&[positions(7)]),
            companion(0, 7, &[array_slot(16, &faces(&[[0, 1, 2, 9]]), 1 << 17)]),
            "a face refers to a vertex past the base vertex count",
        ),
    ];
    for (m, c, reason) in cases {
        let bytes = body_file(&m, &c);
        assert_eq!(
            read_err(&bytes),
            mesh_err(at(&bytes, &c), reason),
            "{reason}"
        );
    }
}

#[test]
fn arrays_must_fit_the_mesh() {
    let faces3 = array_slot(16, &faces(&sample::BODY_FACES), 1 << 17);
    let two_uvs = array_slot(32, &[0; 64], 1 << 17);
    let wrong_data = array_slot(8, &[0; 48], 1 << 17);
    let wrong_element = array_slot(4, &points(&sample::stored_points(7, 0.0)), 1 << 17);
    let reason = "an array's size does not match the mesh";
    let cases: [(Vec<u8>, Vec<u8>, Vec<u8>); 3] = [
        (
            mesh(&[positions(7)]),
            companion(0, 7, &[faces3.clone(), absent(), absent(), two_uvs.clone()]),
            two_uvs,
        ),
        (
            mesh(&[positions(7), wrong_data.clone()]),
            companion(0, 7, std::slice::from_ref(&faces3)),
            wrong_data,
        ),
        (
            mesh(std::slice::from_ref(&wrong_element)),
            companion(0, 7, std::slice::from_ref(&faces3)),
            wrong_element,
        ),
    ];
    for (m, c, bad) in cases {
        let bytes = body_file(&m, &c);
        let planar_at = at(&bytes, &bad) + 1 + 16 + 10 + 18;
        assert_eq!(read_err(&bytes), mesh_err(planar_at, reason));
    }
    let empty = mesh(&[vec![1]]);
    let c = companion(0, 7, &[faces3]);
    let bytes = body_file(&empty, &c);
    assert!(matches!(
        read_err(&bytes),
        ReadError::BadMesh {
            reason: "an array slot holds no array",
            ..
        }
    ));
}

#[test]
fn mesh_objects_have_their_fixed_parts() {
    let no_settings = object(0x52, &[&[0u8; 384][..], &positions(7)].concat());
    let c = sample::body_companion();
    let bytes = body_file(&no_settings, &c);
    let reason = "a mesh holds plain fields and a settings object first";
    assert_eq!(read_err(&bytes), mesh_err(at(&bytes, &no_settings), reason));

    let mut no_tail = sample::body_companion();
    let cut = no_tail.len() - 4 - 100;
    no_tail.drain(cut..no_tail.len() - 4);
    let len = u32::try_from(no_tail.len()).unwrap();
    no_tail[6..10].copy_from_slice(&len.to_le_bytes());
    let bytes = body_file(&sample::body_mesh(384), &no_tail);
    let reason = "a companion ends with plain fields";
    assert_eq!(read_err(&bytes), mesh_err(at(&bytes, &no_tail), reason));
}

/// The error expected for a planar array whose body starts at the given offset.
type Expected = Box<dyn Fn(usize) -> ReadError>;

#[test]
fn streamed_arrays_check_their_header_and_strips() {
    let data = points(&sample::stored_points(7, 0.0));
    let good = planar(12, 12, &data, 1 << 17, &[]);
    let strip = crate::brush::strip(&data, 12);
    // Offsets below count from the planar array's body.
    let strips_at = 20 + 2 * 4;
    let mut long_table = planar_from_strips(84, 12, 12, std::slice::from_ref(&strip), &[]);
    long_table[24..28].copy_from_slice(&40u32.to_le_bytes());
    let mut long_count = good.clone();
    long_count[16..20].copy_from_slice(&1000u32.to_le_bytes());
    let truncated = |at: usize, what| {
        move |body: usize| ReadError::Truncated {
            offset: body + at,
            what,
        }
    };
    let image = |reason| {
        move |body: usize| ReadError::BadImage {
            offset: body - 10,
            reason,
        }
    };
    let cases: [(Vec<u8>, Expected); 6] = [
        (good[..10].to_vec(), Box::new(truncated(8, "array header"))),
        (long_count, Box::new(truncated(20, "strip table"))),
        (
            long_table,
            Box::new(truncated(strips_at + strip.len(), "array strip")),
        ),
        (
            [good.clone(), vec![0, 0]].concat(),
            Box::new(image("data after the last strip")),
        ),
        (
            planar_from_strips(96, 12, 12, std::slice::from_ref(&strip), &[]),
            Box::new(image("decoded size differs from the stated size")),
        ),
        (
            planar_from_strips(72, 12, 12, std::slice::from_ref(&strip), &[]),
            Box::new(image("decoded size differs from the stated size")),
        ),
    ];
    let c = companion(
        0,
        7,
        &[array_slot(16, &faces(&sample::BODY_FACES), 1 << 17)],
    );
    for (i, (body, expected)) in cases.into_iter().enumerate() {
        let slot = array_slot_from(12, data.len(), &body);
        let bytes = body_file(&mesh(std::slice::from_ref(&slot)), &c);
        let body_at = at(&bytes, &slot) + 1 + 16 + 10 + 18 + 10;
        assert_eq!(read_err(&bytes), expected(body_at), "case {i}");
    }
}

#[test]
fn project_sections_have_tags() {
    let mut body = vec![0u8; 16];
    body.extend(0x800u32.to_le_bytes());
    body.extend(object(1, &[1; 4]));
    let untagged = tools_section(&[(1, sample::tool())]);
    body.extend(&untagged);
    body.extend(0u32.to_le_bytes());
    let bytes = file(b"RPZ\0", &object(14, &body));
    let reason = "a project section has a tag before it";
    assert_eq!(read_err(&bytes), layout(at(&bytes, &untagged), reason));
    let tool = tool_object(
        MESH,
        "Body",
        &sample::body_settings(),
        &sample::body_record(384, &RECORD_TAIL),
    );
    let bytes = file(
        b"RPZ\0",
        &project_object(14, &[(2, tools_section(&[(1, tool)]))]),
    );
    assert!(matches!(
        read_err(&bytes),
        ReadError::BadLayout {
            reason: "no list entry for the selected part",
            ..
        }
    ));
}

#[test]
fn messages_name_the_problem_and_the_place() {
    let cases = [
        (
            ReadError::NotZtl,
            "not a tool or project file: the signature is missing",
        ),
        (
            ReadError::BadMesh {
                offset: 4,
                reason: "u",
            },
            "bad mesh at byte 4: u",
        ),
        (
            ReadError::Io {
                offset: 3,
                kind: std::io::ErrorKind::UnexpectedEof,
            },
            "could not read byte 3: unexpected end of file",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
    }
}

#[test]
fn an_object_running_past_its_record_is_cut_off_where_it_starts() {
    let mut value = object(1, &3f32.to_le_bytes());
    value[6..10].copy_from_slice(&100u32.to_le_bytes());
    let body = [
        vec![0x23, 1, 8, 0, 0, 0, 0],
        value.clone(),
        object(1, &0f32.to_le_bytes()),
        vec![0, 1, b'X', 0],
    ]
    .concat();
    let settings = crate::writer::settings_object(&[0x0B], &[object(5, &body)]);
    let record = sample::body_record(384, &sample::list(&sample::hat_record(&RECORD_TAIL)));
    let bytes = file(b"LTZ\0", &tool_object(MESH, "Body", &settings, &record));
    assert_eq!(
        read_err(&bytes),
        ReadError::Truncated {
            offset: at(&bytes, &value),
            what: "setting value"
        }
    );
}
