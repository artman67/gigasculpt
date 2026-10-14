//! Making a file's mesh fit the scene: which columns to keep, the axes and scale, merging
//! vertices that share a position, and dropping faces that use a vertex twice.

use gs_io::mesh::{COLOR, GROUP, MASK};
use gs_protocol::{AttrDomain, AttrValues, NO_VERTEX, PlainMesh};

use super::{UpAxis, mirror};
use crate::mesh::corners;

/// Which columns an import keeps.
#[derive(Clone, Copy)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one switch per kind of data, as the import options name them"
)]
pub(super) struct Keep {
    pub groups: bool,
    pub uvs: bool,
    pub colors: bool,
    pub mask: bool,
}

/// Drops the columns `keep` turns off.
pub(super) fn keep_columns(mesh: &mut PlainMesh, keep: Keep) {
    // The readers make these columns and the UV columns, nothing else.
    mesh.attributes.retain(|name, _| match name.as_str() {
        GROUP => keep.groups,
        COLOR => keep.colors,
        MASK => keep.mask,
        _ => keep.uvs,
    });
}

/// File positions to scene positions: mirrored across the planes `flip` marks, turned so the
/// file's up axis is the scene's, then scaled.
pub(super) fn place(mesh: &mut PlainMesh, flip: [bool; 3], up: UpAxis, scale: f32) {
    for p in &mut mesh.positions {
        *p = up.to_scene(mirror(*p, flip)).map(|x| x * scale);
    }
}

/// Keeps the faces `keep` says yes to, with their face columns; returns how many went.
fn retain_faces(mesh: &mut PlainMesh, keep: impl Fn(&[u32; 4]) -> bool) -> u64 {
    let kept: Vec<bool> = mesh.faces.iter().map(keep).collect();
    let before = mesh.faces.len();
    let mut i = 0;
    mesh.faces.retain(|_| {
        i += 1;
        kept[i - 1]
    });
    for a in mesh.attributes.values_mut() {
        if a.domain == AttrDomain::Face {
            filter(&mut a.values, &kept);
        }
    }
    (before - mesh.faces.len()) as u64
}

/// Keeps the values `kept` marks.
fn filter(values: &mut AttrValues, kept: &[bool]) {
    fn keep<T: Copy>(v: &mut Vec<T>, kept: &[bool]) {
        let mut i = 0;
        v.retain(|_| {
            i += 1;
            kept[i - 1]
        });
    }
    match values {
        AttrValues::F32(v) => keep(v, kept),
        AttrValues::U32(v) => keep(v, kept),
    }
}

/// Drops faces that use a vertex twice, which a part cannot hold (merging vertices makes them
/// from triangles that had two corners at one place); returns how many went. Faces with no
/// area stay.
pub(super) fn drop_repeated(mesh: &mut PlainMesh) -> u64 {
    retain_faces(mesh, |f| {
        let c = corners(f);
        !(0..c.len()).any(|i| c[i + 1..].contains(&c[i]))
    })
}

/// Merges vertices at the same position (`0` and `-0` count as one), each group into its
/// lowest-numbered vertex, which keeps its values; returns how many vertices went. Faces that
/// end up using a vertex twice are left for [`drop_repeated`].
pub(super) fn merge(mesh: &mut PlainMesh) -> u64 {
    let key = |v: u32| mesh.positions[v as usize].map(|x| (x + 0.0).to_bits());
    let mut order: Vec<([u32; 3], u32)> = (0..mesh.positions.len() as u32)
        .map(|v| (key(v), v))
        .collect();
    gs_par::sort(&mut order);
    let mut rep: Vec<u32> = vec![NO_VERTEX; mesh.positions.len()];
    for (i, &(k, v)) in order.iter().enumerate() {
        rep[v as usize] = match i.checked_sub(1).map(|j| order[j]) {
            Some((before, w)) if before == k => rep[w as usize],
            _ => v,
        };
    }
    drop(order);
    let kept: Vec<bool> = (0..rep.len()).map(|v| rep[v] as usize == v).collect();
    let mut new = vec![0u32; rep.len()];
    let mut next = 0u32;
    for v in 0..rep.len() {
        if kept[v] {
            new[v] = next;
            next += 1;
        }
    }
    for f in &mut mesh.faces {
        let n = corners(f).len();
        for v in &mut f[..n] {
            *v = new[rep[*v as usize] as usize];
        }
    }
    let mut i = 0;
    mesh.positions.retain(|_| {
        i += 1;
        kept[i - 1]
    });
    for a in mesh.attributes.values_mut() {
        if a.domain == AttrDomain::Vertex {
            filter(&mut a.values, &kept);
        }
    }
    (rep.len() - next as usize) as u64
}
