//! Synthetic tool and project files using every part the reader knows, and the values they
//! were built from, so tests can change one part and check the rest.

use gs_protocol::NO_VERTEX;

use crate::writer::{
    RECORD_TAIL, absent, array_slot, companion_object, entry, faces, file, item, mesh_object,
    name_object, object, points, project_object, record, setting, settings_object, tool_object,
    tools_section, unnamed_setting,
};

/// Type number of a polygon mesh part.
pub const MESH: u32 = 11;
/// Fixed fields after an entry's names in a version 11 record.
pub const REST_11: usize = 4;
/// The export scale and offset of the selected part.
pub const SCALE: [f32; 3] = [2.0, 2.0, 2.0];
pub const OFFSET: [f32; 3] = [1.0, 0.0, -1.0];

/// The selected part's base: two quads sharing an edge and a triangle.
pub const BODY_FACES: [[u32; 4]; 3] = [[0, 1, 2, 3], [1, 4, 5, 2], [4, 6, 5, NO_VERTEX]];
pub const BODY_GROUPS: [u32; 3] = [1, 2, 2];
/// Face data: the groups with flag bits above them.
pub const BODY_FACE_DATA: [u32; 3] = [1 | 0x0001_0000, 2, 2 | 0x0040_0000];
/// Vertex count of the body's first level (7 base vertices, 9 edges, 3 faces).
pub const BODY_LEVEL1: usize = 19;

/// Stored positions (Y down, Z toward the back) of `n` points.
pub fn stored_points(n: usize, seed: f32) -> Vec<[f32; 3]> {
    (0..n)
        .map(|i| {
            let i = i as f32;
            [i + seed, 2.0 * i - seed, 0.5 - i]
        })
        .collect()
}

/// Per vertex data of the body's base: vertex 0 colored (stored blue, green, red), vertex 2
/// half masked, vertex 3 fully masked.
pub fn body_vertex_data() -> Vec<[u8; 8]> {
    (0..7u8)
        .map(|i| match i {
            0 => [1, 0, 0xFF, 0xFF, 30, 20, 10, 0],
            2 => [1, 0, 0x00, 0x80, 0, 0, 0, 0],
            3 => [1, 0, 0, 0, 0, 0, 0, 0],
            _ => [1, i, 0xFF, 0xFF, 0, 0, 0, 0],
        })
        .collect()
}

pub fn body_uvs() -> Vec<[f32; 8]> {
    (0..3)
        .map(|f| {
            let f = f as f32;
            [f, 0.1, f, 0.2, f, 0.3, f, 0.4]
        })
        .collect()
}

fn floats(rows: &[[f32; 8]]) -> Vec<u8> {
    rows.iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The selected part's mesh object: base positions and data, an absent slot, the first
/// level's positions and data, then plain fields, a slot holding a layer-like object and a
/// plain byte that is not a presence byte.
pub fn body_mesh(head_len: usize) -> Vec<u8> {
    let level1_data: Vec<u8> = vec![[1u8, 1, 0xFF, 0xFF, 0, 0, 0, 0]; BODY_LEVEL1]
        .into_iter()
        .flatten()
        .collect();
    let slots = [
        array_slot(12, &points(&stored_points(7, 0.25)), 36),
        array_slot(8, &body_vertex_data().concat(), 1 << 17),
        absent(),
        array_slot(12, &points(&stored_points(BODY_LEVEL1, 0.5)), 96),
        array_slot(8, &level1_data, 1 << 17),
        absent(),
        absent(),
        absent(),
        absent(),
        [vec![1], object(13, &[0, 0]), object(3, &[7; 12])].concat(),
    ];
    mesh_object(0x52, head_len, &slots, &[0, 0, 0x80, 0x3F, 0, 0])
}

pub fn body_companion() -> Vec<u8> {
    let slots = [
        array_slot(16, &faces(&BODY_FACES), 1 << 17),
        absent(),
        absent(),
        array_slot(32, &floats(&body_uvs()), 1 << 17),
        array_slot(4, &words(&BODY_FACE_DATA), 1 << 17),
        array_slot(4, &words(&[9, 9, 9]), 1 << 17),
        absent(),
    ];
    companion_object(18, 0, 7, &slots, Some((SCALE, OFFSET)))
}

/// Vertex counts of the hat's level 1 and level 3 (one quad: 4, 9, 25, 81).
pub const HAT_LEVEL1: usize = 9;
pub const HAT_LEVEL3: usize = 81;
pub const HAT_GROUP: u32 = 5;

/// The other part: one quad saved at level 1 with a top level 3, no per vertex data, one
/// face group.
pub fn hat_mesh() -> Vec<u8> {
    let slots = [
        array_slot(12, &points(&stored_points(HAT_LEVEL1, -1.0)), 1 << 17),
        absent(),
        absent(),
        array_slot(12, &points(&stored_points(HAT_LEVEL3, 2.0)), 1 << 17),
        absent(),
    ];
    mesh_object(0x4A, 384, &slots, &[0; 20])
}

pub fn hat_companion() -> Vec<u8> {
    let slots = [
        array_slot(16, &faces(&[[0, 1, 2, 3]]), 1 << 17),
        absent(),
        array_slot(4, &HAT_GROUP.to_le_bytes(), 1 << 17),
    ];
    companion_object(17, 1, 4, &slots, None)
}

pub fn body_settings() -> Vec<u8> {
    settings_object(
        &[0x0B, 0, 0, 0, 0x0A, 0, 0, 0],
        &[
            setting(0x23, 0x08, 3.0, "UndoCounter", None),
            unnamed_setting(0x25, 0.0),
            object(9, &[4; 8]),
            setting(0x81, 0x09, 0.0, "Body", None),
        ],
    )
}

pub fn hat_settings() -> Vec<u8> {
    settings_object(&[0x0B], &[setting(0x81, 0x08, 0.0, "Visible Count", None)])
}

/// The hat's record; `after` follows its mesh objects (the tail, or the tail and a list).
pub fn hat_record(after: &[u8]) -> Vec<u8> {
    record(11, 0, 0, &hat_mesh(), Some(&hat_companion()), after)
}

pub const HAT_PATH: &str = "C:\\models\\Hat";
pub const HAT_FLAGS: u32 = 0x811;
pub const BODY_COLOR: [u8; 4] = [0xAC, 0xCC, 0xE8, 0];

/// The list after the body's mesh objects: the hat first (with its objects), then the body.
pub fn list(hat_record: &[u8]) -> Vec<u8> {
    [
        RECORD_TAIL.to_vec(),
        item(0, MESH),
        name_object(HAT_PATH),
        hat_settings(),
        hat_record.to_vec(),
        entry(HAT_FLAGS, [0xFF; 4], "Hat", "Props", REST_11),
        item(0, MESH),
        entry(0x11, BODY_COLOR, "Body", "", REST_11),
    ]
    .concat()
}

/// The tool object with the body selected, built from its record.
pub fn tool_with(record: &[u8]) -> Vec<u8> {
    tool_object(MESH, "Body", &body_settings(), record)
}

pub fn body_record(head_len: usize, after: &[u8]) -> Vec<u8> {
    record(
        11,
        1,
        2,
        &body_mesh(head_len),
        Some(&body_companion()),
        after,
    )
}

pub fn tool() -> Vec<u8> {
    tool_with(&body_record(384, &list(&hat_record(&RECORD_TAIL))))
}

pub fn tool_file() -> Vec<u8> {
    file(b"LTZ\0", &tool())
}

/// A tool with one part whose record holds only a mesh object (a parametric primitive).
pub fn primitive_tool() -> Vec<u8> {
    let mesh = mesh_object(
        0x59,
        420,
        &[array_slot(12, &points(&stored_points(3, 0.0)), 1 << 17)],
        &[],
    );
    let after = [
        RECORD_TAIL.to_vec(),
        item(0, 0),
        entry(0x11, [0; 4], "Sphere3D_1", "", 12),
    ]
    .concat();
    tool_object(
        0,
        "Sphere3D_1",
        &hat_settings(),
        &record(13, 0, 1, &mesh, None, &after),
    )
}

pub fn project_file() -> Vec<u8> {
    let sections = [
        (0x800, object(1, &[2, 0, 0, 0])),
        (0x80, object(1, &[9; 20])),
        (2, tools_section(&[(1, tool()), (2, primitive_tool())])),
        (0x100, object(1, &[3; 8])),
    ];
    file(b"RPZ\0", &project_object(14, &sections))
}
