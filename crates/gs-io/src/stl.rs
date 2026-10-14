//! STL, binary and ASCII, following the public format description.
//!
//! Read: every triangle's three corners as three vertices of their own (STL shares none;
//! welding is the importer's job); normals and the binary attribute bytes are dropped and
//! counted in the notes. A binary file is one object; each ASCII `solid` is one, named. A file
//! is ASCII when it starts with `solid` and its next line starts a facet or ends the solid
//! (binary headers may start with `solid` too).
//!
//! Write: triangles only (quads split), with unit normals, binary by default (an 80-byte header
//! that does not start with `solid`), or ASCII with one `solid` per object.

use std::io::{self, BufReader, BufWriter, Read, Write};

use crate::ReadError;
use crate::mesh::{Builder, Lines, MeshFile, Notes, Object, WriteOptions, finite, split, words};

/// Bytes read ahead to tell ASCII from binary.
const HEAD: u64 = 4096;
/// Header and triangle count of a binary file.
const BINARY_HEAD: usize = 84;
/// Bytes per binary triangle: normal, three corners, attribute bytes.
const TRIANGLE: usize = 50;

/// Reads an STL file. See the module docs for what is kept.
pub fn read(mut source: impl Read) -> Result<MeshFile, ReadError> {
    let mut head = Vec::new();
    let io = |e: io::Error| ReadError::Io {
        offset: 0,
        kind: e.kind(),
    };
    source
        .by_ref()
        .take(HEAD)
        .read_to_end(&mut head)
        .map_err(io)?;
    let ascii = head.starts_with(b"solid") && {
        let next = head.split(|&b| b == b'\n').nth(1);
        let word = next.and_then(|rest| words(rest).next());
        matches!(word, Some(b"facet" | b"endsolid"))
    };
    let src = io::Cursor::new(head).chain(source);
    if ascii {
        read_ascii(Lines::new(BufReader::new(src)))
    } else {
        read_binary(BufReader::with_capacity(1 << 16, src))
    }
}

fn read_binary(mut src: impl Read) -> Result<MeshFile, ReadError> {
    let mut offset = 0usize;
    let mut take = |buf: &mut [u8], what| {
        src.read_exact(buf).map_err(|e| match e.kind() {
            io::ErrorKind::UnexpectedEof => ReadError::Truncated { offset, what },
            kind => ReadError::Io { offset, kind },
        })?;
        offset += buf.len();
        Ok::<usize, ReadError>(offset)
    };
    let mut head = [0u8; BINARY_HEAD];
    take(&mut head, "STL header")?;
    let count = u32::from_le_bytes([head[80], head[81], head[82], head[83]]);
    if count > (u32::MAX - 1) / 3 {
        return Err(ReadError::BadValue {
            offset: 80,
            reason: format!("{count} triangles are more than a mesh can hold"),
        });
    }
    let mut b = Builder::new(String::new());
    let mut notes = Notes::default();
    let mut tri = [0u8; TRIANGLE];
    for t in 0..count {
        let at = take(&mut tri, "STL triangle")? - TRIANGLE;
        // Twelve little-endian floats: the normal, then three corners.
        let words = tri.as_chunks::<4>().0;
        for corner in 0..3 {
            let p = [3, 4, 5].map(|k| f32::from_le_bytes(words[k + 3 * corner]));
            if p.iter().any(|x| !x.is_finite()) {
                return Err(ReadError::BadValue {
                    offset: at as u64,
                    reason: format!("triangle {t} has a corner that is not a finite point"),
                });
            }
            b.positions.push(p);
        }
        if tri[48..] != [0, 0] {
            notes.ignore("attribute bytes (color)", 1);
        }
        b.add_polygon(&[3 * t, 3 * t + 1, 3 * t + 2], None, None, &mut notes);
    }
    notes.ignore("normals", u64::from(count));
    let objects = (count > 0)
        .then(|| b.finish(None, false))
        .into_iter()
        .collect();
    Ok(MeshFile { objects, notes })
}

/// Where an ASCII reader stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum At {
    Outside,
    Solid,
    Facet,
    Loop,
}

fn read_ascii(mut lines: Lines<impl io::BufRead>) -> Result<MeshFile, ReadError> {
    let mut objects = Vec::new();
    let mut notes = Notes::default();
    let mut b = Builder::new(String::new());
    let mut at = At::Outside;
    let mut corners: Vec<u32> = Vec::new();
    lines.each(|n, line| {
        let bad = |reason: &str| ReadError::BadLine {
            line: n,
            reason: reason.into(),
        };
        let mut w = words(line);
        let Some(key) = w.next() else {
            return Ok(true);
        };
        at = match (at, key) {
            (At::Outside, b"solid") => {
                let name = line
                    .trim_ascii_start()
                    .get(5..)
                    .unwrap_or_default()
                    .trim_ascii();
                b = Builder::new(String::from_utf8_lossy(name).into_owned());
                At::Solid
            }
            (At::Solid, b"facet") => {
                notes.ignore("normals", 1);
                At::Facet
            }
            (At::Facet, b"outer") => {
                corners.clear();
                At::Loop
            }
            (At::Loop, b"vertex") => {
                let p: Vec<f32> = w.map(finite).collect::<Option<_>>().unwrap_or_default();
                let p: [f32; 3] = p
                    .try_into()
                    .map_err(|_| bad("a vertex needs 3 finite numbers"))?;
                let v = u32::try_from(b.positions.len());
                corners.push(v.map_err(|_| bad("more vertices than a mesh can hold"))?);
                b.positions.push(p);
                At::Loop
            }
            (At::Loop, b"endloop") if corners.len() >= 3 => {
                b.add_polygon(&corners, None, None, &mut notes);
                At::Facet
            }
            (At::Facet, b"endfacet") => At::Solid,
            (At::Solid, b"endsolid") => {
                let done = std::mem::take(&mut b);
                if done.face_count() > 0 {
                    objects.push(done.finish(None, false));
                }
                At::Outside
            }
            _ => return Err(bad("a line out of place in an ASCII STL file")),
        };
        Ok(true)
    })?;
    if at != At::Outside {
        return Err(ReadError::BadLine {
            line: lines.line + 1,
            reason: "the file ends before endsolid".into(),
        });
    }
    Ok(MeshFile { objects, notes })
}

/// The unit normal of a triangle, or zero when it has no area (worked out in `f64`, so huge
/// coordinates do not overflow).
fn normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let d = |p: [f32; 3]| [0, 1, 2].map(|i| f64::from(p[i]) - f64::from(a[i]));
    let (u, v) = (d(b), d(c));
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        n.map(|x| (x / len) as f32)
    } else {
        [0.0; 3]
    }
}

/// The triangles of `o`, each as its three corner positions.
fn triangles<'a>(
    o: &'a Object,
    options: &'a WriteOptions,
) -> impl Iterator<Item = [[f32; 3]; 3]> + 'a {
    let options = WriteOptions {
        triangles: true,
        ..*options
    };
    o.mesh.faces.iter().flat_map(move |face| {
        let parts =
            (split(face, &o.mesh.positions, options).into_iter()).filter(|c| c[0].is_some());
        parts.map(move |c| [0, 1, 2].map(|k| o.positions[face[c[k].unwrap_or(0)] as usize]))
    })
}

/// Writes `objects` as one STL file. See the module docs for what is written.
pub fn write(out: impl Write, objects: &[Object], options: &WriteOptions) -> io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 16, out);
    if options.binary {
        let count: usize = objects.iter().map(|o| triangles(o, options).count()).sum();
        let count = u32::try_from(count)
            .map_err(|_| io::Error::other("more triangles than a binary STL file holds"))?;
        let mut head = [b' '; 80];
        head[..21].copy_from_slice(b"written by GigaSculpt");
        w.write_all(&head)?;
        w.write_all(&count.to_le_bytes())?;
        for o in objects {
            for [a, b, c] in triangles(o, options) {
                for x in [normal(a, b, c), a, b, c].iter().flatten() {
                    w.write_all(&x.to_le_bytes())?;
                }
                w.write_all(&[0, 0])?;
            }
        }
    } else {
        for o in objects {
            let name: String = o.name.chars().filter(|c| !c.is_control()).collect();
            writeln!(w, "solid {name}")?;
            for [a, b, c] in triangles(o, options) {
                let n = normal(a, b, c);
                writeln!(w, "facet normal {} {} {}\nouter loop", n[0], n[1], n[2])?;
                for p in [a, b, c] {
                    writeln!(w, "vertex {} {} {}", p[0], p[1], p[2])?;
                }
                writeln!(w, "endloop\nendfacet")?;
            }
            writeln!(w, "endsolid {name}")?;
        }
    }
    w.flush()
}
