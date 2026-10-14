//! Auto UV (decision 0061): cut a part's faces into charts, flatten each chart and pack them into
//! the unit square. One file or folder per stage, meeting on the plain data below:
//!
//! | Stage | What it does |
//! |---|---|
//! | [`cut`](fn@cut) | Grows charts on whole faces, cuts each to a disc, checks and splits it (UV2). |
//! | [`seams`] | The one seam finder: an edge whose faces hold different UVs at its ends. |
//! | [`Flattener`] | Lays each chart flat by LSCM, twins as mirror images (UV3). |
//! | [`solve`] | The flattener's Jacobi-preconditioned conjugate gradient (UV3). |
//! | [`metrics`] | Stretch, flips, cover and density of any finished layout. |
//! | [`pack`](fn@pack) | Scales the flat charts to one density and packs them into the square (UV4). |
//!
//! Points are `[f32; 3]`, faces `[u32; 4]` with [`NO_VERTEX`](gs_protocol::NO_VERTEX) in a
//! triangle's fourth corner, a chart is a number per face, and a UV is a `[f32; 2]` per corner.

mod cut;
mod flatten;
pub mod metrics;
mod pack;
pub mod solve;

use std::fmt;

pub use cut::{cut, cut_with, seams};
pub use flatten::{Flat, Flattener};
pub use pack::{Crowded, Mirror, Pack, Twins, pack};

/// A face's four UVs, one per corner, as the 8 face columns `uv0.u` to `uv3.v` hold them (a
/// triangle's fourth is unused).
pub type Corners = [[f32; 2]; 4];

/// A part's surface: its points and faces.
#[derive(Clone, Copy, Debug)]
pub struct Surface<'a> {
    /// Each point's position.
    pub points: &'a [[f32; 3]],
    /// Each face's points, a triangle's fourth `NO_VERTEX`.
    pub faces: &'a [[u32; 4]],
}

/// What steers the cutter, each part left out by default (decision 0061 sections 5 to 7).
#[derive(Clone, Copy, Debug, Default)]
pub struct Steer<'a> {
    /// A group per face: every edge between two groups is a border.
    pub groups: Option<&'a [u32]>,
    /// A layout whose seams are kept: every edge that is a seam in it stays one, and only the
    /// cuts each kept island needs to become discs are added.
    pub keep: Option<&'a [Corners]>,
    /// `uv_hint` per point, from -1 to 1: a face whose corners' mean is -0.7 or below is
    /// protected, and above 0 it attracts seams.
    pub hint: Option<&'a [f32]>,
    /// `uv_density` per point, from -1 to 1: a chart's UV area per surface area is scaled by 4
    /// to its faces' mean.
    pub density: Option<&'a [f32]>,
    /// The mirror the charts and seams follow.
    pub symmetry: Option<Symmetry>,
}

/// A mirror plane through the origin, square to `axis` (0, 1 or 2 for x, y or z). Points pair
/// across it within `tolerance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Symmetry {
    /// The axis the plane is square to.
    pub axis: usize,
    /// How far a point may be from its partner's mirror image.
    pub tolerance: f32,
}

/// The cutter's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Charts {
    /// Each face's chart, charts numbered in the order of their lowest faces.
    pub chart: Vec<u32>,
    /// How many charts there are.
    pub count: u32,
    /// Each face's seams: bit k is set when the edge from its corner k to corner k + 1 is one.
    pub seams: Vec<u8>,
    /// Seam edges that were not seams in the kept layout, each counted once (every seam edge
    /// when nothing is kept).
    pub added_seams: usize,
}

/// One chart as a mesh of its own, the input a flattener takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chart {
    /// Its faces, lowest first.
    pub faces: Vec<u32>,
    /// Its points, one per wedge (a point's corners joined through edges inside the chart that
    /// are not seams), each the part point it sits at.
    pub points: Vec<u32>,
    /// Each face's corners as indices into `points`, a triangle's fourth `NO_VERTEX`.
    pub corners: Vec<[u32; 4]>,
}

/// Why the cutter gave no charts.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// A cut to a disc would have to cross a protected area of `faces` faces, one of whose
    /// points is at `point`.
    Protected {
        /// The protected area's faces.
        faces: usize,
        /// A point on it.
        point: [f32; 3],
    },
    /// `count` points have no partner across the mirror (or sit on a face with no mirror face);
    /// one of them is at `point`.
    Unpaired {
        /// The points without a partner.
        count: usize,
        /// One of them.
        point: [f32; 3],
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Protected { faces, point } => write!(
                f,
                "a cut to a disc must cross a protected area of {faces} faces, near {point:?}; \
                 erase protect along a strip across it"
            ),
            Refusal::Unpaired { count, point } => write!(
                f,
                "{count} points have no mirror partner, one near {point:?}; turn symmetry off, \
                 or run mesh.symmetrize on a part without levels"
            ),
        }
    }
}
