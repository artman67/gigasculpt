//! OBJ: round trips, files as other programs write them, and damaged files.

use gs_io::ReadError;
use gs_io::mesh::{MeshFile, Number, ReadOptions, UV_COLUMNS, WriteOptions, column};
use gs_io::obj::read;
use gs_protocol::PlainMesh;

use crate::common::{
    NO, assert_same, bare, consistent, full_mesh, line_count, only, single, write_as,
};

fn obj(meshes: &[(&str, &PlainMesh)], options: WriteOptions) -> Vec<u8> {
    write_as("obj", meshes, options)
}

fn parse(text: &str) -> MeshFile {
    read(text.as_bytes(), &ReadOptions::default()).unwrap()
}

fn parse_with(text: &str, options: ReadOptions) -> MeshFile {
    read(text.as_bytes(), &options).unwrap()
}

fn col<'a, T: Number>(mesh: &'a PlainMesh, name: &str) -> &'a [T] {
    column(mesh, name, mesh.attributes[name].domain).unwrap()
}

fn line_of(text: &str) -> u64 {
    match read(text.as_bytes(), &ReadOptions::default()) {
        Err(ReadError::BadLine { line, .. }) => line,
        other => panic!("expected a bad line, got {other:?}"),
    }
}

#[test]
fn a_full_mesh_round_trips_exactly() {
    let mesh = full_mesh();
    let bytes = obj(&[("", &mesh)], WriteOptions::default());
    assert!(
        !String::from_utf8_lossy(&bytes).contains("\no "),
        "one unnamed object: no o line"
    );
    let file = read(&bytes[..], &ReadOptions::default()).unwrap();
    assert_eq!(file.objects[0].name, "");
    assert_same(single(&file), &mesh);
    let named = read(
        &obj(&[("part", &mesh)], WriteOptions::default())[..],
        &ReadOptions::default(),
    );
    assert_eq!(named.unwrap().objects[0].name, "part");
    // Several named objects: each comes back under its name, with its own vertices and UVs.
    let mut keep = vec!["group"];
    keep.extend(UV_COLUMNS);
    let small = only(&mesh, &keep);
    let bytes = obj(
        &[("head", &mesh), ("eyes", &small)],
        WriteOptions::default(),
    );
    let file = read(&bytes[..], &ReadOptions::default()).unwrap();
    let names: Vec<&str> = file.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["head", "eyes"]);
    assert_same(&file.objects[0].mesh, &mesh);
    assert_same(&file.objects[1].mesh, &small);
    assert_eq!(file.objects[1].group_names[&65535], "Group65535");
}

#[test]
fn writer_switches_leave_columns_out() {
    let mesh = full_mesh();
    let read_back = |options| written_and_read(&mesh, options);
    assert_same(&read_back(OFF), &bare(&mesh));
    let mask_only = read_back(WriteOptions {
        colors: false,
        ..WriteOptions::default()
    });
    assert_same(
        &mask_only,
        &only(
            &mesh,
            &[
                "group", "mask", "uv0.u", "uv0.v", "uv1.u", "uv1.v", "uv2.u", "uv2.v", "uv3.u",
                "uv3.v",
            ],
        ),
    );
    // Colors still come back from the #MRGB lines alone.
    let colors_from_mrgb = read_back(WriteOptions {
        uvs: false,
        groups: false,
        ..WriteOptions::default()
    });
    let text = String::from_utf8(obj(
        &[("", &mesh)],
        WriteOptions {
            uvs: false,
            groups: false,
            ..WriteOptions::default()
        },
    ))
    .unwrap();
    let without_v_colors: String = text
        .lines()
        .map(|l| {
            if l.starts_with("v ") {
                l.split(' ').take(4).collect::<Vec<_>>().join(" ")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_same(single(&parse(&without_v_colors)), &colors_from_mrgb);
    assert_same(&colors_from_mrgb, &only(&mesh, &["color", "mask"]));
    // Colors alone travel after the positions.
    let colors_only = read_back(WriteOptions {
        uvs: false,
        groups: false,
        mask: false,
        ..WriteOptions::default()
    });
    assert_same(&colors_only, &only(&mesh, &["color"]));
}

/// Only positions and faces.
const OFF: WriteOptions = WriteOptions {
    triangles: false,
    flip_faces: false,
    uvs: false,
    groups: false,
    colors: false,
    mask: false,
    binary: false,
};

fn written_and_read(mesh: &PlainMesh, options: WriteOptions) -> PlainMesh {
    let text = String::from_utf8(obj(&[("", mesh)], options)).unwrap();
    single(&parse_with(&text, ReadOptions::default())).clone()
}

#[test]
fn split_and_flipped_faces_keep_their_corner_values() {
    // The quad splits along its shorter diagonal, corner 1 to 3 (corner 2 is far off); UVs
    // follow their corners.
    let mesh = full_mesh();
    let read_back = |options| written_and_read(&mesh, options);
    let off = OFF;
    let tris = read_back(WriteOptions {
        triangles: true,
        ..off
    });
    assert_eq!(
        tris.faces,
        [[1, 2, 3, NO], [1, 3, 0, NO], [1, 4, 2, NO], [3, 2, 5, NO]]
    );
    let flipped = read_back(WriteOptions {
        flip_faces: true,
        ..off
    });
    assert_eq!(flipped.faces, [[0, 3, 2, 1], [1, 2, 4, NO], [3, 5, 2, NO]]);
    let both = read_back(WriteOptions {
        flip_faces: true,
        triangles: true,
        ..WriteOptions::default()
    });
    assert_eq!(both.faces[..2], [[1, 3, 2, NO], [1, 0, 3, NO]]);
    let uv = |m: &PlainMesh, f: usize, k: usize| {
        [
            col::<f32>(m, UV_COLUMNS[2 * k])[f],
            col::<f32>(m, UV_COLUMNS[2 * k + 1])[f],
        ]
    };
    assert_eq!(uv(&both, 0, 1), uv(&mesh, 0, 3), "UVs follow their corners");
    assert_eq!(
        uv(&both, 1, 1),
        uv(&mesh, 0, 0),
        "and in the second triangle"
    );
}

#[test]
fn files_as_other_programs_write_them_read() {
    // A sculpting app's export: header comments, CRLF on some lines, all vertices, the #MRGB
    // block (64 entries per line), then groups and faces.
    let mut text = String::from("# File exported by a sculpting app\r\n#Vertex Count 70\r\n");
    for i in 0..70 {
        text += &format!("v {i} {} 0.5\r\n", i % 7);
    }
    text += "# The following MRGB block holds mask and color\r\n";
    let entry = |i: usize| match i {
        3 => "00ff0000".to_string(),
        66 => "80102030".to_string(),
        _ => "ffffffff".to_string(),
    };
    text += &format!("#MRGB {}\r\n", (0..64).map(entry).collect::<String>());
    text += &format!("#MRGB {}\r\n", (64..70).map(entry).collect::<String>());
    text += "g Group9\nf 1 2 3\ng Group4\nf 4 5 6 7\n";
    let file = parse(&text);
    assert!(
        file.notes.ignored.is_empty(),
        "comments are not counted: {:?}",
        file.notes
    );
    let m = single(&file).clone();
    assert_eq!(m.positions.len(), 70);
    assert_eq!(m.positions[69], [69.0, 6.0, 0.5]);
    assert_eq!(col::<u32>(&m, "group"), [9, 4]);
    let mask = col::<f32>(&m, "mask");
    assert_eq!(
        (mask[3], mask[66], mask[0]),
        (1.0, 1.0 - 128.0 / 255.0, 0.0)
    );
    let color = col::<u32>(&m, "color");
    assert_eq!(color[3], u32::from_le_bytes([255, 0, 0, 255]));
    assert_eq!(color[66], u32::from_le_bytes([0x10, 0x20, 0x30, 255]));
    assert_eq!(color[0], u32::MAX, "white where nothing was painted");
    // Other programs: weights, colors after the position (with and without a weight),
    // negative indices, normals and UV indices in every form, and lines to ignore.
    let text = "mtllib a.mtl\nv 0 0 0 1\nv 1 0 0 1 0.5 0\nv 1 1 0 0.5 0 0 1\nv 0 1 0\n\
                vt 0.25 0.5\nvt 1 1 0\nvn 0 0 1\ns 1\nusemtl skin\n\
                f -4/1/1 -3/2/1 -2//1 -1\nl 1 2\n";
    let file = parse(text);
    let m = single(&file);
    assert_eq!(m.faces, [[0, 1, 2, 3]]);
    assert_eq!(
        col::<u32>(m, "color")[1],
        u32::from_le_bytes([255, 128, 0, 255])
    );
    assert_eq!(
        col::<u32>(m, "color")[2],
        u32::from_le_bytes([0, 0, 255, 255])
    );
    assert_eq!(col::<u32>(m, "color")[3], u32::MAX);
    assert_eq!(col::<f32>(m, "uv0.u")[0], 0.25);
    assert_eq!(
        (col::<f32>(m, "uv1.u")[0], col::<f32>(m, "uv2.u")[0]),
        (1.0, 0.0)
    );
    assert!(!m.attributes.contains_key("group"), "no named group");
    let ignored = &file.notes.ignored;
    for key in [
        "normals",
        "material libraries",
        "smoothing groups",
        "other lines",
    ] {
        assert_eq!(ignored[key], 1, "{key}");
    }
}

#[test]
fn objects_own_the_vertices_defined_in_them() {
    // Faces may use vertices of earlier blocks (copied in) and vertices and faces interleave;
    // the #MRGB entries of a block are for its own vertices.
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n\
                o second\nv 5 5 5\nf 1 2 4\nv 6 6 6\nf 4 5 3\n#MRGB 00ffffff00ffffff\n\
                o empty\no points\nv 9 9 9\n";
    let file = parse(text);
    assert_eq!(file.objects.len(), 2, "objects without faces are dropped");
    let second = &file.objects[1].mesh;
    assert_eq!(
        second.positions,
        [
            [5.0; 3],
            [0.0; 3],
            [1.0, 0.0, 0.0],
            [6.0; 3],
            [0.0, 1.0, 0.0]
        ]
    );
    assert_eq!(second.faces, [[1, 2, 0, NO], [0, 3, 4, NO]]);
    assert_eq!(col::<f32>(second, "mask"), [1.0, 0.0, 0.0, 1.0, 0.0]);
    assert!(!file.objects[0].mesh.attributes.contains_key("mask"));
    assert_eq!(file.notes.ignored["vertices of objects without faces"], 1);
    let past = parse(
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\no b\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 4 5 6\n\
         #MRGB 00ffffff00ffffff00ffffff00ffffff\n",
    );
    assert_eq!(past.notes.ignored["#MRGB entries past the last vertex"], 1);
    // A borrowed vertex keeps its color and mask.
    let painted = parse(
        "v 0 0 0\nv 1 0 0 1 0 0\nv 0 1 0\n#MRGB ffffffff80ff0000ffffffff\nf 1 2 3\n\
         o second\nv 5 5 5\nf 2 3 4\n",
    );
    let second = &painted.objects[1].mesh;
    let red = u32::from_le_bytes([255, 0, 0, 255]);
    assert_eq!(col::<u32>(second, "color"), [u32::MAX, red, u32::MAX]);
    assert_eq!(col::<f32>(second, "mask"), [0.0, 1.0 - 128.0 / 255.0, 0.0]);
    // Two unnamed objects stay two.
    let small = only(&full_mesh(), &[]);
    let two = read(
        &obj(&[("", &small), ("", &small)], WriteOptions::default())[..],
        &ReadOptions::default(),
    );
    assert_eq!(two.unwrap().objects.len(), 2);
}

#[test]
fn groups_number_in_order_unless_all_are_numbered() {
    let text = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\ng arm\nusemtl red\nf 1 2 3\ng leg\nf 1 2 3\n\
                usemtl blue\ng arm\nf 1 2 3\n";
    let file = parse(text);
    let m = single(&file);
    assert_eq!(
        col::<u32>(m, "group"),
        [0, 1, 2, 1],
        "default, arm, leg, arm"
    );
    let names = &file.objects[0].group_names;
    assert_eq!((names[&0].as_str(), names[&2].as_str()), ("default", "leg"));
    let materials = ReadOptions {
        materials_as_groups: true,
        ..ReadOptions::default()
    };
    assert_eq!(
        col::<u32>(single(&parse_with(text, materials)), "group"),
        [0, 1, 1, 2]
    );
    // All numbered: the numbers are the ids. One name not in the form: order of first use.
    let numbered = "v 0 0 0\nv 1 0 0\nv 0 1 0\ng Group12\nf 1 2 3\ng Group3\nf 1 2 3\n";
    assert_eq!(col::<u32>(single(&parse(numbered)), "group"), [12, 3]);
    let padded = numbered.replace("Group3", "Group03");
    assert_eq!(col::<u32>(single(&parse(&padded)), "group"), [0, 1]);
}

#[test]
fn polygons_split_into_fans_that_mirror() {
    // A pentagon right of x = 0 and its mirror image on the left (corners in the other order,
    // starting elsewhere): their triangles are mirror images of each other.
    let right = [
        [1.0, 0.5, 0.0],
        [3.0, 0.0, 0.0],
        [4.0, 2.0, 0.0],
        [2.5, 3.0, 0.0],
        [1.0, 2.0, 0.0],
    ];
    let mut text = String::new();
    for p in right.iter().chain(&right.map(|p| [-p[0], p[1], p[2]])) {
        text += &format!("v {} {} {}\n", p[0], p[1], p[2]);
    }
    text += "f 1 2 3 4 5\nf 8 7 6 10 9\nf 1 2 3\n";
    let file = parse(&text);
    let m = single(&file);
    assert!(
        !m.attributes.contains_key("group"),
        "no groups unless asked"
    );
    let plain = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let asked = parse_with(
        plain,
        ReadOptions {
            group_polygons: true,
            ..ReadOptions::default()
        },
    );
    assert!(
        !single(&asked).attributes.contains_key("group"),
        "no polygons, no groups"
    );
    assert_eq!(file.notes.polygons_split, 2);
    assert_eq!(m.faces.len(), 7);
    let corners = |f: &[u32; 4]| {
        f[..3]
            .iter()
            .map(|&v| m.positions[v as usize])
            .collect::<Vec<_>>()
    };
    // The fan starts at the corner lowest in y.
    assert!(m.faces[..3].iter().all(|f| f[0] == 1));
    for f in &m.faces[..3] {
        let mut mirrored: Vec<[f32; 3]> = corners(f).iter().map(|p| [-p[0], p[1], p[2]]).collect();
        mirrored.reverse();
        let found = m.faces[3..6].iter().any(|g| {
            let c = corners(g);
            (0..3).any(|r| {
                let mut turned = c.clone();
                turned.rotate_left(r);
                turned == mirrored
            })
        });
        assert!(found, "face {f:?} has no mirror partner");
    }
    let grouped = ReadOptions {
        group_polygons: true,
        ..ReadOptions::default()
    };
    assert_eq!(
        col::<u32>(single(&parse_with(&text, grouped)), "group"),
        [1, 1, 1, 2, 2, 2, 0]
    );
    let named = format!("g Group5\n{text}");
    assert_eq!(
        col::<u32>(single(&parse_with(&named, grouped)), "group"),
        [6, 6, 6, 7, 7, 7, 5]
    );
}

#[test]
fn damaged_lines_are_named() {
    let ok = "v 0 0 0\nv 1 0 0\nv 0 1 0\n";
    let cases = [
        (format!("{ok}f 1 2 x\n"), 4),
        (format!("{ok}f 1 2 0\n"), 4),
        (format!("{ok}f 1 2 4\n"), 4),
        (format!("{ok}f 1 2 -4\n"), 4),
        (format!("{ok}f 1 2\n"), 4),
        (format!("{ok}f 1/1 2 3\n"), 4),
        (format!("{ok}\n\nv 1 nan 0\n"), 6),
        ("v 1 2\n".into(), 1),
        ("v 1 2 3 4 5\n".into(), 1),
        ("v 1 2 3 4 5 6 7 8\n".into(), 1),
        ("vt 1\n".into(), 1),
        ("vt 1 2 3 4\n".into(), 1),
        (format!("{ok}#MRGB ff00000\n"), 4),
        (format!("{ok}#MRGB ff00zz00\n"), 4),
    ];
    for (text, line) in cases {
        assert_eq!(line_of(&text), line, "{text:?}");
    }
    let message = read(&b"v 0 0 0\nf 1 2 3\n"[..], &ReadOptions::default()).unwrap_err();
    assert_eq!(
        message.to_string(),
        "line 2: a face refers to vertex 2, but 1 are defined so far"
    );
}

#[test]
fn a_last_line_without_an_end_reads_and_empty_objects_leave_no_note() {
    let file = parse("o empty\no full\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3");
    assert_eq!(single(&file).faces, [[0, 1, 2, NO]]);
    assert!(file.notes.ignored.is_empty(), "{:?}", file.notes);
}

#[test]
fn lines_up_to_16_mib_read_and_longer_ones_are_refused() {
    let limit = 1 << 24;
    let mut text = format!("# {}\n", "x".repeat(limit - 3));
    text += "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    assert_eq!(single(&parse(&text)).faces.len(), 1);
    let long = format!("v 0 0 0\n# {}\n", "x".repeat(limit));
    assert_eq!(line_of(&long), 2);
}

#[test]
fn every_cut_off_reads_or_names_a_line_inside_the_file() {
    let bytes = obj(&[("", &full_mesh())], WriteOptions::default());
    let lines = line_count(&bytes);
    for end in 0..bytes.len() {
        match read(&bytes[..end], &ReadOptions::default()) {
            Ok(file) => assert!(consistent(&file), "cut at {end}"),
            Err(ReadError::BadLine { line, .. }) => assert!(line <= lines, "cut at {end}"),
            Err(e) => panic!("cut at {end}: {e}"),
        }
    }
}
