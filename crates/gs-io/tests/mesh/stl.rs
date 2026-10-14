//! STL: round trips, telling ASCII from binary, and damaged files.

use gs_io::ReadError;
use gs_io::mesh::{MeshFile, WriteOptions};
use gs_io::stl::read;
use gs_protocol::{PlainMesh, face_triangles};

use crate::common::{NO, bare, consistent, full_mesh, line_count, write_as};

fn stl(meshes: &[(&str, &PlainMesh)], options: WriteOptions) -> Vec<u8> {
    write_as("stl", meshes, options)
}

/// The corners of every triangle, as bits.
fn corners(file: &MeshFile) -> Vec<Vec<[u32; 3]>> {
    (file.objects.iter())
        .map(|o| {
            let m = &o.mesh;
            m.faces
                .iter()
                .flat_map(|f| {
                    assert_eq!(f[3], NO, "triangles only");
                    f[..3]
                        .iter()
                        .map(|&v| m.positions[v as usize].map(f32::to_bits))
                })
                .collect()
        })
        .collect()
}

/// The corners the triangles of `mesh` should have: quads split as they are drawn.
fn expected(mesh: &PlainMesh, flip: bool) -> Vec<[u32; 3]> {
    let tri = |a: u32, b: u32, c: u32| {
        let t = if flip { [a, c, b] } else { [a, b, c] };
        t.map(|v| mesh.positions[v as usize].map(f32::to_bits))
    };
    let at = |v: u32| mesh.positions[v as usize];
    mesh.faces
        .iter()
        .flat_map(|f| {
            let tris = face_triangles(*f, |k| at(f[k]));
            tris.iter().map(|t| tri(f[t[0]], f[t[1]], f[t[2]]))
        })
        .flatten()
        .collect()
}

#[test]
fn triangles_round_trip_in_binary_and_ascii() {
    let mesh = bare(&full_mesh());
    for binary in [true, false] {
        for flip_faces in [false, true] {
            let options = WriteOptions {
                binary,
                flip_faces,
                ..WriteOptions::default()
            };
            let file = read(&stl(&[("part", &mesh)], options)[..]).unwrap();
            assert_eq!(
                corners(&file),
                [expected(&mesh, flip_faces)],
                "binary {binary}"
            );
            assert_eq!(
                file.objects[0].mesh.positions.len(),
                12,
                "three corners per triangle"
            );
            assert_eq!(file.notes.ignored["normals"], 4);
            assert_eq!(file.objects[0].name, if binary { "" } else { "part" });
        }
    }
    // ASCII keeps one solid per object; binary puts every triangle in one.
    let two = [("a", &mesh), ("b", &mesh)];
    let ascii = read(
        &stl(
            &two,
            WriteOptions {
                binary: false,
                ..WriteOptions::default()
            },
        )[..],
    )
    .unwrap();
    assert_eq!(ascii.objects.len(), 2);
    let binary = read(&stl(&two, WriteOptions::default())[..]).unwrap();
    assert_eq!(
        corners(&binary),
        [[expected(&mesh, false), expected(&mesh, false)].concat()]
    );
}

#[test]
fn binary_files_may_start_with_solid() {
    let mut bytes = stl(&[("", &bare(&full_mesh()))], WriteOptions::default());
    assert!(
        !bytes.starts_with(b"solid"),
        "our header never starts with solid"
    );
    bytes[..12].copy_from_slice(b"solid thing\n");
    let file = read(&bytes[..]).unwrap();
    assert_eq!(file.objects[0].mesh.faces.len(), 4);
    // Normals are recomputed from the corners: unit length, pointing out of the winding.
    let n: Vec<f32> = (0..3)
        .map(|i| f32::from_le_bytes(bytes[84 + 4 * i..88 + 4 * i].try_into().unwrap()))
        .collect();
    let length = n.iter().map(|x| x * x).sum::<f32>();
    assert!((length - 1.0).abs() < 1e-6, "{n:?}");
    // Colored attribute bytes are dropped and counted; an empty file holds no object.
    bytes[84 + 48] = 7;
    assert_eq!(
        read(&bytes[..]).unwrap().notes.ignored["attribute bytes (color)"],
        1
    );
    let empty = [vec![b' '; 80], vec![0; 4]].concat();
    assert!(read(&empty[..]).unwrap().objects.is_empty());
    assert!(
        read(&b"solid x\nendsolid x\n"[..])
            .unwrap()
            .objects
            .is_empty()
    );
}

#[test]
fn normals_point_out_of_the_winding() {
    let tilted = PlainMesh {
        positions: vec![[1.0; 3], [2.0, 3.0, 4.0], [5.0, 6.0, 8.0]],
        faces: vec![[0, 1, 2, NO]],
        attributes: std::collections::BTreeMap::new(),
    };
    let bytes = stl(&[("", &tilted)], WriteOptions::default());
    let n: Vec<f32> = (0..3)
        .map(|i| f32::from_le_bytes(bytes[84 + 4 * i..88 + 4 * i].try_into().unwrap()))
        .collect();
    // (1, 2, 3) x (4, 5, 7) = (-1, 5, -3), made unit length.
    let len = 35f64.sqrt();
    assert_eq!(n, [-1.0, 5.0, -3.0].map(|x: f64| (x / len) as f32));
}

#[test]
fn a_degenerate_triangle_gets_a_zero_normal() {
    let flat = PlainMesh {
        positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        faces: vec![[0, 1, 2, NO]],
        attributes: std::collections::BTreeMap::new(),
    };
    let text = String::from_utf8(stl(
        &[("", &flat)],
        WriteOptions {
            binary: false,
            ..WriteOptions::default()
        },
    ))
    .unwrap();
    assert!(text.contains("facet normal 0 0 0"), "{text}");
}

#[test]
fn damaged_files_are_named() {
    let ascii = "solid s\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid s\n";
    assert_eq!(read(ascii.as_bytes()).unwrap().objects.len(), 1);
    let cases = [
        (ascii.replace("vertex 1 0 0", "vertex 1 0"), 5),
        (ascii.replace("vertex 1 0 0", "vertex 1 0 nan"), 5),
        (ascii.replace("outer loop", "inner loop"), 3),
        (ascii.replace("vertex 0 1 0\n", ""), 6),
        (ascii.replace("endfacet\n", ""), 8),
        (ascii.replace("endsolid s\n", ""), 9),
        (format!("{ascii}junk\n"), 10),
    ];
    for (text, line) in cases {
        match read(text.as_bytes()) {
            Err(ReadError::BadLine { line: got, .. }) => assert_eq!(got, line, "{text}"),
            other => panic!("{text}: {other:?}"),
        }
    }
    let mut bytes = stl(&[("", &bare(&full_mesh()))], WriteOptions::default());
    bytes[84 + 12..88 + 12].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        read(&bytes[..]).unwrap_err(),
        ReadError::BadValue {
            offset: 84,
            reason: "triangle 0 has a corner that is not a finite point".into()
        }
    );
    bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        read(&bytes[..]),
        Err(ReadError::BadValue { offset: 80, .. })
    ));
    // The most triangles a mesh can hold is read as such (and then the file is too short).
    let mut bytes = stl(&[("", &bare(&full_mesh()))], WriteOptions::default());
    bytes[80..84].copy_from_slice(&((u32::MAX - 1) / 3).to_le_bytes());
    assert!(matches!(read(&bytes[..]), Err(ReadError::Truncated { .. })));
    bytes[80..84].copy_from_slice(&((u32::MAX - 1) / 3 + 1).to_le_bytes());
    assert!(matches!(
        read(&bytes[..]),
        Err(ReadError::BadValue { offset: 80, .. })
    ));
}

#[test]
fn every_cut_off_is_an_error_inside_the_file() {
    for binary in [true, false] {
        let bytes = stl(
            &[("s", &full_mesh())],
            WriteOptions {
                binary,
                ..WriteOptions::default()
            },
        );
        let lines = line_count(&bytes);
        for end in 0..bytes.len() {
            match read(&bytes[..end]) {
                Err(ReadError::Truncated { offset, .. }) => assert!(offset <= end, "cut at {end}"),
                Err(ReadError::BadLine { line, .. }) => {
                    assert!(!binary && line <= lines, "cut at {end}");
                }
                Err(e) => panic!("cut at {end}: {e}"),
                Ok(file) => {
                    assert!(
                        !binary
                            && consistent(&file)
                            && end + 1 >= bytes.len() - "endsolid s\n".len(),
                        "cut at {end} read"
                    );
                }
            }
        }
    }
}
