//! One object as a reader collects it: positions, faces and the columns, filled in face by
//! face, with polygons split into triangles.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};

use super::{COLOR, GROUP, MASK, MeshObject, Notes, UV_COLUMNS};

/// The color a vertex gets when others in its object have one and it has none.
const WHITE: u32 = u32::MAX;

/// An object being read.
#[derive(Default)]
pub(crate) struct Builder {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    faces: Vec<[u32; 4]>,
    uv: Option<[Vec<f32>; 8]>,
    group: Option<Vec<u32>>,
    color: Option<Vec<u32>>,
    mask: Option<Vec<f32>>,
    /// The faces each split polygon became, as `start..end`.
    polygons: Vec<(usize, usize)>,
}

/// `column`, made on first use and filled with `fill` up to `len` values, then `value` at
/// `index`.
fn put<T: Copy>(column: &mut Option<Vec<T>>, len: usize, index: usize, value: T, fill: T) {
    let values = column.get_or_insert_with(Vec::new);
    values.resize(values.len().max(len), fill);
    values[index] = value;
}

impl Builder {
    pub fn new(name: String) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Sets vertex `v`'s color (packed RGBA) and mask, where given.
    pub fn paint_vertex(&mut self, v: usize, color: Option<u32>, mask: Option<f32>) {
        let n = self.positions.len();
        if let Some(c) = color {
            put(&mut self.color, n, v, c, WHITE);
        }
        if let Some(m) = mask {
            put(&mut self.mask, n, v, m, 0.0);
        }
    }

    /// The color and mask of vertex `v`, if set, to copy it into another object.
    pub fn paint(&self, v: usize) -> (Option<u32>, Option<f32>) {
        let color = self.color.as_ref().and_then(|c| c.get(v).copied());
        (color, self.mask.as_ref().and_then(|m| m.get(v).copied()))
    }

    /// Adds a polygon over vertices already added (the caller checks the indices), with the
    /// UVs of its corners and its face group if it has them. More than four corners become a
    /// fan of triangles from the corner lowest in y, then z, then distance from x = 0, so a
    /// polygon and its mirror image across x = 0 split alike.
    pub fn add_polygon(
        &mut self,
        corners: &[u32],
        uvs: Option<&[[f32; 2]]>,
        group: Option<u32>,
        notes: &mut Notes,
    ) {
        let n = corners.len();
        if n <= 4 {
            let mut face = [NO_VERTEX; 4];
            face[..n].copy_from_slice(corners);
            let mut uv = [[0.0; 2]; 4];
            if let Some(uvs) = uvs {
                uv[..n].copy_from_slice(uvs);
            }
            self.push(face, uvs.map(|_| uv), group);
            return;
        }
        notes.polygons_split += 1;
        let key = |k: usize| {
            let p = self.positions[corners[k] as usize];
            [p[1], p[2], p[0].abs()]
        };
        let order = |a: usize, b: usize| {
            let (ka, kb) = (key(a), key(b));
            (ka.iter().zip(&kb).map(|(x, y)| x.total_cmp(y)))
                .find(|o| o.is_ne())
                .unwrap_or(Ordering::Equal)
        };
        let apex = (0..n).min_by(|&a, &b| order(a, b)).unwrap_or(0);
        let start = self.faces.len();
        for k in 1..n - 1 {
            let at = [apex, (apex + k) % n, (apex + k + 1) % n];
            let face = [corners[at[0]], corners[at[1]], corners[at[2]], NO_VERTEX];
            let uv = uvs.map(|u| [u[at[0]], u[at[1]], u[at[2]], [0.0; 2]]);
            self.push(face, uv, group);
        }
        self.polygons.push((start, self.faces.len()));
    }

    /// Replaces every face group id `g` with `ids[g]`.
    pub fn renumber_groups(&mut self, ids: &[u32]) {
        for g in self.group.iter_mut().flatten() {
            *g = ids[*g as usize];
        }
    }

    fn push(&mut self, face: [u32; 4], uv: Option<[[f32; 2]; 4]>, group: Option<u32>) {
        let f = self.faces.len();
        self.faces.push(face);
        if let Some(uv) = uv {
            let cols = self.uv.get_or_insert_with(Default::default);
            for (k, col) in cols.iter_mut().enumerate() {
                col.resize(f, 0.0);
                col.push(uv[k / 2][k % 2]);
            }
        }
        if let Some(group) = group {
            put(&mut self.group, f + 1, f, group, 0);
        }
    }

    /// The object, with its columns filled out to full length. `group_names` names its face
    /// groups (`None` drops the group column); `group_polygons` gives each split polygon a
    /// group of its own, numbered after the highest group.
    pub fn finish(
        self,
        group_names: Option<BTreeMap<u32, String>>,
        group_polygons: bool,
    ) -> MeshObject {
        let (nv, nf) = (self.positions.len(), self.faces.len());
        let mut attributes = BTreeMap::new();
        let mut column = |name: &str, domain, values| {
            attributes.insert(name.to_string(), Attribute { domain, values });
        };
        let mut group = self.group.filter(|_| group_names.is_some());
        if group_polygons && !self.polygons.is_empty() {
            let groups = group.get_or_insert_with(Vec::new);
            groups.resize(nf, 0);
            let first = groups.iter().max().map_or(0, |&m| m + 1);
            for (id, &(from, to)) in (first..).zip(&self.polygons) {
                groups[from..to].fill(id);
            }
        }
        if let Some(mut g) = group {
            g.resize(nf, 0);
            column(GROUP, AttrDomain::Face, AttrValues::U32(g));
        }
        if let Some(mut c) = self.color {
            c.resize(nv, WHITE);
            column(COLOR, AttrDomain::Vertex, AttrValues::U32(c));
        }
        if let Some(mut m) = self.mask {
            m.resize(nv, 0.0);
            column(MASK, AttrDomain::Vertex, AttrValues::F32(m));
        }
        for (name, mut values) in UV_COLUMNS.into_iter().zip(self.uv.into_iter().flatten()) {
            values.resize(nf, 0.0);
            column(name, AttrDomain::Face, AttrValues::F32(values));
        }
        MeshObject {
            name: self.name,
            mesh: PlainMesh {
                positions: self.positions,
                faces: self.faces,
                attributes,
            },
            group_names: group_names.unwrap_or_default(),
        }
    }
}
