//! Differential tests: files written by our own writer from the format notes, read back field
//! by field, and faces of higher levels against the numbering worked out by hand.

use std::io::Cursor;

use gs_io::ReadError;
use gs_io::ztl::{FileKind, Geometry, RawSection, SettingKind, Ztl, open, read, subdivide_faces};
use gs_protocol::{AttrValues, NO_VERTEX, PlainMesh};

use crate::sample::{
    self, BODY_COLOR, BODY_FACE_DATA, BODY_FACES, BODY_GROUPS, BODY_LEVEL1, HAT_FLAGS, HAT_GROUP,
    HAT_LEVEL1, HAT_LEVEL3, HAT_PATH, MESH, OFFSET, SCALE,
};
use crate::writer::{RECORD_TAIL, entry, file, item, object};

fn read_ok(bytes: &[u8]) -> Ztl {
    read(bytes).unwrap_or_else(|e| panic!("synthetic file should read: {e}"))
}

/// Stored points in scene axes: Y and Z change sign.
fn scene(points: &[[f32; 3]]) -> Vec<[f32; 3]> {
    points.iter().map(|&[x, y, z]| [x, -y, -z]).collect()
}

fn column<'a>(mesh: &'a PlainMesh, name: &str) -> &'a AttrValues {
    &mesh
        .attributes
        .get(name)
        .unwrap_or_else(|| panic!("no {name} column"))
        .values
}

fn u32s<'a>(mesh: &'a PlainMesh, name: &str) -> &'a [u32] {
    let AttrValues::U32(v) = column(mesh, name) else {
        panic!("{name} should hold u32 values");
    };
    v
}

/// The level-1 faces of `BODY_FACES`, numbered by hand: edge points 7 to 10 and center 11 for
/// the first quad; the second quad reuses edge point 8 for the shared edge and adds 12 to 15;
/// the triangle reuses 13 and adds 16 to 18.
const BODY_LEVEL1_FACES: [[u32; 4]; 11] = [
    [0, 7, 11, 10],
    [7, 1, 8, 11],
    [11, 8, 2, 9],
    [10, 11, 9, 3],
    [1, 12, 15, 8],
    [12, 4, 13, 15],
    [15, 13, 5, 14],
    [8, 15, 14, 2],
    [4, 16, 18, 13],
    [16, 6, 17, 18],
    [13, 18, 17, 5],
];

#[test]
fn reads_the_header_and_the_part_list_in_list_order() {
    let z = read_ok(&sample::tool_file());
    assert_eq!(z.header.kind, FileKind::Tool);
    assert_eq!(z.header.preamble.writer, "2021.1");
    assert_eq!(z.header.icon.name, "Image");
    assert_eq!(z.header.icon.image.data, crate::writer::icon_pixels());
    assert_eq!(z.header.project, None);
    let [tool] = &z.tools[..] else {
        panic!("a tool file holds one tool");
    };
    assert_eq!((tool.project_flag, tool.selected), (None, 1));
    assert_eq!(tool.head.len(), 54);
    let [hat, body] = &tool.parts[..] else {
        panic!("two parts expected");
    };
    assert_eq!((hat.name.as_str(), body.name.as_str()), (HAT_PATH, "Body"));
    assert_eq!(
        (hat.kind, hat.lead, body.kind, body.lead),
        (MESH, 0, MESH, 0)
    );
    assert_eq!(
        (
            hat.entry.flags,
            hat.entry.name.as_str(),
            hat.entry.folder.as_deref()
        ),
        (HAT_FLAGS, "Hat", Some("Props"))
    );
    assert_eq!(
        (body.entry.color, body.entry.folder.as_deref()),
        (BODY_COLOR, None)
    );
    assert_eq!(body.entry.rest, [0; 4]);
    assert_eq!((hat.record_version, body.record_version), (11, 11));
    assert_eq!(body.record_head[12..20], [1, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!((hat.own_entry.clone(), body.extra.clone()), (None, vec![]));
}

#[test]
fn reads_settings_including_unnamed_records_and_skips_other_objects() {
    let z = read_ok(&sample::tool_file());
    let body = &z.tools[0].parts[1];
    assert_eq!(
        body.settings.head,
        [0x0B, 0, 0, 0, 0x0A, 0, 0, 0, 1],
        "plain fields up to the first record, its presence byte included"
    );
    let records: Vec<_> = body
        .settings
        .records
        .iter()
        .map(|r| (r.name.as_str(), r.kind, r.value, r.on))
        .collect();
    assert_eq!(
        records,
        [
            ("UndoCounter", SettingKind::Slider, 3.0, false),
            ("", SettingKind::Other(0x25), 0.0, false),
            ("Body", SettingKind::Switch, 0.0, true),
        ]
    );
}

#[test]
fn decodes_the_base_level_with_groups_mask_color_and_uvs() {
    let z = read_ok(&sample::tool_file());
    let g = &z.geometry[0][1];
    assert_eq!((g.mesh_version, g.companion_version), (0x52, Some(18)));
    assert_eq!(
        (
            g.mesh_head.len(),
            g.companion_head.len(),
            g.companion_tail.len()
        ),
        (384, 13, 172)
    );
    let m = g.mesh.as_ref().expect("the body is a polygon mesh");
    assert_eq!(m.base_faces(), BODY_FACES);
    assert_eq!(
        (m.base_vertex_count, m.current_level, m.level_count),
        (7, 0, 2)
    );
    let base = &m.levels[0];
    assert_eq!(base.level, 0);
    assert_eq!(base.mesh.positions, scene(&sample::stored_points(7, 0.25)));
    assert_eq!(base.mesh.faces, BODY_FACES);
    assert_eq!(base.vertex_data, sample::body_vertex_data());
    assert_eq!(u32s(&base.mesh, "group"), BODY_GROUPS);
    let AttrValues::F32(mask) = column(&base.mesh, "mask") else {
        panic!("the mask holds f32 values");
    };
    let half = 1.0 - f32::from(0x8000u16) / 65535.0;
    assert_eq!(mask[..], [0.0, 0.0, half, 1.0, 0.0, 0.0, 0.0]);
    let color = u32s(&base.mesh, "color");
    assert_eq!(
        color[0],
        u32::from_le_bytes([10, 20, 30, 255]),
        "stored as blue, green, red"
    );
    assert_eq!(color[1], u32::from_le_bytes([0, 0, 0, 255]));
    let uvs: Vec<[f32; 8]> = m
        .uvs
        .as_ref()
        .expect("the body has UVs")
        .iter()
        .map(|f| {
            [
                f[0][0], f[0][1], f[1][0], f[1][1], f[2][0], f[2][1], f[3][0], f[3][1],
            ]
        })
        .collect();
    assert_eq!(uvs, sample::body_uvs());
    assert_eq!(m.face_data.as_deref(), Some(&BODY_FACE_DATA[..]));
    let export = m.export.expect("the body stores an export scale");
    assert_eq!((export.scale, export.offset), (SCALE, OFFSET));
}

#[test]
fn the_top_level_gets_the_faces_of_its_level() {
    let z = read_ok(&sample::tool_file());
    let m = z.geometry[0][1].mesh.as_ref().unwrap();
    let top = &m.levels[1];
    assert_eq!(top.level, 1);
    assert_eq!(
        top.mesh.positions,
        scene(&sample::stored_points(BODY_LEVEL1, 0.5))
    );
    assert_eq!(top.mesh.faces, BODY_LEVEL1_FACES);
    assert_eq!(u32s(&top.mesh, "group"), [1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2]);
    assert!(
        !top.mesh.attributes.contains_key("mask") && !top.mesh.attributes.contains_key("color"),
        "an unmasked, unpainted level has no mask or color column"
    );
}

#[test]
fn a_part_saved_above_its_base_keeps_its_level_and_its_top_level() {
    let z = read_ok(&sample::tool_file());
    let m = z.geometry[0][0]
        .mesh
        .as_ref()
        .expect("the hat is a polygon mesh");
    assert_eq!(
        (m.base_vertex_count, m.current_level, m.level_count),
        (4, 1, 4)
    );
    assert_eq!(m.base_faces(), [[0, 1, 2, 3]]);
    let [current, top] = &m.levels[..] else {
        panic!("two stored levels");
    };
    assert_eq!((current.level, top.level), (1, 3));
    let level1 = [[0, 4, 8, 7], [4, 1, 5, 8], [8, 5, 2, 6], [7, 8, 6, 3]];
    assert_eq!(current.mesh.faces, level1);
    assert_eq!(
        current.mesh.positions,
        scene(&sample::stored_points(HAT_LEVEL1, -1.0))
    );
    let (level2, count2) = subdivide_faces(&level1, 9).unwrap();
    let (level3, count3) = subdivide_faces(&level2, count2).unwrap();
    assert_eq!((count2, count3 as usize), (25, HAT_LEVEL3));
    assert_eq!(top.mesh.faces, level3, "level 3 is built on from level 1");
    assert_eq!(
        top.mesh.positions,
        scene(&sample::stored_points(HAT_LEVEL3, 2.0))
    );
    assert_eq!(u32s(&current.mesh, "group"), [HAT_GROUP; 4]);
    assert_eq!(u32s(&top.mesh, "group"), [HAT_GROUP; 64]);
    assert!(current.vertex_data.is_empty() && top.vertex_data.is_empty());
    assert_eq!(
        current.mesh.attributes.len(),
        1,
        "no mask or color without vertex data"
    );
    assert_eq!((m.uvs.clone(), m.export), (None, None));
}

#[test]
fn subdividing_numbers_each_shared_edge_once() {
    let (faces, count) = subdivide_faces(&BODY_FACES, 7).unwrap();
    assert_eq!((faces.as_slice(), count), (&BODY_LEVEL1_FACES[..], 19));
    let (level2, count2) = subdivide_faces(&faces, count).unwrap();
    assert_eq!(level2.len(), 44);
    // 19 vertices, 29 edges, 11 faces at level 1.
    assert_eq!(count2, 19 + 29 + 11);
    assert_eq!(
        level2[..4],
        [
            [0, 19, 23, 22],
            [19, 7, 20, 23],
            [23, 20, 11, 21],
            [22, 23, 21, 10]
        ]
    );
    assert_eq!(
        subdivide_faces(&[[0, 1, 2, NO_VERTEX]], 3).unwrap().1,
        3 + 3 + 1
    );
    assert_eq!(
        subdivide_faces(&[[0, 1, 2, 7]], 7),
        None,
        "index past the count"
    );
    assert_eq!(subdivide_faces(&[[NO_VERTEX, 1, 2, 3]], 4), None);
    assert_eq!(subdivide_faces(&[], 5), Some((vec![], 5)));
}

#[test]
fn slots_not_decoded_are_kept_by_location() {
    let bytes = sample::tool_file();
    let z = read_ok(&bytes);
    let other = &z.geometry[0][1].other;
    let [slot, again] = &other[..] else {
        panic!("two slots kept, found {other:?}");
    };
    assert_eq!((slot.companion, slot.index, slot.array), (false, 9, None));
    let versions: Vec<u16> = slot.sections.iter().map(|s| s.version).collect();
    assert_eq!(versions, [13, 3]);
    let RawSection { offset, len, .. } = slot.sections[1];
    assert_eq!(&bytes[offset + 10..offset + len - 4], [7; 12]);
    let shape = again.array.map(|a| (a.element, a.count));
    assert_eq!(
        (again.companion, again.index, shape),
        (true, 5, Some((4, 3))),
        "a second array of per face words is kept, the first is the face data"
    );
}

#[test]
fn a_project_lists_every_tool_and_section() {
    let z = read_ok(&sample::project_file());
    assert_eq!(z.header.kind, FileKind::Project);
    let project = z.header.project.as_ref().expect("a project object");
    assert_eq!(project.version, 14);
    let tags: Vec<u32> = project.sections.iter().map(|(t, _)| *t).collect();
    assert_eq!(tags, [0x800, 0x80, 2, 0x100]);
    let flags: Vec<_> = z.tools.iter().map(|t| t.project_flag).collect();
    assert_eq!(flags, [Some(1), Some(2)]);
    assert_eq!(z.tools[0].parts.len(), 2);
    let primitive = &z.tools[1];
    assert_eq!(primitive.parts[0].entry.name, "Sphere3D_1");
    let g: &Geometry = &z.geometry[1][0];
    assert_eq!((g.mesh_version, g.companion_version), (0x59, None));
    assert!(g.mesh.is_none(), "a primitive has no faces");
    assert_eq!(
        g.other[0].array.map(|a| (a.element, a.count)),
        Some((12, 3))
    );
}

#[test]
fn open_decodes_a_part_only_when_asked() {
    // Break the hat's positions: the list still reads, the body still decodes.
    let broken = crate::writer::mesh_object(0x4A, 384, &[crate::writer::absent()], &[]);
    let hat = crate::writer::record(
        11,
        0,
        0,
        &broken,
        Some(&sample::hat_companion()),
        &RECORD_TAIL,
    );
    let bytes = file(
        b"LTZ\0",
        &sample::tool_with(&sample::body_record(384, &sample::list(&hat))),
    );
    let mut f = open(Cursor::new(bytes.as_slice())).expect("the list reads without geometry");
    assert_eq!(f.tools[0].parts.len(), 2);
    assert!(f.geometry(0, 1).is_ok());
    let err = f.geometry(0, 0).expect_err("the hat has no positions");
    assert!(matches!(
        err,
        ReadError::BadMesh {
            reason: "a mesh with faces stores positions",
            ..
        }
    ));
    assert!(read(&bytes).is_err());
    assert!(matches!(
        f.geometry(0, 2),
        Err(ReadError::BadLayout {
            reason: "no such tool or part",
            ..
        })
    ));
    assert!(f.geometry(1, 0).is_err());
}

#[test]
fn a_part_record_may_list_itself() {
    let own = [
        RECORD_TAIL.to_vec(),
        item(0, MESH),
        entry(0x11, [1; 4], "PolyMesh3D", "", 4),
    ]
    .concat();
    let bytes = file(
        b"LTZ\0",
        &sample::tool_with(&sample::body_record(
            384,
            &sample::list(&sample::hat_record(&own)),
        )),
    );
    let z = read_ok(&bytes);
    let hat = &z.tools[0].parts[0];
    let own = hat
        .own_entry
        .as_ref()
        .expect("the hat's record keeps its own entry");
    assert_eq!((own.name.as_str(), own.color), ("PolyMesh3D", [1; 4]));
    assert_eq!(hat.entry.name, "Hat");
}

#[test]
fn a_second_mesh_group_is_kept_and_the_list_still_reads() {
    let second = [
        vec![0, 1, 4, 0, 0, 0],
        object(13, &[0, 0]),
        crate::writer::rect_object(),
        object(0x4A, &[5; 30]),
        object(28, &[6; 30]),
    ]
    .concat();
    let after = sample::list(&sample::hat_record(&RECORD_TAIL));
    let after = [second, after[1..].to_vec()].concat();
    let z = read_ok(&file(
        b"LTZ\0",
        &sample::tool_with(&sample::body_record(384, &after)),
    ));
    let body = &z.tools[0].parts[1];
    let versions: Vec<u16> = body.extra.iter().map(|s| s.version).collect();
    assert_eq!(versions, [0x4A, 28]);
    assert_eq!(z.tools[0].parts[0].entry.name, "Hat");
}

#[test]
fn plain_fields_of_any_length_around_the_search_window() {
    // The body's mesh head is plain fields; lengths around the reader's search window make the
    // next object start at every position near a window edge.
    let expected = read_ok(&sample::tool_file()).geometry[0][1]
        .mesh
        .as_ref()
        .unwrap()
        .levels[1]
        .mesh
        .positions
        .clone();
    for head in 240..=520 {
        let after = sample::list(&sample::hat_record(&RECORD_TAIL));
        let bytes = file(
            b"LTZ\0",
            &sample::tool_with(&sample::body_record(head, &after)),
        );
        let z = read(&bytes).unwrap_or_else(|e| panic!("head of {head} bytes: {e}"));
        let g = &z.geometry[0][1];
        assert_eq!(g.mesh_head.len(), head);
        assert_eq!(g.mesh.as_ref().unwrap().levels[1].mesh.positions, expected);
    }
}

#[test]
fn a_long_list_reads_every_entry_in_order() {
    let mut after = RECORD_TAIL.to_vec();
    for i in 0..5 {
        after.extend(item(0, MESH));
        after.extend(crate::writer::name_object(&format!("H{i}")));
        after.extend(sample::hat_settings());
        after.extend(sample::hat_record(&RECORD_TAIL));
        after.extend(entry(0x11, [0; 4], &format!("Hat {i}"), "", 4));
    }
    after.extend(item(0, MESH));
    after.extend(entry(0x11, [0; 4], "Body", "", 4));
    let z = read_ok(&file(
        b"LTZ\0",
        &sample::tool_with(&sample::body_record(384, &after)),
    ));
    let names: Vec<&str> = z.tools[0]
        .parts
        .iter()
        .map(|s| s.entry.name.as_str())
        .collect();
    assert_eq!(names, ["Hat 0", "Hat 1", "Hat 2", "Hat 3", "Hat 4", "Body"]);
    assert_eq!(z.tools[0].selected, 5);
}
