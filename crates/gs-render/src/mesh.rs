//! The renderer's input mesh and its CPU-side preparation for upload: triangulation, normals,
//! octahedral encoding, chunks with cache-optimized index order and their culling bounds.
//! The `MeshStore` path (`nodes.rs`) cuts and assembles chunks with the same functions.

use std::collections::BTreeSet;

use crate::cull::{ChunkCull, Cone, ConeBuilder};
use crate::error::RenderError;
use crate::math::{V3, above, add, cross, dot, normalize_or, sub};
use crate::streams::{COLOR, WHITE};
use bytemuck::{Pod, Zeroable};
use gs_core::groups::group_color;
use gs_protocol::{
    AttrDomain, AttrValues, GROUP, HIDE, MASK, PlainMesh, arity, corners, quad_diagonal,
    quad_triangles,
};

/// What the renderer draws: a borrowed [`PlainMesh`], plus optional smooth normals.
///
/// Faces are triangles (`NO_VERTEX` in the 4th slot) and quads. A quad is drawn as the two
/// triangles `gs_protocol::face_triangles` gives, split along its shorter diagonal (decision
/// 0040, section 2); its diagonal is never wireframed. Faces whose `hide` column
/// is set are not drawn (decision 0015); the `mask`, `color` (vertex) and `group` (face)
/// columns are drawn as a store's are (decision 0039, section 9); other columns are not drawn.
#[derive(Clone, Copy, Debug)]
pub struct RenderMesh<'a> {
    mesh: &'a PlainMesh,
    normals: Option<&'a [[f32; 3]]>,
}

impl<'a> From<&'a PlainMesh> for RenderMesh<'a> {
    /// Draws `mesh` with area-weighted normals computed on upload.
    fn from(mesh: &'a PlainMesh) -> Self {
        RenderMesh {
            mesh,
            normals: None,
        }
    }
}

impl<'a> RenderMesh<'a> {
    /// Draws with these smooth normals, one per position, instead of computed ones.
    #[must_use]
    pub fn with_normals(self, normals: &'a [[f32; 3]]) -> Self {
        RenderMesh {
            normals: Some(normals),
            ..self
        }
    }
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Bounds {
    /// Contains nothing: `include` of any point makes it that point.
    pub const EMPTY: Bounds = Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };

    /// Grows the box to contain `p`. A NaN coordinate is ignored.
    pub fn include(&mut self, p: [f32; 3]) {
        // Plain comparisons (one instruction each) rather than `f32::min`, whose NaN rule costs
        // several; a NaN fails both comparisons, so it is ignored all the same.
        for ((lo, hi), v) in self.min.iter_mut().zip(&mut self.max).zip(p) {
            if v < *lo {
                *lo = v;
            }
            if v > *hi {
                *hi = v;
            }
        }
    }

    /// Grows the box to contain `other`. An empty `other` (a node left empty by upkeep) changes
    /// nothing: its corners lie at infinity, so it is joined per side, not as two points.
    pub fn union(&mut self, other: &Bounds) {
        for i in 0..3 {
            if other.min[i] < self.min[i] {
                self.min[i] = other.min[i];
            }
            if other.max[i] > self.max[i] {
                self.max[i] = other.max[i];
            }
        }
    }

    /// The eight corners, or none for an empty box.
    pub fn corners(&self) -> Vec<[f32; 3]> {
        if self.min[0] > self.max[0] {
            return Vec::new();
        }
        let ([a, b, c], [x, y, z]) = (self.min, self.max);
        vec![
            [a, b, c],
            [x, b, c],
            [a, y, c],
            [x, y, c],
            [a, b, z],
            [x, b, z],
            [a, y, z],
            [x, y, z],
        ]
    }
}

/// A run of triangles in the index pool, the unit of drawing and culling: a fixed number of
/// triangles of a `PlainMesh`, or the faces of one node of a `MeshStore`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    /// Index of the chunk's first triangle in the index pool (its first index is 3 times this).
    pub first_triangle: u32,
    pub triangle_count: u32,
    /// Bounds of every vertex the chunk's triangles use; a store node's also hold its own
    /// live vertices that none of its faces use.
    pub bounds: Bounds,
}

/// How [`crate::Renderer::upload`] cuts the mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadOptions {
    /// Triangles per chunk. Every chunk but the last has exactly this many.
    pub chunk_triangles: u32,
}

impl Default for UploadOptions {
    /// 16,384 triangles: about one sculpt node of 8K vertices (ARCHITECTURE section 3.1).
    fn default() -> Self {
        UploadOptions {
            chunk_triangles: 16_384,
        }
    }
}

/// One vertex in the vertex pool: position and octahedral normal, 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct GpuVertex {
    pub position: [f32; 3],
    /// Two snorm16 values, x in the low half.
    pub normal: u32,
}

impl GpuVertex {
    pub fn new(position: V3, normal: V3) -> Self {
        GpuVertex {
            position,
            normal: pack(encode_normal(normal)),
        }
    }
}

/// Everything the GPU pools need but the vertices, built on the CPU.
pub(crate) struct CpuPools {
    pub indices: Vec<u32>,
    /// Hidden-edge masks, 4 bits per triangle, 8 triangles per word. Bit k hides the edge
    /// opposite the triangle's vertex k (a quad's diagonal).
    pub hidden_edges: Vec<u32>,
    pub chunks: Vec<Chunk>,
    /// One culling record per chunk.
    pub cull: Vec<ChunkCull>,
    /// Chunks that draw no triangle: every face hidden.
    pub blank: BTreeSet<u32>,
    /// The mask stream, one [`mask_byte`] per vertex slot, padded to whole words; `None` for a
    /// mesh without a mask.
    pub mask: Option<Vec<u8>>,
    /// The point color stream, one `Rgba8` word per vertex slot; `None` without colors.
    pub colors: Option<Vec<u32>>,
    /// The group color stream, one word per triangle of the index pool (white in the room
    /// past a chunk's triangles); `None` without groups.
    pub groups: Option<Vec<u32>>,
}

/// Each triangle's face group color as a word (`gs_core::groups::group_color`, seed 0, as
/// Blender's default), for faces given with their group ids, in [`triangulate`]'s order: a
/// quad's two triangles take its face's.
pub(crate) fn group_words<'f>(faces: impl IntoIterator<Item = (&'f [u32; 4], u32)>) -> Vec<u32> {
    // Neighboring faces mostly share a group, so the last color is kept.
    let mut last = (0, WHITE);
    let mut words = Vec::new();
    for (face, id) in faces {
        if id != last.0 {
            last = (id, u32::from_le_bytes(group_color(id, 0)));
        }
        words.push(last.1);
        if arity(*face) == 4 {
            words.push(last.1);
        }
    }
    words
}

/// A mask value as the GPU's mask stream holds it (decision 0015): 0 free to 255 fully masked.
/// Not a number masks fully, as brushes read it.
pub(crate) fn mask_byte(m: f32) -> u8 {
    if m.is_nan() {
        return 255;
    }
    // A non-negative float truncates like `floor`, so adding a half rounds.
    (m.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Faces cut into triangles: quads into two, each triangle with the edge a quad's diagonal
/// hides and whether its face is drawn (not hidden), and its group color when the mesh shows
/// groups (empty otherwise; see [`group_words`]).
#[derive(Default)]
pub(crate) struct Triangles {
    pub tris: Vec<[u32; 3]>,
    pub edges: Vec<u8>,
    pub shown: Vec<bool>,
    pub colors: Vec<u32>,
}

/// One chunk before it is placed in the pools: its triangles in drawing order with their
/// hidden edges, whether each is drawn and their group colors (empty without groups), the
/// bounds of the vertices they use, and their normal cone.
pub(crate) struct ChunkPart {
    tris: Vec<[u32; 3]>,
    hidden: Vec<u8>,
    shown: Vec<bool>,
    colors: Vec<u32>,
    bounds: Bounds,
    cone: Cone,
}

impl ChunkPart {
    /// A chunk of `t`'s triangles in the order `t` has them, culled by `bounds` and `cone`.
    pub fn whole(t: Triangles, bounds: Bounds, cone: Cone) -> ChunkPart {
        ChunkPart {
            tris: t.tris,
            hidden: t.edges,
            shown: t.shown,
            colors: t.colors,
            bounds,
            cone,
        }
    }

    /// Its triangles' group colors, in drawing order; empty without groups.
    pub fn group_words(&self) -> &[u32] {
        &self.colors
    }

    /// Whether any of its triangles is drawn.
    pub fn drawn(&self) -> bool {
        self.shown.contains(&true)
    }

    /// How many triangles it has, drawn or not.
    pub fn triangles(&self) -> u32 {
        self.tris.len() as u32
    }

    /// Its hidden-edge words, for a chunk whose first triangle is a multiple of 8, so that it
    /// owns whole words.
    pub fn hidden_words(&self) -> Vec<u32> {
        let mut words = vec![0u32; self.hidden.len().div_ceil(8)];
        for (t, &h) in self.hidden.iter().enumerate() {
            words[t / 8] |= u32::from(h) << ((t % 8) * 4);
        }
        words
    }

    /// Its triangles' corners as `slot` numbers them, three per triangle. A hidden face's
    /// triangles are written with one corner three times: they keep their place in the chunk,
    /// so showing the face again rewrites only this chunk, and they draw nothing.
    pub fn indices(&self, slot: impl Fn(u32) -> u32) -> impl Iterator<Item = u32> {
        (self.tris.iter().zip(&self.shown))
            .flat_map(move |(t, &shown)| if shown { *t } else { [t[0]; 3] }.map(&slot))
    }
}

/// Bounds and normal cone of a `PlainMesh` chunk's triangles, the cone around the sum of the
/// normals of their corners.
fn measure(tris: &[[u32; 3]], pos: impl Fn(u32) -> V3, nrm: impl Fn(u32) -> V3) -> (Bounds, Cone) {
    let mut bounds = Bounds::EMPTY;
    let mut axis = [0.0; 3];
    for &v in tris.iter().flatten() {
        bounds.include(pos(v));
        axis = add(axis, nrm(v));
    }
    let mut cone = ConeBuilder::new(axis);
    for &[a, b, c] in tris {
        cone.add(pos(a), pos(b), pos(c));
    }
    (bounds, cone.finish())
}

/// One chunk's triangles (`range` of `t`) reordered for the vertex cache, with the bounds and
/// cone measured for them. The order depends on the triangles alone, never on which are shown,
/// so hiding faces never moves a triangle.
pub(crate) fn cut(
    t: &Triangles,
    range: std::ops::Range<usize>,
    bounds: Bounds,
    cone: Cone,
) -> ChunkPart {
    let tris = &t.tris[range.clone()];
    let order = optimize_chunk(tris);
    let colors = if t.colors.is_empty() {
        Vec::new()
    } else {
        order.iter().map(|&k| t.colors[range.start + k]).collect()
    };
    ChunkPart {
        tris: order.iter().map(|&k| tris[k]).collect(),
        hidden: order.iter().map(|&k| t.edges[range.start + k]).collect(),
        shown: order.iter().map(|&k| t.shown[range.start + k]).collect(),
        colors,
        bounds,
        cone,
    }
}

/// Lays the chunks one after another in the index pool, writing each corner `v` as `slot(v)`.
/// A chunk of `t` triangles gets `room(t)` of the pool (at least `t`); the rest of its range
/// holds zeros and is not drawn.
pub(crate) fn assemble(
    parts: &[ChunkPart],
    slot: impl Fn(u32) -> u32,
    room: impl Fn(u32) -> u32,
) -> CpuPools {
    let total: usize = parts.iter().map(|p| room(p.triangles()) as usize).sum();
    let mut indices = Vec::with_capacity(total * 3);
    let mut hidden_edges = vec![0u32; total.div_ceil(8)];
    let grouped = parts.iter().any(|p| !p.colors.is_empty());
    let mut groups = grouped.then(|| vec![WHITE; total]);
    let (mut chunks, mut cull, mut blank) = (Vec::new(), Vec::new(), BTreeSet::new());
    let mut at = 0;
    for (n, part) in parts.iter().enumerate() {
        let (first, count) = (at, part.triangles());
        at += room(count);
        indices.resize(first as usize * 3, 0);
        for (k, &h) in part.hidden.iter().enumerate() {
            let t = first as usize + k;
            hidden_edges[t / 8] |= u32::from(h) << ((t % 8) * 4);
        }
        if let Some(groups) = &mut groups {
            groups[first as usize..][..part.colors.len()].copy_from_slice(&part.colors);
        }
        indices.extend(part.indices(&slot));
        let drawn = if part.drawn() { count } else { 0 };
        cull.push(ChunkCull::new(first, drawn, &part.bounds, part.cone));
        if !part.drawn() {
            blank.insert(n as u32);
        }
        chunks.push(Chunk {
            first_triangle: first,
            triangle_count: count,
            bounds: part.bounds,
        });
    }
    indices.resize(total * 3, 0);
    CpuPools {
        indices,
        hidden_edges,
        chunks,
        cull,
        blank,
        mask: None,
        colors: None,
        groups,
    }
}

/// Quad halves, split either way: the first hides the diagonal opposite its vertex 1, the
/// second opposite its vertex 2 (`gs_protocol::face_triangles`).
const HIDE_OPPOSITE_1: u8 = 0b010;
const HIDE_OPPOSITE_2: u8 = 0b100;

/// The vertex pool and the other pools of a `PlainMesh`.
pub(crate) fn prepare(
    view: RenderMesh,
    opts: UploadOptions,
) -> Result<(Vec<GpuVertex>, CpuPools), RenderError> {
    let mesh = view.mesh;
    check_faces(mesh)?;
    if let Some(normals) = view.normals
        && normals.len() != mesh.positions.len()
    {
        return Err(RenderError::Mesh(format!(
            "{} normals for {} positions",
            normals.len(),
            mesh.positions.len()
        )));
    }
    if opts.chunk_triangles == 0 {
        return Err(RenderError::Mesh(
            "chunk_triangles must be at least 1".into(),
        ));
    }
    let computed;
    let normals = if let Some(n) = view.normals {
        n
    } else {
        computed = area_weighted_normals(mesh);
        &computed
    };
    let vertices = (mesh.positions.iter().zip(normals))
        .map(|(&position, &n)| GpuVertex::new(position, n))
        .collect();
    let hide = match mesh.attributes.get(HIDE).map(|a| &a.values) {
        Some(AttrValues::U32(h)) if h.len() == mesh.faces.len() => h.as_slice(),
        _ => &[],
    };
    let shown = |i: usize| hide.get(i).is_none_or(|&h| h == 0);
    let at = |v: u32| mesh.positions[v as usize];
    let faces = mesh.faces.iter().enumerate();
    let mut t = triangulate(faces.map(|(i, f)| (f, shown(i), diagonal(f, at))));
    let column = |name: &str, domain: AttrDomain| {
        let a = mesh.attributes.get(name).filter(|a| a.domain == domain)?;
        let len = match domain {
            AttrDomain::Vertex => mesh.positions.len(),
            AttrDomain::Face => mesh.faces.len(),
        };
        match &a.values {
            AttrValues::U32(v) if v.len() == len => Some(v.as_slice()),
            _ => None,
        }
    };
    if let Some(ids) = column(GROUP, AttrDomain::Face) {
        t.colors = group_words(mesh.faces.iter().zip(ids.iter().copied()));
    }
    let size = opts.chunk_triangles as usize;
    let (pos, nrm) = (
        |v: u32| mesh.positions[v as usize],
        |v: u32| normals[v as usize],
    );
    let parts = gs_par::map(t.tris.len().div_ceil(size), |k| {
        let range = k * size..((k + 1) * size).min(t.tris.len());
        let (bounds, cone) = measure(&t.tris[range.clone()], pos, nrm);
        cut(&t, range, bounds, cone)
    });
    let mut cpu = assemble(&parts, |v| v, |t| t);
    if let Some(AttrValues::F32(m)) = mesh.attributes.get(MASK).map(|a| &a.values)
        && m.len() == mesh.positions.len()
    {
        let mut bytes: Vec<u8> = m.iter().map(|&x| mask_byte(x)).collect();
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        cpu.mask = Some(bytes);
    }
    cpu.colors = column(COLOR, AttrDomain::Vertex).map(<[u32]>::to_vec);
    Ok((vertices, cpu))
}

/// Every face corner names an existing vertex.
fn check_faces(mesh: &PlainMesh) -> Result<(), RenderError> {
    let n = mesh.positions.len();
    for (i, face) in mesh.faces.iter().enumerate() {
        if let Some(&bad) = corners(face).iter().find(|&&v| v as usize >= n) {
            return Err(RenderError::Mesh(format!(
                "face {i} uses vertex {bad}, but there are {n}"
            )));
        }
    }
    Ok(())
}

/// Area-weighted smooth normals, unit length. These are the normals [`crate::Renderer::upload`]
/// draws when none are given.
///
/// A vertex gets `[0, 0, 1]` when no face uses it, and also when its face normals cancel down
/// to rounding noise: a sum no longer than [`FOLD_NOISE`] times the sum, over its triangles
/// `a b c`, of the longer of `|b - a|` and `|c - a|` squared. f32 rounding alone can leave a sum
/// that long (two faces folded back to back leave one), so its direction means nothing. The f64
/// reference in the tests applies the same rule.
pub fn vertex_normals(mesh: &PlainMesh) -> Result<Vec<[f32; 3]>, RenderError> {
    check_faces(mesh)?;
    Ok(area_weighted_normals(mesh))
}

/// The near-zero rule of [`vertex_normals`]: 256 f32 rounding steps (`f32::EPSILON` each). A
/// triangle's cross product is off by a few steps of its longer edge squared, and summing `k`
/// of them adds about `k` more, so for a vertex with fewer than about 200 faces the noise stays
/// under half of this.
pub const FOLD_NOISE: f32 = 256.0 * f32::EPSILON;

fn area_weighted_normals(mesh: &PlainMesh) -> Vec<[f32; 3]> {
    let p = &mesh.positions;
    let mut sum = vec![([0.0f32; 3], 0.0f32); p.len()];
    let at = |v: u32| p[v as usize];
    let all = triangulate(mesh.faces.iter().map(|f| (f, true, diagonal(f, at))));
    for t in &all.tris {
        let [a, b, c] = t.map(|v| v as usize);
        let (e1, e2) = (sub(p[b], p[a]), sub(p[c], p[a]));
        let n = cross(e1, e2);
        let size = dot(e1, e1).max(dot(e2, e2));
        for v in [a, b, c] {
            sum[v] = (add(sum[v].0, n), sum[v].1 + size);
        }
    }
    sum.into_iter()
        .map(|(n, size)| {
            if above(dot(n, n).sqrt(), FOLD_NOISE * size) {
                normalize_or(n, [0.0, 0.0, 1.0])
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect()
}

/// A unit normal as two snorm16 octahedral coordinates (ARCHITECTURE section 6: 8 bits band
/// visibly under a MatCap, 16 do not). The shader decodes it.
pub fn encode_normal(n: [f32; 3]) -> [i16; 2] {
    // A zero vector stays (0, 0), which decodes to +Z.
    let l1 = (n[0].abs() + n[1].abs() + n[2].abs()).max(f32::MIN_POSITIVE);
    let (mut x, mut y) = (n[0] / l1, n[1] / l1);
    if n[2].is_sign_negative() {
        // Fold the lower half over the diagonals. On the fold lines both halves decode alike.
        (x, y) = ((1.0 - y.abs()).copysign(x), (1.0 - x.abs()).copysign(y));
    }
    [snorm16(x), snorm16(y)]
}

fn snorm16(v: f32) -> i16 {
    // `x.round() as i16`, without the library call baseline x86-64 makes for `round`: the
    // fraction left after truncating decides, exactly, for |x| up to 2^23 (checked against
    // `round` for every f32 from -1.5 to 1.5, NaN and the infinities).
    let x = v.clamp(-1.0, 1.0) * 32767.0;
    let t = x as i32;
    let f = x - t as f32;
    (t + i32::from(f >= 0.5) - i32::from(f <= -0.5)) as i16
}

/// x in the low half, as the Snorm16x2 vertex format and `unpack2x16snorm` read it.
fn pack(e: [i16; 2]) -> u32 {
    let ([a, b], [c, d]) = (e[0].to_le_bytes(), e[1].to_le_bytes());
    u32::from_le_bytes([a, b, c, d])
}

/// The diagonal quad `face` is drawn split along (`gs_protocol::quad_diagonal`), its corners at
/// `pos`; 0 for a triangle.
pub(crate) fn diagonal(face: &[u32; 4], pos: impl Fn(u32) -> [f32; 3]) -> usize {
    if arity(*face) == 3 {
        0
    } else {
        quad_diagonal(face.map(pos))
    }
}

/// Splits quads into two triangles, each along the diagonal given with it (as [`diagonal`]
/// gives it), and records which edge of each triangle is a diagonal, and whether its face,
/// given with a `shown` flag, is drawn.
pub(crate) fn triangulate<'f>(
    faces: impl IntoIterator<Item = (&'f [u32; 4], bool, usize)>,
) -> Triangles {
    let mut t = Triangles::default();
    for (face, shown, diagonal) in faces {
        if arity(*face) == 3 {
            t.tris.push([face[0], face[1], face[2]]);
            t.edges.push(0);
            t.shown.push(shown);
            continue;
        }
        let [first, second] = quad_triangles(diagonal);
        for (corners, edge) in [(first, HIDE_OPPOSITE_1), (second, HIDE_OPPOSITE_2)] {
            t.tris.push(corners.map(|k| face[k]));
            t.edges.push(edge);
            t.shown.push(shown);
        }
    }
    t
}

/// The order to draw a chunk's triangles in, from meshoptimizer's vertex cache optimizer.
/// It runs on chunk-local vertex numbers, so its cost does not grow with the whole mesh.
pub(crate) fn optimize_chunk(tris: &[[u32; 3]]) -> Vec<usize> {
    let mut globals: Vec<u32> = tris.iter().flatten().copied().collect();
    globals.sort_unstable();
    globals.dedup();
    let local: Vec<u32> = tris
        .iter()
        .flatten()
        .map(|v| match globals.binary_search(v) {
            Ok(i) | Err(i) => i as u32,
        })
        .collect();
    let optimized = meshopt::optimize_vertex_cache(&local, globals.len());
    triangle_order(&local, &optimized)
}

/// Which input triangle each output triangle is. meshoptimizer keeps each triangle's corner
/// order and only permutes triangles, so equal corner triples identify a triangle.
fn triangle_order(before: &[u32], after: &[u32]) -> Vec<usize> {
    // Sorting both lists by corners lines up each output triangle with its input triangle;
    // identical triangles pair up in order, which is as good as any other pairing.
    let mut order = vec![0; after.len() / 3];
    for ((_, input), (_, output)) in by_corners(before).into_iter().zip(by_corners(after)) {
        order[output] = input;
    }
    order
}

/// Each triangle of an index list with its position, sorted by its corners.
fn by_corners(list: &[u32]) -> Vec<(&[u32; 3], usize)> {
    let mut keyed: Vec<(&[u32; 3], usize)> = list
        .as_chunks::<3>()
        .0
        .iter()
        .enumerate()
        .map(|(i, t)| (t, i))
        .collect();
    keyed.sort_unstable();
    keyed
}
