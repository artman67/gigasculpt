//! Writing PLY.

use std::io::{self, BufWriter, Write};

use gs_protocol::AttrDomain;

use crate::mesh::{COLOR, Object, WriteOptions, column, split};

/// Writes `objects` as one PLY file. See the module docs for what is written.
pub fn write(out: impl Write, objects: &[Object], options: &WriteOptions) -> io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 16, out);
    let colors: Vec<Option<&[u32]>> = (objects.iter())
        .map(|o| {
            let c = column::<u32>(o.mesh, COLOR, AttrDomain::Vertex);
            c.filter(|c| options.colors && c.len() == o.positions.len())
        })
        .collect();
    let any_color = colors.iter().any(Option::is_some);
    let vertices: usize = objects.iter().map(|o| o.positions.len()).sum();
    let faces: usize = (objects.iter())
        .flat_map(|o| {
            o.mesh
                .faces
                .iter()
                .map(|f| split(f, &o.mesh.positions, *options))
        })
        .map(|parts| parts.iter().filter(|c| c[0].is_some()).count())
        .sum();
    let format = if options.binary {
        "binary_little_endian"
    } else {
        "ascii"
    };
    writeln!(w, "ply\nformat {format} 1.0\ncomment written by GigaSculpt")?;
    writeln!(w, "element vertex {vertices}")?;
    writeln!(w, "property float x\nproperty float y\nproperty float z")?;
    if any_color {
        for c in ["red", "green", "blue", "alpha"] {
            writeln!(w, "property uchar {c}")?;
        }
    }
    writeln!(w, "element face {faces}")?;
    writeln!(w, "property list uchar int vertex_indices\nend_header")?;
    for (o, colors) in objects.iter().zip(&colors) {
        for (i, p) in o.positions.iter().enumerate() {
            let rgba = colors.map_or([0xFF; 4], |c| c[i].to_le_bytes());
            if options.binary {
                for x in p {
                    w.write_all(&x.to_le_bytes())?;
                }
                if any_color {
                    w.write_all(&rgba)?;
                }
            } else {
                write!(w, "{} {} {}", p[0], p[1], p[2])?;
                if any_color {
                    write!(w, " {} {} {} {}", rgba[0], rgba[1], rgba[2], rgba[3])?;
                }
                writeln!(w)?;
            }
        }
    }
    let mut base = 0u32;
    for o in objects {
        for face in &o.mesh.faces {
            let parts = split(face, &o.mesh.positions, *options);
            for corners in parts.into_iter().filter(|c| c[0].is_some()) {
                let mut ids = [0u32; 4];
                let mut n = 0;
                for k in corners.into_iter().flatten() {
                    ids[n] = base + face[k];
                    n += 1;
                }
                if options.binary {
                    w.write_all(&[n as u8])?;
                    for v in &ids[..n] {
                        w.write_all(&v.to_le_bytes())?;
                    }
                } else {
                    write!(w, "{n}")?;
                    for v in &ids[..n] {
                        write!(w, " {v}")?;
                    }
                    writeln!(w)?;
                }
            }
        }
        base += o.positions.len() as u32;
    }
    w.flush()
}
