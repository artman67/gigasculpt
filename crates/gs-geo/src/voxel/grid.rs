//! The sample grid and its sparse storage: 8 x 8 x 8 bricks, allocated only near the surface.
//!
//! A brick that is not allocated is uniform: every sample in it is inside, or every sample is
//! outside, and it reads as plus or minus [`FAR`]. The rule that keeps this honest: every
//! sample with a neighbour of the other sign (a "boundary sample") lives in an allocated brick.

use super::{CellSize, GROWTH, Kept, MAX_CELLS, RemeshError};

/// Samples along each side of a brick.
pub(crate) const B: u32 = 8;
/// Samples in a brick.
pub(crate) const BRICK: usize = 512;
/// The distance stored for samples farther than [`super::band::REACH`] from every triangle.
pub(crate) const FAR: f32 = 2.0;
/// Margin, in voxels, between the bounding box and the first sample of the grid.
pub const PAD: f64 = 3.0;
/// Bits per axis of a brick's Morton code (bricks per side stay under 1024).
pub(crate) const BRICK_BITS: u32 = 10;
/// No triangle: the sample is beyond [`super::band::REACH`].
pub(crate) const NONE: u32 = u32::MAX;

/// Samples per side of a brick's block of values, including one layer past its far faces.
pub(crate) const SIDE: u32 = B + 1;

/// Table entries that are not brick slots.
const UNKNOWN: u32 = 0xFFFF_FFFF;
const OUT: u32 = 0xFFFF_FFFE;
const IN: u32 = 0xFFFF_FFFD;

/// Where the samples are: sample `(i, j, k)` sits at `origin + voxel * (i, j, k)`.
///
/// The grid covers the bounding box plus [`PAD`] voxels on every side: `origin` is the box's
/// minimum corner minus `PAD * voxel`, and along each axis the grid holds whole bricks of 8
/// samples, enough to reach `PAD` voxels past the box's maximum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    /// World position of sample `(0, 0, 0)`.
    pub origin: [f64; 3],
    /// Distance between neighbouring samples, in scene units.
    pub voxel: f64,
    /// Bricks along each axis; samples along each axis are 8 times as many.
    pub bricks: [u32; 3],
}

impl Grid {
    /// The grid for a bounding box `lo..hi`, and the cell size to keep for the next remesh.
    pub(crate) fn new(
        lo: [f64; 3],
        hi: [f64; 3],
        cell: CellSize,
    ) -> Result<(Self, Kept), RemeshError> {
        let longest = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        let (voxel, cells, limit) = match cell {
            CellSize::Resolution(n) => (longest / f64::from(n), f64::from(n), n.max(GROWTH)),
            CellSize::Size(size) => {
                let cells = longest / f64::from(size);
                (f64::from(size), cells, (cells.ceil() as u32).max(GROWTH))
            }
            CellSize::Kept(kept) => {
                // The kept size, unless the longest side has outgrown its limit.
                let voxel = kept.voxel.max(longest / f64::from(kept.limit));
                (voxel, longest / voxel, kept.limit)
            }
        };
        if !(voxel.is_finite() && voxel >= f64::MIN_POSITIVE) {
            return Err(RemeshError::BadCellSize);
        }
        if cells > f64::from(MAX_CELLS) {
            return Err(RemeshError::TooManyCells);
        }
        let origin = lo.map(|c| c - PAD * voxel);
        let bricks = std::array::from_fn(|a| {
            let top = (hi[a] - origin[a]) / voxel + PAD;
            (top.floor() as u32 + 1).div_ceil(B)
        });
        let grid = Self {
            origin,
            voxel,
            bricks,
        };
        Ok((grid, Kept { voxel, limit }))
    }

    /// Samples along each axis.
    pub fn samples(&self) -> [u32; 3] {
        self.bricks.map(|b| b * B)
    }

    /// A world position in grid units.
    pub(crate) fn to_grid(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|a| ((f64::from(p[a]) - self.origin[a]) / self.voxel) as f32)
    }

    /// A grid position in world units.
    pub(crate) fn to_world(self, g: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|a| (self.origin[a] + f64::from(g[a]) * self.voxel) as f32)
    }
}

/// One allocated brick: signed distances (negative inside) and the nearest triangle of each
/// sample, in row-major order (x fastest).
pub(crate) struct Brick {
    /// Brick coordinates (sample coordinates divided by 8).
    pub at: [u32; 3],
    /// Signed distance in voxels, clamped to [`FAR`]. The sign bit says inside.
    pub value: Box<[f32; BRICK]>,
    /// Nearest triangle, or [`NONE`] past [`super::band::REACH`].
    pub near: Box<[u32; BRICK]>,
}

/// Index of a sample inside its brick.
pub(crate) fn local(p: [u32; 3]) -> usize {
    (p[0] % B + B * (p[1] % B) + B * B * (p[2] % B)) as usize
}

/// The sample at index `i` of the brick at `at`.
pub(crate) fn sample(at: [u32; 3], i: usize) -> [u32; 3] {
    let i = i as u32;
    [
        at[0] * B + i % B,
        at[1] * B + i / B % B,
        at[2] * B + i / (B * B),
    ]
}

/// Offset of cube corner `k`, numbered `x + 2y + 4z`.
pub(crate) fn corner(k: usize) -> [u32; 3] {
    let k = k as u32;
    [k & 1, k >> 1 & 1, k >> 2]
}

/// Index of brick-local coordinates `c` in a gathered block.
pub(crate) fn block_index(c: [u32; 3]) -> usize {
    (c[0] + SIDE * (c[1] + SIDE * c[2])) as usize
}

/// The eight corner values of the cell at brick-local `c`, from a gathered block.
pub(crate) fn cell(block: &[f32], c: [u32; 3]) -> [f32; 8] {
    std::array::from_fn(|k| {
        let d = corner(k);
        block[block_index(std::array::from_fn(|a| c[a] + d[a]))]
    })
}

/// The sparse field: a dense table over all bricks, pointing at the allocated ones.
pub(crate) struct Field {
    pub grid: Grid,
    table: Vec<u32>,
    pub bricks: Vec<Brick>,
    /// With several runs to fold (`super::fold`), each run's nearest triangle at each sample of
    /// the brick in that slot as the band keys it, run after run, until the brick is signed.
    /// Empty for one run, and for the slots of bricks the band did not measure.
    pub runs: Vec<Box<[u64]>>,
}

impl Field {
    pub(crate) fn new(grid: Grid) -> Self {
        let [x, y, z] = grid.bricks.map(|b| b as usize);
        Self {
            grid,
            table: vec![UNKNOWN; x * y * z],
            bricks: Vec::new(),
            runs: Vec::new(),
        }
    }

    fn index(&self, b: [u32; 3]) -> usize {
        let [nx, ny, _] = self.grid.bricks;
        (b[0] + nx * (b[1] + ny * b[2])) as usize
    }

    /// The brick next to `b` at offset `d`, if it is inside the grid.
    pub(crate) fn neighbor(&self, b: [u32; 3], d: [i32; 3]) -> Option<[u32; 3]> {
        let mut out = [0; 3];
        for a in 0..3 {
            let c = b[a].checked_add_signed(d[a])?;
            if c >= self.grid.bricks[a] {
                return None;
            }
            out[a] = c;
        }
        Some(out)
    }

    /// Slot of an allocated brick, or `None` for a uniform one.
    pub(crate) fn slot(&self, b: [u32; 3]) -> Option<usize> {
        let entry = self.table[self.index(b)];
        (entry < IN).then_some(entry as usize)
    }

    /// Bricks in the whole grid, allocated or not.
    pub(crate) fn capacity(&self) -> usize {
        self.table.len()
    }

    /// True if the brick is allocated or its uniform sign is known.
    pub(crate) fn known(&self, b: [u32; 3]) -> bool {
        self.table[self.index(b)] != UNKNOWN
    }

    /// Records the sign of a brick not seen before. A brick that is allocated, or whose sign is
    /// already known, keeps what it has.
    pub(crate) fn set_uniform(&mut self, b: [u32; 3], inside: bool) {
        let i = self.index(b);
        if self.table[i] == UNKNOWN {
            self.table[i] = if inside { IN } else { OUT };
        }
    }

    /// Adds an allocated brick.
    ///
    /// # Panics
    /// If the brick is already allocated: that is a bug, and carrying on would loop.
    pub(crate) fn insert(&mut self, brick: Brick) -> usize {
        let i = self.index(brick.at);
        // Read the table itself, so a broken `slot` cannot hide a second allocation.
        assert!(self.table[i] >= IN, "brick allocated twice");
        let slot = self.bricks.len();
        self.table[i] = slot as u32;
        self.bricks.push(brick);
        slot
    }

    /// The signed value of a sample: its brick's entry, or plus or minus [`FAR`] in a uniform
    /// brick.
    pub(crate) fn value(&self, p: [u32; 3]) -> f32 {
        let b = p.map(|c| c / B);
        if (0..3).any(|a| b[a] >= self.grid.bricks[a]) {
            return FAR;
        }
        match self.table[self.index(b)] {
            IN => -FAR,
            UNKNOWN | OUT => FAR,
            slot => self.bricks[slot as usize].value[local(p)],
        }
    }

    /// The values of the brick at `at` and of one more layer past its +x, +y and +z faces, so
    /// every cell whose first corner is in the brick has all eight corners. Indexed by
    /// [`block_index`].
    pub(crate) fn gather(&self, at: [u32; 3]) -> Vec<f32> {
        let mut block = vec![0.0; (SIDE * SIDE * SIDE) as usize];
        let base = at.map(|c| c * B);
        // The brick's own samples: copied, or its one uniform value.
        let uniform = [self.value(base); BRICK];
        let own = self
            .slot(at)
            .map_or(&uniform, |slot| &*self.bricks[slot].value);
        for (i, &v) in own.iter().enumerate() {
            block[block_index(sample([0; 3], i))] = v;
        }
        // The layer past each far face, from the neighbouring bricks.
        for a in 0..3 {
            for j in 0..SIDE * SIDE {
                let mut c = [0; 3];
                c[a] = B;
                c[(a + 1) % 3] = j % SIDE;
                c[(a + 2) % 3] = j / SIDE;
                block[block_index(c)] = self.value(std::array::from_fn(|k| base[k] + c[k]));
            }
        }
        block
    }

    /// The nearest triangle of a sample, or [`NONE`].
    pub(crate) fn near(&self, p: [u32; 3]) -> u32 {
        self.slot(p.map(|c| c / B))
            .map_or(NONE, |s| self.bricks[s].near[local(p)])
    }

    /// Allocates a uniform brick, every sample at its uniform value. Returns its slot.
    pub(crate) fn materialize(&mut self, b: [u32; 3]) -> usize {
        if let Some(slot) = self.slot(b) {
            return slot;
        }
        let v = self.value(sample(b, 0));
        self.insert(Brick {
            at: b,
            value: Box::new([v; BRICK]),
            near: Box::new([NONE; BRICK]),
        })
    }
}
