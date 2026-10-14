//! Writing OBJ.

use std::io::{self, BufWriter, Write};

use gs_protocol::AttrDomain;

use crate::mesh::{COLOR, GROUP, MASK, Object, Uvs, WriteOptions, column, split};

/// Entries per `#MRGB` line, as sculpting apps write them.
const MRGB_PER_LINE: usize = 64;

/// Writes `objects` as one OBJ file. See the module docs for what is written.
pub fn write(out: impl Write, objects: &[Object], options: &WriteOptions) -> io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 16, out);
    writeln!(w, "# written by GigaSculpt")?;
    let (mut v_base, mut vt_base) = (1u64, 1u64);
    for o in objects {
        if objects.len() > 1 || !o.name.is_empty() {
            writeln!(w, "o {}", o.name)?;
        }
        let nv = o.positions.len();
        let vertex =
            |name| column::<u32>(o.mesh, name, AttrDomain::Vertex).filter(|c| c.len() == nv);
        let colors = vertex(COLOR).filter(|_| options.colors);
        for (i, p) in o.positions.iter().enumerate() {
            write!(w, "v {} {} {}", p[0], p[1], p[2])?;
            if let Some(c) = colors {
                let [r, g, b, _] = c[i].to_le_bytes().map(|x| f32::from(x) / 255.0);
                write!(w, " {r} {g} {b}")?;
            }
            writeln!(w)?;
        }
        let mask = column::<f32>(o.mesh, MASK, AttrDomain::Vertex).filter(|m| m.len() == nv);
        if options.mask && (mask.is_some() || colors.is_some()) {
            mrgb(&mut w, nv, mask, colors)?;
        }
        let uvs = Uvs::of(o.mesh).filter(|_| options.uvs);
        let table = uvs.map(|uvs| uv_table(o.mesh.faces.len(), uvs));
        if let Some((unique, _)) = &table {
            for &(u, v) in unique {
                writeln!(w, "vt {} {}", f32::from_bits(u), f32::from_bits(v))?;
            }
        }
        let groups = (column::<u32>(o.mesh, GROUP, AttrDomain::Face))
            .filter(|g| options.groups && g.len() == o.mesh.faces.len());
        let mut last_group = None;
        for (f, face) in o.mesh.faces.iter().enumerate() {
            if let Some(g) = groups.map(|g| g[f]).filter(|&g| last_group != Some(g)) {
                writeln!(w, "g Group{g}")?;
                last_group = Some(g);
            }
            let uv_ids = table.as_ref().map(|(_, ids)| &ids[f * 4..f * 4 + 4]);
            let parts = split(face, &o.mesh.positions, *options);
            for corners in parts.into_iter().filter(|c| c[0].is_some()) {
                w.write_all(b"f")?;
                for k in corners.into_iter().flatten() {
                    write!(w, " {}", v_base + u64::from(face[k]))?;
                    if let Some(ids) = uv_ids {
                        write!(w, "/{}", vt_base + u64::from(ids[k]))?;
                    }
                }
                writeln!(w)?;
            }
        }
        v_base += nv as u64;
        vt_base += table.map_or(0, |(unique, _)| unique.len() as u64);
    }
    w.flush()
}

/// The `#MRGB` block: per vertex the mask byte (255 not masked), then red, green and blue
/// (white without colors).
fn mrgb(
    w: &mut impl Write,
    nv: usize,
    mask: Option<&[f32]>,
    colors: Option<&[u32]>,
) -> io::Result<()> {
    writeln!(
        w,
        "# mask and color per vertex as MMRRGGBB, the form sculpting apps read"
    )?;
    for start in (0..nv).step_by(MRGB_PER_LINE) {
        w.write_all(b"#MRGB ")?;
        for i in start..nv.min(start + MRGB_PER_LINE) {
            let m = mask.map_or(0.0, |m| m[i]).clamp(0.0, 1.0);
            let [r, g, b, _] = colors.map_or([0xFF; 4], |c| c[i].to_le_bytes());
            let mm = ((1.0 - m) * 255.0).round() as u8;
            write!(w, "{mm:02x}{r:02x}{g:02x}{b:02x}")?;
        }
        writeln!(w)?;
    }
    Ok(())
}

/// The distinct UVs (as the bits of `u` and `v`, sorted) and, per face corner, the index of
/// its UV among them.
fn uv_table(faces: usize, uvs: Uvs) -> (Vec<(u32, u32)>, Vec<u32>) {
    let key = |c: usize| uvs.face(c / 4)[c % 4].map(f32::to_bits).into();
    let mut unique: Vec<(u32, u32)> = (0..faces * 4).map(key).collect();
    gs_par::sort(&mut unique);
    unique.dedup();
    let ids = (0..faces * 4)
        .map(|c| unique.partition_point(|&u| u < key(c)) as u32)
        .collect();
    (unique, ids)
}
