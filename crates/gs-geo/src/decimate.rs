//! Decimate (decision 0034): fewer, larger triangles made of the mesh's own points. A pure
//! function, mesh in and mesh out, around meshoptimizer's edge collapse (`meshopt`, pinned).
//!
//! 1. Locks: points with a mask of 0.5 or more, points of hidden faces, and, when groups are
//!    kept, points where faces of two groups meet.
//! 2. The split key: shown faces are cut into triangles, and each triangle corner becomes a
//!    split point keyed by (point, the face's values in every face column but the corner
//!    columns, the corner's values in the corner columns). Corners with the same key share one;
//!    a point with several is a seam, which meshoptimizer keeps in place or moves along.
//! 3. meshoptimizer collapses edges until the shown triangles reach the target, weighing the
//!    corner columns (the UVs) and the paint (the caller's color per point) in its error. It
//!    never moves a point.
//! 4. Each new triangle takes its face columns from its first corner and its corner columns
//!    corner by corner; point columns stay with their points; hidden faces come back as they
//!    were, after the triangles.
//! 5. [`repair`]: duplicate faces go, crowded edges split (`weld::repair`), unused points go.
//!
//! [`unpaired`] checks a part is symmetric and [`mirror`] makes the other half, for a
//! symmetric decimate.

use gs_protocol::{
    AttrDomain, AttrValues, Attribute, GROUP, HIDE, MASK, NO_VERTEX, PlainMesh, face_triangles,
};
use meshopt::SimplifyOptions;

use crate::weld::{self, corners};

/// The mask from which a point is locked.
const MASKED: f32 = 0.5;
/// How much the corner columns count in the error against distance, which meshoptimizer
/// measures in a unit box: research 22's weight for UVs.
const CORNER_WEIGHT: f32 = 50.0;
/// How much each channel of the paint counts in the error against distance (decision 0034,
/// section 3): the least weight at which a sphere painted in bands and decimated to 10% reads
/// no point's color wrong, and a red line one point wide on a flat sheet keeps its borders
/// within one and a half spacings.
pub const PAINT_WEIGHT: f32 = 1.0;
/// The most attributes meshoptimizer weighs; further corner columns are only in the split key.
const MOST_WEIGHED: usize = 32;

/// What to decimate to and what to keep.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Shown triangles to keep. Hidden faces are set aside and come back on top.
    pub faces: usize,
    /// Keep the `group` column and lock the points on its borders. Off: the column is dropped.
    pub keep_groups: bool,
    /// Lock the points on open edges.
    pub keep_borders: bool,
    /// Ask for more even triangles (meshoptimizer's Regularize).
    pub uniform: bool,
    /// Face columns that hold one value per corner, corner `k` in column `k` (the UVs).
    pub corners: Vec<[String; 4]>,
    /// Three values for each point of the mesh, weighed in the error beside the corner columns
    /// (the color in linear light), so painted detail is not collapsed away. Empty, or all one
    /// value, weighs nothing.
    pub paint: Vec<[f32; 3]>,
    /// How much each of them counts against distance, as [`PAINT_WEIGHT`].
    pub paint_weight: f32,
}

/// What repair changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Repairs {
    /// Faces removed as duplicates.
    pub duplicate_faces: u64,
    /// Edges on more than two faces, split.
    pub crowded_edges: u64,
    /// Points added by the splits.
    pub points_added: u64,
}

/// A decimated mesh and what it took.
#[derive(Clone, Debug)]
pub struct Decimated {
    /// Triangles, then the hidden faces as they were.
    pub mesh: PlainMesh,
    /// The shown triangles meshoptimizer left, before repair.
    pub shown: usize,
    /// meshoptimizer's error estimate, in the mesh's units.
    pub error: f32,
    /// The points locked.
    pub locked: usize,
    /// What repair changed.
    pub repairs: Repairs,
}

/// `mesh` decimated as `settings` say.
pub fn decimate(mesh: &PlainMesh, settings: &Settings) -> Decimated {
    let hidden: Vec<bool> = match face_column(mesh, HIDE) {
        Some(AttrValues::U32(h)) => h.iter().map(|&x| x != 0).collect(),
        _ => vec![false; mesh.faces.len()],
    };
    let lock = locks(mesh, &hidden, settings.keep_groups);
    let corner_columns: Vec<[Option<&AttrValues>; 4]> = (settings.corners.iter())
        .map(|g| g.each_ref().map(|name| face_column(mesh, name)))
        .filter(|g| g.iter().any(Option::is_some))
        .collect();
    let in_corners = |name: &str| settings.corners.iter().any(|g| g.iter().any(|c| c == name));
    let face_columns: Vec<&AttrValues> = (mesh.attributes.iter())
        .filter(|(name, a)| a.domain == AttrDomain::Face && !in_corners(name))
        .filter(|(name, _)| settings.keep_groups || *name != GROUP)
        .map(|(_, a)| &a.values)
        .collect();
    let split = Split::new(mesh, &hidden, &face_columns, &corner_columns);
    let (tris, error) = simplify(mesh, &split, &lock, settings, &corner_columns);
    let simplified = assemble(mesh, &split, &tris, &hidden, settings);
    let (mesh, repairs, _) = repair(&simplified);
    Decimated {
        mesh,
        shown: tris.len() / 3,
        error,
        locked: lock.iter().filter(|&&l| l).count(),
        repairs,
    }
}

/// The face column `name`, if there is one.
fn face_column<'a>(mesh: &'a PlainMesh, name: &str) -> Option<&'a AttrValues> {
    (mesh.attributes.get(name))
        .filter(|a| a.domain == AttrDomain::Face)
        .map(|a| &a.values)
}

/// Which points are locked: masked ones, those of hidden faces, and group borders.
fn locks(mesh: &PlainMesh, hidden: &[bool], keep_groups: bool) -> Vec<bool> {
    let mut lock = match mesh.attributes.get(MASK) {
        Some(Attribute {
            domain: AttrDomain::Vertex,
            values: AttrValues::F32(m),
        }) => m.iter().map(|&x| x >= MASKED).collect(),
        _ => vec![false; mesh.positions.len()],
    };
    for (face, _) in mesh.faces.iter().zip(hidden).filter(|f| *f.1) {
        for &v in corners(face) {
            lock[v as usize] = true;
        }
    }
    if let (true, Some(AttrValues::U32(groups))) = (keep_groups, face_column(mesh, GROUP)) {
        let mut first: Vec<Option<u32>> = vec![None; mesh.positions.len()];
        for (face, &g) in mesh.faces.iter().zip(groups) {
            for &v in corners(face) {
                let seen = first[v as usize].get_or_insert(g);
                lock[v as usize] |= *seen != g;
            }
        }
    }
    lock
}

/// The bits of value `i` of a column (`-0` read as `0`, so it keys as `0` does).
fn bits(values: &AttrValues, i: usize) -> u32 {
    match values {
        AttrValues::F32(v) => (v[i] + 0.0).to_bits(),
        AttrValues::U32(v) => v[i],
    }
}

/// A column of the same type as `like`, `n` values long, value `j` from `bits(j)`.
fn column_like(like: &AttrValues, n: usize, bits: impl Fn(usize) -> u32 + Sync) -> AttrValues {
    match like {
        AttrValues::F32(_) => AttrValues::F32(gs_par::map(n, |j| f32::from_bits(bits(j)))),
        AttrValues::U32(_) => AttrValues::U32(gs_par::map(n, bits)),
    }
}

/// Gives each row a class: rows with the same (class, `value`) share one, in sorted order.
fn refine(class: &mut [u32], value: impl Fn(usize) -> u32 + Sync) {
    let mut keyed = gs_par::map(class.len(), |i| {
        ((u64::from(class[i]) << 32) | u64::from(value(i)), i as u32)
    });
    gs_par::sort(&mut keyed);
    for (k, same) in keyed.chunk_by(|a, b| a.0 == b.0).enumerate() {
        for &(_, i) in same {
            class[i as usize] = k as u32;
        }
    }
}

/// The shown faces as triangles over split points.
struct Split {
    /// Each triangle corner's split point, three per triangle.
    index: Vec<u32>,
    /// Each split point's point.
    point: Vec<u32>,
    /// Where each split point was first found, as (face, corner slot): its face and corner
    /// values are read there.
    at: Vec<(u32, u8)>,
}

impl Split {
    fn new(
        mesh: &PlainMesh,
        hidden: &[bool],
        face_columns: &[&AttrValues],
        corner_columns: &[[Option<&AttrValues>; 4]],
    ) -> Split {
        let nf = mesh.faces.len();
        let mut face_class = vec![0; nf];
        for col in face_columns {
            refine(&mut face_class, |f| bits(col, f));
        }
        let mut corner_class = vec![0; if corner_columns.is_empty() { 0 } else { nf * 4 }];
        for group in corner_columns {
            refine(&mut corner_class, |i| {
                group[i % 4].map_or(0, |c| bits(c, i / 4))
            });
        }
        // Triangle corners as (face, slot): a quad is cut as it is drawn (decision 0040,
        // section 2).
        let fans: Vec<(u32, u8)> = (0..nf)
            .filter(|&f| !hidden[f])
            .flat_map(|f| {
                let face = mesh.faces[f];
                let tris = face_triangles(face, |k| mesh.positions[face[k] as usize]);
                tris.iter().flatten().map(move |&s| (f as u32, s as u8))
            })
            .collect();
        let mut keyed = gs_par::map(fans.len(), |c| {
            let (f, s) = (fans[c].0 as usize, usize::from(fans[c].1));
            let corner = corner_class.get(f * 4 + s).copied().unwrap_or(0);
            let key = (
                mesh.faces[f][s],
                (u64::from(face_class[f]) << 32) | u64::from(corner),
            );
            (key, c as u32)
        });
        gs_par::sort(&mut keyed);
        let mut split = Split {
            index: vec![0; fans.len()],
            point: Vec::new(),
            at: Vec::new(),
        };
        for same in keyed.chunk_by(|a, b| a.0 == b.0) {
            let id = split.point.len() as u32;
            split.point.push(same[0].0.0);
            split.at.push(fans[same[0].1 as usize]);
            for &(_, c) in same {
                split.index[c as usize] = id;
            }
        }
        split
    }
}

/// meshoptimizer's triangles over the split points, and its error estimate.
fn simplify(
    mesh: &PlainMesh,
    split: &Split,
    lock: &[bool],
    settings: &Settings,
    corner_columns: &[[Option<&AttrValues>; 4]],
) -> (Vec<u32>, f32) {
    if split.index.is_empty() {
        return (Vec::new(), 0.0);
    }
    let positions: Vec<[f32; 3]> = (split.point.iter())
        .map(|&p| mesh.positions[p as usize])
        .collect();
    // Paint all one color would still move meshoptimizer's choices: an unpainted part is left
    // as it decimates without paint.
    let bits = |c: [f32; 3]| c.map(f32::to_bits);
    let paint = settings.paint_weight > 0.0
        && (settings.paint.windows(2)).any(|w| bits(w[0]) != bits(w[1]));
    let channels = if paint { 3 } else { 0 };
    let weighed = &corner_columns[..corner_columns.len().min(MOST_WEIGHED - channels)];
    let width = channels + weighed.len();
    let mut attributes = Vec::with_capacity(width * split.point.len());
    for (&p, &(f, s)) in split.point.iter().zip(&split.at) {
        if paint {
            attributes.extend(settings.paint[p as usize]);
        }
        attributes.extend(weighed.iter().map(|g| match g[usize::from(s)] {
            Some(AttrValues::F32(v)) => v[f as usize],
            _ => 0.0,
        }));
    }
    let mut weights = vec![settings.paint_weight; channels];
    weights.resize(width, CORNER_WEIGHT);
    let locks: Vec<bool> = split.point.iter().map(|&p| lock[p as usize]).collect();
    let mut options = SimplifyOptions::ErrorAbsolute;
    if settings.uniform {
        options |= SimplifyOptions::Regularize;
    }
    if settings.keep_borders {
        options |= SimplifyOptions::LockBorder;
    }
    let target = 3 * settings.faces.min(split.index.len() / 3);
    let mut error = 0.0;
    let tris = meshopt::simplify_with_attributes_and_locks_decoder(
        &split.index,
        &positions,
        &attributes,
        &weights,
        4 * width,
        &locks,
        target,
        f32::MAX,
        options,
        Some(&mut error),
    );
    (tris, error)
}

/// The simplified mesh over the input's points: the triangles with their columns, then the
/// hidden faces as they were.
fn assemble(
    mesh: &PlainMesh,
    split: &Split,
    tris: &[u32],
    hidden: &[bool],
    settings: &Settings,
) -> PlainMesh {
    let held: Vec<u32> = (0..mesh.faces.len() as u32)
        .filter(|&f| hidden[f as usize])
        .collect();
    let nt = tris.len() / 3;
    let n = nt + held.len();
    let corner = |j: usize, k: usize| split.at[tris[3 * j + k] as usize];
    let mut faces: Vec<[u32; 4]> = (0..nt)
        .map(|j| {
            let p = |k| split.point[tris[3 * j + k] as usize];
            [p(0), p(1), p(2), NO_VERTEX]
        })
        .collect();
    faces.extend(held.iter().map(|&f| mesh.faces[f as usize]));
    let mut attributes = std::collections::BTreeMap::new();
    for (name, a) in &mesh.attributes {
        if name == GROUP && !settings.keep_groups {
            continue;
        }
        let values = match a.domain {
            AttrDomain::Vertex => a.values.clone(),
            AttrDomain::Face => match corner_slot(settings, name) {
                // Corner k of a new triangle reads its split point's own corner and column.
                Some((group, k)) => column_like(&a.values, n, |j| match (j < nt, k < 3) {
                    (true, true) => {
                        let (f, s) = corner(j, k);
                        face_column(mesh, &group[usize::from(s)]).map_or(0, |c| bits(c, f as usize))
                    }
                    (true, false) => 0,
                    (false, _) => bits(&a.values, held[j - nt] as usize),
                }),
                None => column_like(&a.values, n, |j| {
                    let f = if j < nt { corner(j, 0).0 } else { held[j - nt] };
                    bits(&a.values, f as usize)
                }),
            },
        };
        attributes.insert(
            name.clone(),
            Attribute {
                domain: a.domain,
                values,
            },
        );
    }
    PlainMesh {
        positions: mesh.positions.clone(),
        faces,
        attributes,
    }
}

/// The corner group a face column belongs to, and its slot in it.
fn corner_slot<'a>(settings: &'a Settings, name: &str) -> Option<(&'a [String; 4], usize)> {
    (settings.corners.iter()).find_map(|g| Some((g, g.iter().position(|c| c == name)?)))
}

/// `mesh` repaired (`weld::repair`), with its columns following the faces and points, and the
/// points no face uses gone. Also says, for each point of the result, which point of `mesh` it
/// is (a copy names the point it copies).
pub fn repair(mesh: &PlainMesh) -> (PlainMesh, Repairs, Vec<u32>) {
    let n = mesh.positions.len();
    let fixed = weld::repair(&mesh.faces, n);
    let total = n + fixed.copies.len();
    let mut used = vec![false; total];
    for face in &fixed.faces {
        for &v in corners(face) {
            used[v as usize] = true;
        }
    }
    let mut renumber = vec![NO_VERTEX; total];
    let mut points = Vec::new();
    for v in (0..total).filter(|&v| used[v]) {
        renumber[v] = points.len() as u32;
        points.push(if v < n { v as u32 } else { fixed.copies[v - n] });
    }
    let faces = (fixed.faces.iter())
        .map(|f| {
            f.map(|v| {
                if v == NO_VERTEX {
                    v
                } else {
                    renumber[v as usize]
                }
            })
        })
        .collect();
    let attributes = (mesh.attributes.iter())
        .map(|(name, a)| {
            let pick: &[u32] = match a.domain {
                AttrDomain::Vertex => &points,
                AttrDomain::Face => &fixed.from,
            };
            let values = column_like(&a.values, pick.len(), |j| bits(&a.values, pick[j] as usize));
            (
                name.clone(),
                Attribute {
                    domain: a.domain,
                    values,
                },
            )
        })
        .collect();
    let repairs = Repairs {
        duplicate_faces: fixed.duplicate_faces,
        crowded_edges: fixed.crowded_edges,
        points_added: fixed.copies.len() as u64,
    };
    let positions = points.iter().map(|&p| mesh.positions[p as usize]).collect();
    let out = PlainMesh {
        positions,
        faces,
        attributes,
    };
    (out, repairs, points)
}

/// The first point with no point within `tolerance` of its mirror image across the plane
/// through the origin square to `axis`, if there is one.
pub fn unpaired(positions: &[[f32; 3]], axis: usize, tolerance: f32) -> Option<usize> {
    let n = positions.len();
    let mirrored = positions.iter().map(|&p| {
        let mut q = p;
        q[axis] = -q[axis];
        q
    });
    let both: Vec<[f32; 3]> = positions.iter().copied().chain(mirrored).collect();
    let weld = weld::near(&both, tolerance);
    (0..n).find(|&i| weld[n + i] as usize >= n)
}

/// `mesh` and its mirror image across the plane through the origin square to `axis`, joined at
/// the points on the plane, which both halves share: each other point gets a mirrored copy and
/// each face a mirrored copy with its winding reversed, its corner columns (`corner_columns`,
/// as in [`Settings`]) reversed with it. Also gives each point's partner.
pub fn mirror(
    mesh: &PlainMesh,
    axis: usize,
    corner_columns: &[[String; 4]],
) -> (PlainMesh, Vec<u32>) {
    let n = mesh.positions.len();
    let mut partner: Vec<u32> = (0..n as u32).collect();
    let mut source: Vec<u32> = (0..n as u32).collect();
    for v in 0..n {
        if mesh.positions[v][axis] != 0.0 {
            partner[v] = source.len() as u32;
            partner.push(v as u32);
            source.push(v as u32);
        }
    }
    let positions = (source.iter().enumerate())
        .map(|(i, &v)| {
            let mut p = mesh.positions[v as usize];
            if i >= n {
                p[axis] = -p[axis];
            }
            p
        })
        .collect();
    let nf = mesh.faces.len();
    let flipped = mesh.faces.iter().map(|face| {
        let c = corners(face);
        let mut g = [NO_VERTEX; 4];
        for (i, &v) in c.iter().enumerate() {
            g[c.len() - 1 - i] = partner[v as usize];
        }
        g
    });
    let faces = mesh.faces.iter().copied().chain(flipped).collect();
    // Slot k of a mirrored face is slot (corners - 1 - k) of its original.
    let slot = |f: usize, k: usize| {
        let c = corners(&mesh.faces[f]).len();
        if k < c { c - 1 - k } else { k }
    };
    let attributes = (mesh.attributes.iter())
        .map(|(name, a)| {
            let values = match a.domain {
                AttrDomain::Vertex => column_like(&a.values, source.len(), |i| {
                    bits(&a.values, source[i] as usize)
                }),
                AttrDomain::Face => {
                    let group = corner_columns
                        .iter()
                        .find_map(|g| Some((g, g.iter().position(|c| c == name)?)));
                    column_like(&a.values, 2 * nf, |j| match (j < nf, group) {
                        (true, _) => bits(&a.values, j),
                        (false, None) => bits(&a.values, j - nf),
                        (false, Some((g, k))) => {
                            let f = j - nf;
                            let from = face_column(mesh, &g[slot(f, k)]);
                            from.map_or(0, |c| bits(c, f))
                        }
                    })
                }
            };
            (
                name.clone(),
                Attribute {
                    domain: a.domain,
                    values,
                },
            )
        })
        .collect();
    (
        PlainMesh {
            positions,
            faces,
            attributes,
        },
        partner,
    )
}
