//! Surface Nets (Gibson 1998): one vertex in every cell whose corners change sign, placed at the
//! average of the points where the cell's edges cross zero, and one quad across every
//! sign-changing edge between samples, joining the four cells around it.
//!
//! Output order follows space: bricks in Z order, cells in Z order inside each brick, so the
//! vertices are sorted by the Morton code of their cell and neighbours sit close in memory.

use super::Work;
use super::grid::{B, BRICK, BRICK_BITS, Field, SIDE, block_index, cell, corner, local};
use crate::math::{V3, morton, unmorton};

/// The closest a crossing gets to either end of its edge: 1/32 of the way along, and 31/32.
const FIRST: f32 = 0.031_25;
const LAST: f32 = 0.968_75;

/// The cube's 12 edges as pairs of corners (`x + 2y + 4z`).
const EDGES: [[usize; 2]; 12] = [
    [0, 1],
    [2, 3],
    [4, 5],
    [6, 7],
    [0, 2],
    [1, 3],
    [4, 6],
    [5, 7],
    [0, 4],
    [1, 5],
    [2, 6],
    [3, 7],
];

/// The extracted surface, in grid units.
pub(crate) struct Nets {
    /// Vertex positions.
    pub verts: Vec<V3>,
    /// The cell (its first corner sample) each vertex sits in.
    pub cells: Vec<[u32; 3]>,
    /// Quads, wound counter-clockwise seen from outside.
    pub faces: Vec<[u32; 4]>,
}

/// One brick's surface cells: a bit per cell in Z order, their vertices in that order, and
/// the inside bit of every sample of its gathered block.
#[derive(Default)]
struct Part {
    mask: [u64; 8],
    verts: Vec<V3>,
    cells: Vec<[u32; 3]>,
    inside: [u64; 12],
}

impl Part {
    /// How many surface cells come before Z-order position `l`.
    fn rank(&self, l: usize) -> u32 {
        let below: u32 = self.mask[..l / 64].iter().map(|w| w.count_ones()).sum();
        below + (self.mask[l / 64] & ((1u64 << (l % 64)) - 1)).count_ones()
    }

    /// Whether the block sample at index `i` is inside.
    fn inside(&self, i: usize) -> bool {
        self.inside[i / 64] >> (i % 64) & 1 == 1
    }
}

/// Cells of a brick in Z order, and each cell's Z-order position by its [`local`] index.
struct Order {
    cells: Vec<[u32; 3]>,
    position: Vec<u16>,
}

impl Order {
    fn new() -> Self {
        let cells: Vec<[u32; 3]> = (0..BRICK as u64).map(|l| unmorton(l, 3)).collect();
        let mut position = vec![0; BRICK];
        for (l, &c) in (0u16..).zip(&cells) {
            position[local(c)] = l;
        }
        Self { cells, position }
    }
}

/// Extracts the surface of the field.
pub(crate) fn extract(field: &Field, work: &mut Work) -> Nets {
    // A surface cell's first corner is a boundary sample or one step below one on each axis,
    // so its brick is allocated or just below an allocated brick.
    let mut keys = Vec::new();
    for b in &field.bricks {
        for k in 0..8 {
            let d = corner(k).map(|c| -c.cast_signed());
            if let Some(n) = field.neighbor(b.at, d) {
                keys.push(morton(n, BRICK_BITS));
            }
        }
    }
    gs_par::sort_keys(&mut keys);
    keys.dedup();
    let order = Order::new();
    let parts = gs_par::map(keys.len(), |m| {
        cells(field, unmorton(keys[m], BRICK_BITS), &order)
    });
    let mut first = Vec::with_capacity(parts.len());
    let mut count = 0u32;
    for part in &parts {
        first.push(count);
        count += part.verts.len() as u32;
    }
    let vertex = |cell: [u32; 3]| -> Option<u32> {
        let m = keys
            .binary_search(&morton(cell.map(|c| c / B), BRICK_BITS))
            .ok()?;
        Some(first[m] + parts[m].rank(usize::from(order.position[local(cell)])))
    };
    let faces = gs_par::map(keys.len(), |m| {
        let at = unmorton(keys[m], BRICK_BITS);
        quads(field, at, &parts[m], &order, &vertex)
    });
    work.cells_checked += (parts.len() * BRICK) as u64;
    Nets {
        verts: parts.iter().flat_map(|p| p.verts.iter().copied()).collect(),
        cells: parts.iter().flat_map(|p| p.cells.iter().copied()).collect(),
        faces: faces.concat(),
    }
}

/// The surface cells of the brick at `at` and their vertices.
fn cells(field: &Field, at: [u32; 3], order: &Order) -> Part {
    let block = field.gather(at);
    let mut part = Part::default();
    for (i, v) in block.iter().enumerate() {
        part.inside[i / 64] += u64::from(v.is_sign_negative()) << (i % 64);
    }
    for (l, &c) in order.cells.iter().enumerate() {
        let v = cell(&block, c);
        let inside = v.map(f32::is_sign_negative);
        if inside.iter().all(|&s| s == inside[0]) {
            continue;
        }
        let mut sum = [0.0f32; 3];
        let mut crossings = 0.0f32;
        for [k0, k1] in EDGES {
            if inside[k0] != inside[k1] {
                // The signs differ, so the gap is the sum of the sizes. Both are zero when the
                // edge lies in the surface; then take its middle. A crossing never sits right
                // on a sample: when the surface passes exactly through one, every cell around it
                // would put its vertex there, and their quads would have no area.
                let (a, b) = (v[k0].abs(), v[k1].abs());
                let t = if a + b > 0.0 { a / (a + b) } else { 0.5 };
                let t = t.clamp(FIRST, LAST);
                let (p0, p1) = (corner(k0), corner(k1));
                for a in 0..3 {
                    sum[a] += p0[a] as f32 + t * (p1[a] as f32 - p0[a] as f32);
                }
                crossings += 1.0;
            }
        }
        part.mask[l / 64] += 1u64 << (l % 64);
        let cell: [u32; 3] = std::array::from_fn(|a| at[a] * B + c[a]);
        part.verts
            .push(std::array::from_fn(|a| cell[a] as f32 + sum[a] / crossings));
        part.cells.push(cell);
    }
    part
}

/// The quads across the sign-changing edges that start at a sample of the brick at `at`. The
/// edge from sample `p` along axis `a` is shared by the cells at `p`, `p - b`, `p - c` and
/// `p - b - c`, where `b` and `c` are the next two axes; seen from the outside end of the
/// edge, they run counter-clockwise in the order below. Only allocated bricks hold boundary
/// samples, so only they have quads.
fn quads(
    field: &Field,
    at: [u32; 3],
    part: &Part,
    order: &Order,
    vertex: &impl Fn([u32; 3]) -> Option<u32>,
) -> Vec<[u32; 4]> {
    let mut out = Vec::new();
    if field.slot(at).is_none() {
        return out;
    }
    for &c in &order.cells {
        let i = block_index(c);
        let inside = part.inside(i);
        for (a, step) in [1, SIDE, SIDE * SIDE].into_iter().enumerate() {
            if part.inside(i + step as usize) == inside {
                continue;
            }
            let p: [u32; 3] = std::array::from_fn(|k| at[k] * B + c[k]);
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            let mut pb = p;
            pb[b] -= 1;
            let mut pc = p;
            pc[c] -= 1;
            let mut pbc = pb;
            pbc[c] -= 1;
            let ring = [pbc, pc, p, pb].map(vertex);
            let (Some(v00), Some(v10), Some(v11), Some(v01)) = (ring[0], ring[1], ring[2], ring[3])
            else {
                continue;
            };
            out.push(if inside {
                [v00, v10, v11, v01]
            } else {
                [v00, v01, v11, v10]
            });
        }
    }
    out
}
