//! Voxel Remesh: rebuild a mesh as an evenly spaced surface of
//! quads. A pure function: mesh in, new mesh out.
//!
//! The steps, each in its own module:
//!
//! 1. `input`: split faces into triangles, in grid units.
//! 2. `cap`: check whether the mesh (each solid, for [`remesh_parts`]) is closed; if it only
//!    has holes, cap each one.
//! 3. `band`: the exact distance to the nearest triangle (caps included) for every sample
//!    within about one voxel of the surface, stored sparsely in 8 x 8 x 8 bricks.
//! 4. `rows` or `gwn`, then `sign`: which samples are inside. Closed or capped input uses the
//!    exact winding number along grid rows; anything else uses the generalized winding number.
//!    Each solid is read on its own, and a sample is inside when it is inside any. For Live
//!    Boolean ([`remesh_fold`]), `fold` instead combines the operands' signed distances by
//!    their operations: union, difference or intersect, left to right. `prepare` runs steps 1
//!    to 4 and finds the solids.
//! 5. `repair`: remove the few sample patterns that would make the surface non-manifold.
//! 6. `nets`: Surface Nets turns sign changes into quads, in spatial order.
//! 7. `finish`: find each new vertex's source point on the old surface; optionally move it
//!    there (project) and smooth those moves (blur).
//! 8. `attrs`: where each new vertex and face came from, for the engine to carry columns by.
//!    With hole closing 0 ([`remesh_holes`]), `cap` then removes the faces that came from a cap.
//!
//! Live Boolean's repair 5 ([`repair_operand`]) is the same remesh of one operand, holes always
//! closed, turned right way out first if it faces inward.
//!
//! Everything runs in parallel through `gs-par`, so the result is the same at any thread count.

mod attrs;
mod band;
mod cap;
mod finish;
mod fold;
mod grid;
mod gwn;
mod input;
mod nets;
mod prepare;
mod repair;
mod rows;
mod sign;

use std::fmt;

use gs_protocol::PlainMesh;

pub use attrs::Sources;
pub use band::{PIECE, REACH};
pub use cap::CloseHoles;
pub use grid::{Grid, PAD};
#[doc(hidden)]
pub use sign::settled_inside;

use self::grid::{BRICK, sample};
use self::gwn::Gwn;
use self::prepare::{Prepared, Solid, prepare};
use crate::operation::Operation;

/// Most cells allowed along the longest side of the bounding box.
pub const MAX_CELLS: u32 = 2048;
/// Cells the longest side may grow to at a kept cell size before the grid is refit to it
/// (catalog `geometry.voxel-remesh-resolution`), unless the size was set finer than this.
pub const GROWTH: u32 = 1024;

/// How fine the new surface is.
#[derive(Clone, Copy, Debug)]
pub enum CellSize {
    /// Cells along the longest side of the bounding box.
    Resolution(u32),
    /// Cell size in the mesh's own units (for a part the engine remeshes in place, the part's
    /// units, not the scene's).
    Size(f32),
    /// The cell size an earlier remesh kept ([`Remeshed::kept`]).
    Kept(Kept),
}

/// A cell size kept from one remesh to the next, so a part sculpted bigger gets more faces,
/// not bigger ones. Setting the size with `Resolution(n)` or `Size(s)` keeps that size, with a
/// limit of [`GROWTH`] cells along the longest side, or of the cells it was set at if more.
/// A remesh with `Kept` uses the kept size while the longest side fits in `limit` cells, and
/// otherwise refits the grid to exactly `limit` cells (the size grows); it then keeps the size
/// it used, with the same limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kept {
    /// Cell size in the units of the mesh it was kept from (for a part the engine remeshes in
    /// place, the part's own units: the engine scales it when the part's points move into the
    /// scene).
    pub voxel: f64,
    /// The most cells the longest side may span at this size.
    pub limit: u32,
}

/// Settings of one remesh (catalog items `geometry.voxel-remesh*`).
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// How fine the new surface is.
    pub cell: CellSize,
    /// Move each new vertex onto the old surface, keeping detail finer than a cell.
    pub project: bool,
    /// 0 to 100: how much to smooth the projection's moves. Only used with `project`.
    pub blur: f32,
}

/// Where the inside/outside decision came from. With several solids ([`remesh_parts`]), the
/// last in this list that any of them took.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SignMethod {
    /// Exact winding number along grid rows: the input was closed (every edge used as often in
    /// one direction as in the other, vertices at the same position counting as one).
    Winding,
    /// Exact winding number along grid rows, after capping every hole: the input had holes in
    /// an otherwise clean surface (no edge used twice in the same direction or by three faces).
    CappedWinding,
    /// Generalized winding number: some edge was used by three or more faces, or by two that
    /// run along it in the same direction.
    GeneralizedWinding,
}

/// Work counters. They show how much the remesh did, so a test can tell the sparse path from
/// a dense one, or a culled loop from one that checks everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Bricks of 512 samples allocated.
    pub bricks: u64,
    /// Sample-to-triangle distances computed: one per sample in the reach box of each piece,
    /// the samples within [`REACH`] voxels of the piece's bounding box on every axis (in grid
    /// units, `(position - origin) / voxel` rounded to `f32`). A triangle (caps over holes
    /// included) whose longest edge is `L` voxels is cut into `n * n` pieces,
    /// `n = max(1, ceil(L / PIECE))`; flat triangles, whose edge cross product has a squared
    /// length of at most 1e-12 square voxels, have none.
    pub distance_evaluations: u64,
    /// Brick and piece pairs binned: for each piece above, every brick holding a sample of its
    /// reach box.
    pub binned: u64,
    /// Row positions tested for crossings (closed or capped input): for each triangle (caps
    /// included) whose shadow on the (y, z) plane has area, every whole (y, z) in the shadow's
    /// bounding box, with corners snapped to 1/65536 voxel.
    pub crossing_tests: u64,
    /// Holes capped: the simple loops their rims make.
    pub holes: u64,
    /// Samples of allocated bricks whose sign was asked for: 512 per brick signed.
    pub sign_queries: u64,
    /// Uniform bricks whose sign was read from one sample: each unallocated brick next to an
    /// allocated one, once.
    pub uniform_queries: u64,
    /// Terms summed for the generalized winding numbers of allocated samples.
    pub winding_terms: u64,
    /// Bricks allocated because an inside/outside change ran into them from an allocated brick
    /// (a hole's film), or because the inside reached the grid's edge in them. Always 0 unless
    /// the generalized winding number signed some of the input.
    pub film_bricks: u64,
    /// Samples moved inside by the repair.
    pub flips: u64,
    /// Cells checked for a sign change: 512 per brick that could hold a surface cell.
    pub cells_checked: u64,
    /// Steps taken walking from each vertex's first guess to its closest input triangle.
    pub walk_steps: u64,
}

/// A remeshed surface and how it was made.
#[derive(Clone, Debug)]
pub struct Remeshed {
    /// The new mesh: quads, vertices in Morton order of their grid cell.
    pub mesh: PlainMesh,
    /// The grid it was sampled on.
    pub grid: Grid,
    /// The cell size to keep for the next remesh of this part.
    pub kept: Kept,
    /// Where the inside/outside decision came from.
    pub sign: SignMethod,
    /// Work counters.
    pub work: Work,
    /// Where each new vertex and face came from, to carry the input's columns by (the new mesh
    /// has none). Empty when the input has no columns.
    pub sources: Sources,
}

/// Why a remesh could not run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemeshError {
    /// The mesh has no faces with three distinct corners.
    NoFaces,
    /// This face uses a vertex index past the end of the vertex list.
    BadFace(usize),
    /// This attribute has the wrong number of values.
    BadAttribute(String),
    /// A vertex coordinate is NaN or infinite.
    NotFinite,
    /// The cell size is zero, negative or not a number, or the mesh is a single point.
    BadCellSize,
    /// The cells along the longest side would exceed [`MAX_CELLS`].
    TooManyCells,
    /// The parts' face counts given to [`remesh_parts`] (or the operands' given to
    /// [`remesh_fold`]) do not add up to the mesh's faces.
    BadParts,
}

impl fmt::Display for RemeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFaces => write!(f, "the mesh has no faces"),
            Self::BadFace(face) => write!(f, "face {face} uses a vertex that does not exist"),
            Self::BadAttribute(name) => write!(f, "attribute '{name}' has the wrong length"),
            Self::NotFinite => write!(f, "a vertex coordinate is not a finite number"),
            Self::BadCellSize => write!(
                f,
                "the cell size must be a positive number, and the mesh more than a point"
            ),
            Self::TooManyCells => write!(
                f,
                "more than {MAX_CELLS} cells along the longest side; use a lower resolution"
            ),
            Self::BadParts => write!(f, "the parts' face counts do not add up to the faces"),
        }
    }
}

impl std::error::Error for RemeshError {}

/// Rebuilds `mesh` as a closed, evenly spaced quad surface.
///
/// The result is closed and manifold, and inside the grid. Overlapping parts merge into one
/// shell; a cavity whose faces point inward stays hollow; a hole gets a patch across its rim
/// (flat for a flat rim); an inside region of messy input that reaches the grid's edge is cut
/// off one sample in (see `sign`). Detail smaller than a cell is lost unless `settings.project`
/// is on. The mesh is one part: to remesh several as solids, whichever way each faces, use
/// [`remesh_parts`].
pub fn remesh(mesh: &PlainMesh, settings: &Settings) -> Result<Remeshed, RemeshError> {
    remesh_parts(mesh, &[mesh.faces.len()], settings)
}

/// [`remesh`], closing holes as `close` says. With [`CloseHoles::Off`] the inside is read with
/// each hole capped, as always, then the surface the caps made is removed: each new face whose
/// nearest old triangle is a cap (the rule [`Sources::faces`] follows), and the vertices only
/// those faces used. An open mesh is left open about where its rim was. A closed mesh, or one
/// read by the generalized winding number, which has no caps, remeshes the same either way.
pub fn remesh_holes(
    mesh: &PlainMesh,
    settings: &Settings,
    close: CloseHoles,
) -> Result<Remeshed, RemeshError> {
    remesh_parts_holes(mesh, &[mesh.faces.len()], settings, close)
}

/// [`remesh`] of a mesh made of parts one after another: `parts` counts each part's faces, in
/// order (as `crate::parts::merge` lays them out), and must add up to the mesh's faces.
///
/// The result is the union of the parts as solids: a sample is inside when it is inside any
/// part, each part's inside read from its own winding. So which way a part faces does not
/// matter (an inside-out part over another stays filled where one winding over both would
/// cancel to a hollow), and a cavity wound inward in a part stays hollow. Parts that share a
/// point (bit for bit) count as one part: the pieces of a split part have holes where they were
/// cut, and are closed only together. Each part is capped, or left to the generalized winding
/// number, on its own; the reported [`SignMethod`] is the most general any part took.
pub fn remesh_parts(
    mesh: &PlainMesh,
    parts: &[usize],
    settings: &Settings,
) -> Result<Remeshed, RemeshError> {
    remesh_parts_holes(mesh, parts, settings, CloseHoles::Large)
}

/// [`remesh_parts`], closing holes as `close` says (see [`remesh_holes`]). The fold of the
/// parts all as unions ([`remesh_fold`]).
pub fn remesh_parts_holes(
    mesh: &PlainMesh,
    parts: &[usize],
    settings: &Settings,
    close: CloseHoles,
) -> Result<Remeshed, RemeshError> {
    let operands: Vec<(usize, Operation)> = parts.iter().map(|&n| (n, Operation::Union)).collect();
    remesh_fold(mesh, &operands, settings, close)
}

/// Live Boolean's fold (decision 0057 section 3): [`remesh_parts_holes`] of a mesh made of
/// operands one after another, each combined with the result so far by its operation, left to
/// right. `operands` counts each operand's faces, in order, with its operation; the counts must
/// add up to the mesh's faces ([`RemeshError::BadParts`] otherwise). The first operand is where
/// the fold starts, so its operation is not used.
///
/// Each operand is a solid, read as [`remesh_parts`] reads a part (capped or left to the
/// generalized winding number on its own, whichever way it faces). Operands next to each other
/// with the same operation are one run, read as one solid as `remesh_parts` reads its parts:
/// a run of intersects is inside where all its operands are. The runs then fold as level sets:
/// union, difference and intersect are the minimum, the maximum with the run negated, and the
/// maximum of the signed distances (see `fold`). Points shared bit for bit join operands into
/// one solid only within a run, so a cutter touching the body is not welded into it.
/// [`Remeshed::sources`] points name the operand whose surface each new point came from, an
/// earlier operand on a tie. An intersect with an operand that has no faces leaves nothing: the
/// result has no faces. All unions remesh exactly as [`remesh_parts_holes`].
pub fn remesh_fold(
    mesh: &PlainMesh,
    operands: &[(usize, Operation)],
    settings: &Settings,
    close: CloseHoles,
) -> Result<Remeshed, RemeshError> {
    Ok(build(mesh, operands, settings, close, false)?.0)
}

/// A repaired operand ([`repair_operand`]).
#[derive(Clone, Debug)]
pub struct Repaired {
    /// The operand remeshed: closed and manifold.
    pub remeshed: Remeshed,
    /// The operand faced inward (its volume, holes capped, was negative) and was read turned
    /// right way out.
    pub turned: bool,
}

/// Live Boolean's repair 5 (decision 0057 section 3): one operand remeshed into a closed,
/// manifold solid that faces out, ready to fold. Holes are always closed ([`CloseHoles::Large`]).
/// An operand whose volume (holes capped) is negative faces inward and is turned right way out
/// before its inside is read; the result says so. A closed or capped operand reads the same
/// either way, by its row winding; messy input is read by the generalized winding number of
/// the turned triangles, and dangling faces (a fin) are kept, adding no volume. Errors as
/// [`remesh`].
pub fn repair_operand(mesh: &PlainMesh, settings: &Settings) -> Result<Repaired, RemeshError> {
    let operand = [(mesh.faces.len(), Operation::Union)];
    let (remeshed, turned) = build(mesh, &operand, settings, CloseHoles::Large, true)?;
    Ok(Repaired { remeshed, turned })
}

/// The remesh behind [`remesh_fold`] and [`repair_operand`]; also says whether the input was
/// turned right way out (only ever with `turn`).
fn build(
    mesh: &PlainMesh,
    operands: &[(usize, Operation)],
    settings: &Settings,
    close: CloseHoles,
    turn: bool,
) -> Result<(Remeshed, bool), RemeshError> {
    let Prepared {
        soup,
        caps,
        mut field,
        kept,
        sign,
        method,
        mut work,
        turned,
    } = prepare(mesh, operands, settings.cell, turn)?;
    repair::repair(&mut field, &sign, &mut work);
    let mut nets = nets::extract(&field, &mut work);
    let columns = !mesh.attributes.is_empty();
    let uncap = close == CloseHoles::Off && caps > 0;
    // Where each new vertex came from, needed to project, to carry attributes or to find the
    // surface the caps made.
    let (sources, steps) = if settings.project || columns || uncap {
        finish::sources(&field, &soup, &nets)
    } else {
        (Vec::new(), 0)
    };
    work.walk_steps = steps;
    let mut placed = finish::place(&nets, &soup, &sources, settings.project, settings.blur);
    let mut carry = if columns || uncap {
        attrs::find(&soup, &sources, &nets.faces, &placed)
    } else {
        Sources::default()
    };
    if uncap {
        cap::uncap(&mut carry, &mut nets.faces, &mut placed);
        if !columns {
            carry = Sources::default();
        }
    }
    let grid = field.grid;
    work.bricks = field.bricks.len() as u64;
    let positions = gs_par::map(placed.len(), |v| grid.to_world(placed[v]));
    let remeshed = Remeshed {
        mesh: PlainMesh {
            positions,
            faces: nets.faces,
            ..PlainMesh::default()
        },
        grid,
        kept,
        sign: method,
        work,
        sources: carry,
    };
    Ok((remeshed, turned))
}

/// One sample of the signed distance field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Grid coordinates.
    pub at: [u32; 3],
    /// Signed distance to the surface in voxels, negative inside, clamped to 2.
    pub value: f32,
    /// The nearest of the triangles whose reach box holds the sample, or `None` if none does.
    /// Triangles are numbered as the input's faces split into triangles, in order (quads as
    /// they are drawn, `gs_protocol::face_triangles`), then the caps in [`Distances::caps`].
    pub near: Option<u32>,
}

/// The signed distance field a remesh meshes (before its repair step), for inspection: every
/// sample of every allocated brick.
#[derive(Clone, Debug)]
pub struct Distances {
    /// The grid.
    pub grid: Grid,
    /// Where the signs came from.
    pub sign: SignMethod,
    /// The triangles that cap holes, as input vertex indices.
    pub caps: Vec<[u32; 3]>,
    /// The samples, brick by brick.
    pub samples: Vec<Sample>,
    /// Work counters.
    pub work: Work,
}

/// The signed distance field of `mesh` on the grid a remesh with `cell` would use.
pub fn signed_distance(mesh: &PlainMesh, cell: CellSize) -> Result<Distances, RemeshError> {
    let Prepared {
        soup,
        caps,
        field,
        method,
        mut work,
        ..
    } = prepare(mesh, &[(mesh.faces.len(), Operation::Union)], cell, false)?;
    work.bricks = field.bricks.len() as u64;
    let caps = soup.tris[soup.tris.len() - caps..].to_vec();
    let samples = field
        .bricks
        .iter()
        .flat_map(|b| {
            (0..BRICK).map(|i| Sample {
                at: sample(b.at, i),
                value: b.value[i],
                near: (b.near[i] != grid::NONE).then_some(b.near[i]),
            })
        })
        .collect();
    Ok(Distances {
        grid: field.grid,
        sign: method,
        caps,
        samples,
        work,
    })
}

/// The generalized winding number of `mesh` at each point, and how many terms each sum took.
/// About 1 inside a closed mesh, 0 outside, and in between near holes.
pub fn winding_numbers(
    mesh: &PlainMesh,
    points: &[[f32; 3]],
) -> Result<Vec<(f64, u64)>, RemeshError> {
    let (tris, _) = input::triangles(mesh)?;
    let world = |v: u32| mesh.positions[v as usize].map(f64::from);
    let gwn = Gwn::new(tris.iter().map(|t| t.map(world)).collect());
    Ok(gs_par::map(points.len(), |i| {
        gwn.number(points[i].map(f64::from))
    }))
}
