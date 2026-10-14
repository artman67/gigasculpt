//! PLY: round trips, every property type in both byte orders and in ASCII, and damaged files.

use gs_io::ReadError;
use gs_io::mesh::{MeshFile, ReadOptions, WriteOptions, column};
use gs_io::ply::read;
use gs_protocol::{AttrDomain, PlainMesh};

use crate::common::{
    NO, assert_same, bare, consistent, full_mesh, line_count, only, single, write_as,
};

fn ply(meshes: &[(&str, &PlainMesh)], options: WriteOptions) -> Vec<u8> {
    write_as("ply", meshes, options)
}

fn parse(bytes: &[u8]) -> Result<MeshFile, ReadError> {
    read(bytes, &ReadOptions::default())
}

fn colors(mesh: &PlainMesh) -> &[u32] {
    column(mesh, "color", AttrDomain::Vertex).unwrap()
}

#[test]
fn meshes_round_trip_in_binary_and_ascii() {
    let mesh = only(&full_mesh(), &["color"]);
    for binary in [true, false] {
        let options = WriteOptions {
            binary,
            ..WriteOptions::default()
        };
        assert_same(
            single(&parse(&ply(&[("", &mesh)], options)).unwrap()),
            &mesh,
        );
        let plain = ply(
            &[("", &mesh)],
            WriteOptions {
                colors: false,
                ..options
            },
        );
        assert_same(single(&parse(&plain).unwrap()), &bare(&mesh));
        let tris = parse(&ply(
            &[("", &mesh)],
            WriteOptions {
                triangles: true,
                flip_faces: true,
                ..options
            },
        ));
        // The quad splits along its shorter diagonal, corner 1 to 3 (corner 2 is far off).
        assert_eq!(
            single(&tris.unwrap()).faces,
            [[1, 3, 2, NO], [1, 0, 3, NO], [1, 2, 4, NO], [3, 5, 2, NO]]
        );
    }
    // Several objects become one; one without colors is white.
    let both = parse(&ply(
        &[("a", &mesh), ("b", &bare(&mesh))],
        WriteOptions::default(),
    ))
    .unwrap();
    let m = single(&both);
    assert_eq!(m.positions.len(), 12);
    assert_eq!(m.faces[3], [6, 7, 8, 9]);
    assert_eq!(colors(m)[..6], *colors(&mesh));
    assert!(colors(m)[6..].iter().all(|&c| c == u32::MAX));
}

/// The bytes of one value of a PLY type.
fn value(ty: &str, x: f64, big: bool) -> Vec<u8> {
    let mut b = match ty {
        "char" => vec![x as i8 as u8],
        "uchar" => vec![x as u8],
        "short" => (x as i16).to_le_bytes().to_vec(),
        "ushort" => (x as u16).to_le_bytes().to_vec(),
        "int" => (x as i32).to_le_bytes().to_vec(),
        "uint" => (x as u32).to_le_bytes().to_vec(),
        "float" => (x as f32).to_le_bytes().to_vec(),
        "double" => x.to_le_bytes().to_vec(),
        _ => panic!("{ty}"),
    };
    if big {
        b.reverse();
    }
    b
}

/// A PLY file: positions of type `ty` (plus an ignored property of every other type), a
/// triangle and a quad with list counts and indices of the given types.
fn typed(format: &str, ty: &str, count: &str, index: &str) -> Vec<u8> {
    const TYPES: [&str; 8] = [
        "char", "uchar", "short", "ushort", "int", "uint", "float", "double",
    ];
    let points = [
        [1.0, 2.0, 3.0],
        [-4.0, 5.0, 6.0],
        [7.0, -8.0, 9.0],
        [10.0, 11.0, -12.0],
    ];
    let mut head = format!("ply\nformat {format} 1.0\ncomment by hand\nelement vertex 4\n");
    for name in ["x", "y", "z"] {
        head += &format!("property {ty} {name}\n");
    }
    for (i, t) in TYPES.iter().enumerate() {
        head += &format!("property {t} extra{i}\n");
    }
    head += &format!("element face 2\nproperty list {count} {index} vertex_index\n");
    head += "property uchar flags\nelement edge 1\nproperty int a\nproperty list uchar int b\nend_header\n";
    let mut out = head.into_bytes();
    let faces: [&[f64]; 2] = [&[0.0, 1.0, 2.0], &[0.0, 2.0, 3.0, 1.0]];
    if format == "ascii" {
        for p in points {
            let extra = [
                "-3",
                "200",
                "-300",
                "60000",
                "-70000",
                "4000000000",
                "0.5",
                "0.25",
            ];
            out.extend(format!("{} {} {} {}\n", p[0], p[1], p[2], extra.join(" ")).bytes());
        }
        for f in faces {
            let ids: Vec<String> = f.iter().map(ToString::to_string).collect();
            out.extend(format!("{} {} 9\n", f.len(), ids.join(" ")).bytes());
        }
        out.extend(b"5 2 6 7\n");
        return out;
    }
    let big = format == "binary_big_endian";
    for p in points {
        for x in p {
            out.extend(value(ty, x, big));
        }
        for (t, x) in TYPES
            .iter()
            .zip([-3.0, 200.0, -300.0, 6e4, -7e4, 4e9, 0.5, 0.25])
        {
            out.extend(value(t, x, big));
        }
    }
    for f in faces {
        out.extend(value(count, f.len() as f64, big));
        for &v in f {
            out.extend(value(index, v, big));
        }
        out.push(9);
    }
    out.extend(value("int", 5.0, big));
    out.extend([2]);
    out.extend(value("int", 6.0, big));
    out.extend(value("int", 7.0, big));
    out
}

#[test]
fn every_type_reads_in_every_format() {
    let types = [
        "char", "uchar", "short", "ushort", "int", "uint", "float", "double",
    ];
    for format in ["ascii", "binary_little_endian", "binary_big_endian"] {
        for (i, ty) in types.iter().enumerate() {
            let (count, index) = (types[i % 6], types[(i + 2) % 6 + 2]);
            let file = parse(&typed(format, ty, count, index));
            let file = file.unwrap_or_else(|e| panic!("{format} {ty}: {e}"));
            let m = single(&file);
            let unsigned = ty.starts_with('u');
            let want: Vec<[f32; 3]> = [
                [1.0, 2.0, 3.0],
                [-4.0, 5.0, 6.0],
                [7.0, -8.0, 9.0],
                [10.0, 11.0, -12.0],
            ]
            .iter()
            .map(|p| p.map(|x: f32| if unsigned { x.max(0.0) } else { x }))
            .collect();
            if !unsigned {
                assert_eq!(m.positions, want, "{format} {ty}");
            }
            assert_eq!(m.faces, [[0, 1, 2, NO], [0, 2, 3, 1]], "{format} {ty}");
            let ignored = &file.notes.ignored;
            assert_eq!(ignored["vertex property extra7"], 4);
            assert_eq!(
                (ignored["face property flags"], ignored["element edge"]),
                (2, 1)
            );
        }
    }
}

#[test]
fn colors_read_as_bytes_or_fractions() {
    let text = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
                property float z\nproperty float red\nproperty float green\nproperty float blue\n\
                element face 1\nproperty list uchar uint vertex_indices\nend_header\n\
                0 0 0 1 0.5 0\n1 0 0 2 -1 0.25\n0 1 0 0 0 0\n3 0 1 2\n";
    let m = parse(text.as_bytes()).unwrap();
    let c = colors(single(&m));
    assert_eq!(c[0], u32::from_le_bytes([255, 128, 0, 255]));
    assert_eq!(c[1], u32::from_le_bytes([255, 0, 64, 255]), "clamped");
    let bytes = text
        .replace("float red", "uchar red")
        .replace("float green", "uchar green")
        .replace("float blue", "uchar blue");
    let bytes = bytes
        .replace("0 0 0 1 0.5 0", "0 0 0 1 300 0")
        .replace("1 0 0 2 -1 0.25", "1 0 0 2 1 4");
    let m = parse(bytes.as_bytes()).unwrap();
    assert_eq!(colors(single(&m))[0], u32::from_le_bytes([1, 255, 0, 255]));
    // A file without faces holds no object.
    let points = "ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n0 0 0\n";
    assert!(parse(points.as_bytes()).unwrap().objects.is_empty());
}

fn error_of(bytes: &[u8]) -> ReadError {
    parse(bytes)
        .err()
        .unwrap_or_else(|| panic!("{:?} read", String::from_utf8_lossy(bytes)))
}

#[test]
fn damaged_headers_and_values_are_named() {
    let head = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
                property float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";
    let body = "0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
    let line = |text: &str| match error_of(text.as_bytes()) {
        ReadError::BadLine { line, .. } => line,
        e => panic!("{e}"),
    };
    assert_eq!(line(&head.replace("ply\n", "plyx\n")), 1);
    assert_eq!(line(""), 1);
    assert_eq!(line(&head.replace("ascii", "utf8")), 2);
    assert_eq!(
        line(&head.replace(
            "1.0\nelement vertex 3",
            "1.0\nproperty float w\nelement vertex 3"
        )),
        3
    );
    assert_eq!(line(&head.replace("vertex 3", "vertex three")), 3);
    assert_eq!(
        line(&head.replace("property float x", "property quad x")),
        4
    );
    assert_eq!(line(&head.replace("list uchar int", "list quad int")), 8);
    assert_eq!(line(&head.replace("element face 1", "elephant face 1")), 7);
    assert_eq!(line(&head.replace("format ascii 1.0\n", "")), 8);
    assert_eq!(line(&head.replace("end_header\n", "")), 9);
    assert_eq!(
        line(&head.replace("property float x", "property float q")),
        9
    );
    assert_eq!(
        line(&head.replace("property float x", "property list uchar float x")),
        9
    );
    assert_eq!(line(&head.replace("vertex_indices", "corners")), 9);
    assert_eq!(line(&head.replace("vertex 3", "vertex 4294967295")), 9);
    let with = |b: &str| line(&format!("{head}{b}"));
    assert_eq!(with("0 0 0\n1 0 0\n"), 12);
    assert_eq!(with("0 0 0\n1 0\n"), 11);
    assert_eq!(with("0 0 0\n1 0 0 4\n"), 11);
    assert_eq!(with("0 0 0\n1 0 z\n"), 11);
    assert_eq!(with("0 0 0\n1 0 inf\n"), 11);
    assert_eq!(with(&body.replace("3 0 1 2", "3 0 1 3")), 13);
    assert_eq!(with(&body.replace("3 0 1 2", "2 0 1")), 13);
    assert_eq!(with(&body.replace("3 0 1 2", "-1")), 13);
    let binary = ply(&[("", &bare(&full_mesh()))], WriteOptions::default());
    let at = binary
        .windows(11)
        .position(|w| w == b"end_header\n")
        .unwrap()
        + 11;
    let mut bad = binary.clone();
    bad[at + 6 * 12 + 1..at + 6 * 12 + 5].copy_from_slice(&9u32.to_le_bytes());
    assert_eq!(
        error_of(&bad),
        ReadError::BadValue {
            offset: at as u64 + 6 * 12 + 17,
            reason: "a face refers to vertex 9, but there are 6".into()
        }
    );
}

#[test]
fn a_list_too_long_is_refused_before_its_items() {
    // A count read as a 32-bit word: 70,000 corners, more than one polygon may have.
    let mut bytes = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\n\
                      property float y\nproperty float z\nelement face 1\n\
                      property list uint int vertex_indices\nend_header\n"
        .to_vec();
    let head = bytes.len() as u64;
    bytes.extend([0; 12]);
    bytes.extend(70_000u32.to_le_bytes());
    let err = error_of(&bytes);
    assert_eq!(
        err,
        ReadError::BadValue {
            offset: head + 16,
            reason: "a list length of 70000".into()
        }
    );
    assert_eq!(
        err.to_string(),
        format!("bad value at byte {}: a list length of 70000", head + 16)
    );
}

#[test]
fn every_cut_off_is_an_error_inside_the_file() {
    for binary in [true, false] {
        let options = WriteOptions {
            binary,
            ..WriteOptions::default()
        };
        let bytes = ply(&[("", &full_mesh())], options);
        let lines = line_count(&bytes);
        for end in 0..bytes.len() {
            match parse(&bytes[..end]) {
                Err(ReadError::Truncated { offset, .. }) => {
                    assert!(binary && offset <= end, "cut at {end}");
                }
                Err(ReadError::BadLine { line, .. }) => assert!(line <= lines, "cut at {end}"),
                Err(e) => panic!("cut at {end}: {e}"),
                Ok(file) => {
                    assert!(
                        !binary && end + 1 == bytes.len() && consistent(&file),
                        "cut at {end} read"
                    );
                }
            }
        }
    }
}
