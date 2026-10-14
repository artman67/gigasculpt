//! Test-only geometry: shape builders, and the slow, independent references gs-testkit does
//! not have yet (nearest distance from a point to a mesh, the exact winding number, connected
//! pieces), all in `f64`. They share no code with gs-geo. Hausdorff distance, volume and the
//! manifold check come from gs-testkit.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh, face_triangles};

pub use gs_testkit::measure::{P, cross, dot, norm, sub};

pub fn wide(p: [f32; 3]) -> P {
    p.map(f64::from)
}

// ---------------------------------------------------------------- shapes

/// The surface of the box `lo..hi`, each side an `n` by `n` grid of quads, wound
/// counter-clockwise seen from outside.
pub fn cuboid(lo: P, hi: P, n: u32) -> PlainMesh {
    let mut index: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut positions = Vec::new();
    for i in 0..=n {
        for j in 0..=n {
            for k in 0..=n {
                let g = [i, j, k];
                if g.iter().any(|&c| c == 0 || c == n) {
                    index.insert(g, positions.len() as u32);
                    let t = g.map(|c| f64::from(c) / f64::from(n));
                    positions.push(std::array::from_fn(|a| {
                        (lo[a] + t[a] * (hi[a] - lo[a])) as f32
                    }));
                }
            }
        }
    }
    let mut faces = Vec::new();
    for (axis, side, a, b) in (0..3)
        .flat_map(|axis| [0, n].map(|side| (axis, side)))
        .flat_map(|(axis, side)| (0..n * n).map(move |ab| (axis, side, ab / n, ab % n)))
    {
        // Corner (du, dv) of the cell: `side` on `axis`, then the next two axes in turn.
        let ring = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(du, dv)| {
            let g: [u32; 3] = std::array::from_fn(|k| match (k + 3 - axis) % 3 {
                0 => side,
                1 => a + du,
                _ => b + dv,
            });
            index[&g]
        });
        let outward = side == n;
        faces.push(if outward {
            ring
        } else {
            [ring[0], ring[3], ring[2], ring[1]]
        });
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

/// A quad sphere: a subdivided cube pushed out onto the sphere.
pub fn sphere(center: P, radius: f64, n: u32) -> PlainMesh {
    let mut mesh = cuboid([-1.0; 3], [1.0; 3], n);
    for p in &mut mesh.positions {
        let q = wide(*p);
        let s = radius / norm(q);
        *p = std::array::from_fn(|a| (center[a] + q[a] * s) as f32);
    }
    mesh
}

/// A torus around the y axis: ring radius `big`, tube radius `small`.
pub fn torus(big: f64, small: f64, nu: u32, nv: u32) -> PlainMesh {
    let tau = std::f64::consts::TAU;
    let mut positions = Vec::new();
    for i in 0..nu {
        let u = tau * f64::from(i) / f64::from(nu);
        for j in 0..nv {
            let v = tau * f64::from(j) / f64::from(nv);
            let r = big + small * libm::cos(v);
            positions.push([
                (r * libm::cos(u)) as f32,
                (small * libm::sin(v)) as f32,
                (r * libm::sin(u)) as f32,
            ]);
        }
    }
    let at = |i: u32, j: u32| (i % nu) * nv + j % nv;
    let mut faces = Vec::new();
    for i in 0..nu {
        for j in 0..nv {
            faces.push([at(i, j), at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

/// Both meshes in one, attributes dropped.
pub fn merge(a: &PlainMesh, b: &PlainMesh) -> PlainMesh {
    let shift = a.positions.len() as u32;
    let mut out = a.clone();
    out.attributes.clear();
    out.positions.extend(&b.positions);
    out.faces.extend(
        b.faces
            .iter()
            .map(|f| f.map(|v| if v == NO_VERTEX { v } else { v + shift })),
    );
    out
}

/// The mesh with every face's winding reversed.
pub fn flipped(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    for f in &mut out.faces {
        if f[3] == NO_VERTEX {
            f.swap(1, 2);
        } else {
            f.swap(1, 3);
        }
    }
    out
}

/// The mesh with its first face added again: that face's edges are then each used twice the
/// same way, which no cap can fix.
pub fn doubled(mesh: &PlainMesh) -> PlainMesh {
    let mut out = mesh.clone();
    out.faces.push(mesh.faces[0]);
    out
}

/// The mesh without the faces `drop` says to drop, and without the vertices only they used.
pub fn without(mesh: &PlainMesh, drop: impl Fn(&[P]) -> bool) -> PlainMesh {
    let faces: Vec<[u32; 4]> = mesh
        .faces
        .iter()
        .filter(|f| !drop(&corners(mesh, f)))
        .copied()
        .collect();
    let mut used = vec![u32::MAX; mesh.positions.len()];
    let mut positions = Vec::new();
    for &v in faces.iter().flatten().filter(|&&v| v != NO_VERTEX) {
        if used[v as usize] == u32::MAX {
            used[v as usize] = positions.len() as u32;
            positions.push(mesh.positions[v as usize]);
        }
    }
    let faces = faces
        .iter()
        .map(|f| f.map(|v| if v == NO_VERTEX { v } else { used[v as usize] }))
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: BTreeMap::new(),
    }
}

pub fn column(domain: AttrDomain, values: AttrValues) -> Attribute {
    Attribute { domain, values }
}

// ---------------------------------------------------------------- measurements

/// The corner positions of a face.
pub fn corners(mesh: &PlainMesh, f: &[u32; 4]) -> Vec<P> {
    f.iter()
        .filter(|&&v| v != NO_VERTEX)
        .map(|&v| wide(mesh.positions[v as usize]))
        .collect()
}

/// Enclosed volume (gs-testkit's measure).
pub fn volume(mesh: &PlainMesh) -> f64 {
    gs_testkit::measure::stats(mesh).volume
}

/// Total face area (gs-testkit's measure).
pub fn surface_area(mesh: &PlainMesh) -> f64 {
    gs_testkit::measure::stats(mesh).area
}

/// Every face as triangles, quads split as they are drawn ([`face_triangles`]), in `f64`.
pub fn triangles(mesh: &PlainMesh) -> Vec<[P; 3]> {
    let at = |v: u32| mesh.positions[v as usize];
    (mesh.faces.iter())
        .flat_map(|&f| {
            let tris = face_triangles(f, |k| at(f[k]));
            tris.iter().map(move |t| t.map(|k| wide(at(f[k]))))
        })
        .collect()
}

/// The representative of `x`'s set in a union-find forest.
fn root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// Number of connected pieces (faces sharing a vertex are connected).
pub fn components(mesh: &PlainMesh) -> usize {
    let mut parent: Vec<usize> = (0..mesh.positions.len()).collect();
    for f in &mesh.faces {
        for &v in f.iter().filter(|&&v| v != NO_VERTEX).skip(1) {
            let (a, b) = (
                root(&mut parent, f[0] as usize),
                root(&mut parent, v as usize),
            );
            parent[a] = b;
        }
    }
    let mut roots: Vec<usize> = (0..parent.len()).map(|v| root(&mut parent, v)).collect();
    roots.sort_unstable();
    roots.dedup();
    roots.len()
}

/// Distance from `p` to segment `ab`.
pub fn segment_distance(p: P, a: P, b: P) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(f64::MIN_POSITIVE)).clamp(0.0, 1.0);
    norm(sub(
        p,
        [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]],
    ))
}

/// Distance from `p` to triangle `t`: to its plane if `p` projects inside it (checked with
/// sub-triangle areas), otherwise to the nearest edge.
pub fn triangle_distance(p: P, t: &[P; 3]) -> f64 {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let area = norm(n);
    let edges = || {
        segment_distance(p, t[0], t[1])
            .min(segment_distance(p, t[1], t[2]))
            .min(segment_distance(p, t[2], t[0]))
    };
    if area <= 1e-300 {
        return edges();
    }
    let unit = n.map(|c| c / area);
    let height = dot(sub(p, t[0]), unit);
    let q = std::array::from_fn(|a| p[a] - height * unit[a]);
    let inside = (0..3).all(|k| dot(cross(sub(t[(k + 1) % 3], t[k]), sub(q, t[k])), n) >= 0.0);
    if inside { height.abs() } else { edges() }
}

/// Fast-enough nearest distance to a fixed triangle set: triangles bucketed on a coarse grid.
pub struct Nearest {
    tris: Vec<[P; 3]>,
    lo: P,
    cell: f64,
    dims: [usize; 3],
    buckets: Vec<Vec<u32>>,
}

impl Nearest {
    pub fn new(tris: Vec<[P; 3]>, cell: f64) -> Self {
        let mut lo = [f64::MAX; 3];
        let mut hi = [f64::MIN; 3];
        for p in tris.iter().flatten() {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let dims = std::array::from_fn(|a| ((hi[a] - lo[a]) / cell) as usize + 1);
        let mut buckets = vec![Vec::new(); dims[0] * dims[1] * dims[2]];
        let mut out = Self {
            tris: Vec::new(),
            lo,
            cell,
            dims,
            buckets: Vec::new(),
        };
        for (i, t) in tris.iter().enumerate() {
            let cmin = out.cell_of(std::array::from_fn(|a| t[0][a].min(t[1][a]).min(t[2][a])));
            let cmax = out.cell_of(std::array::from_fn(|a| t[0][a].max(t[1][a]).max(t[2][a])));
            for x in cmin[0]..=cmax[0] {
                for y in cmin[1]..=cmax[1] {
                    for z in cmin[2]..=cmax[2] {
                        buckets[x + dims[0] * (y + dims[1] * z)].push(i as u32);
                    }
                }
            }
        }
        out.tris = tris;
        out.buckets = buckets;
        out
    }

    fn cell_of(&self, p: P) -> [usize; 3] {
        std::array::from_fn(|a| {
            (((p[a] - self.lo[a]) / self.cell).max(0.0) as usize).min(self.dims[a] - 1)
        })
    }

    /// Distance from `p` to the nearest triangle (infinite for a point that is not finite).
    ///
    /// Searches outward ring by ring of buckets around `p`'s bucket. A triangle that lies
    /// only in buckets more than `r` rings out is at least `r` bucket widths away, so the search
    /// stops once the best distance found is within that. A point more than 4 bucket widths
    /// outside the triangles' bounding box gets its distance to that box instead: a lower
    /// bound, which is all a failing check needs, and quick for points thrown far off.
    pub fn distance(&self, p: P) -> f64 {
        if !p.iter().all(|c| c.is_finite()) {
            return f64::INFINITY;
        }
        let outside: P = std::array::from_fn(|a| {
            let hi = self.lo[a] + self.cell * self.dims[a] as f64;
            (self.lo[a] - p[a]).max(p[a] - hi).max(0.0)
        });
        let away = norm(outside);
        if away > 4.0 * self.cell {
            return away;
        }
        let c = self.cell_of(p);
        let mut best = f64::INFINITY;
        for r in 0..=self.dims.iter().copied().max().unwrap_or(0) {
            for x in c[0].saturating_sub(r)..=(c[0] + r).min(self.dims[0] - 1) {
                for y in c[1].saturating_sub(r)..=(c[1] + r).min(self.dims[1] - 1) {
                    for z in c[2].saturating_sub(r)..=(c[2] + r).min(self.dims[2] - 1) {
                        let ring = [x, y, z].iter().zip(&c).map(|(a, b)| a.abs_diff(*b)).max();
                        if ring != Some(r) {
                            continue;
                        }
                        for &t in &self.buckets[x + self.dims[0] * (y + self.dims[1] * z)] {
                            best = best.min(triangle_distance(p, &self.tris[t as usize]));
                        }
                    }
                }
            }
            if best <= r as f64 * self.cell {
                break;
            }
        }
        best
    }
}

/// Points spread over a mesh's faces: corners, edge midpoints and face centres.
pub fn surface_points(mesh: &PlainMesh) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for f in &mesh.faces {
        let c = corners(mesh, f);
        let n = c.len() as f64;
        out.extend(&c);
        out.push(std::array::from_fn(|a| {
            c.iter().map(|p| p[a]).sum::<f64>() / n
        }));
        for k in 0..c.len() {
            let (p, q) = (c[k], c[(k + 1) % c.len()]);
            out.push(std::array::from_fn(|a| f64::midpoint(p[a], q[a])));
        }
    }
    out
}

/// gs-testkit's `measure::hausdorff_from` (same samples: every vertex, then every face centre),
/// sped up with a bucket grid of cell size `cell`, for meshes too big for its brute force in a
/// debug build. The oracle self-tests hold it to gs-testkit's answer.
pub fn fast_hausdorff_from(from: &PlainMesh, to: &PlainMesh, cell: f64) -> f64 {
    let near = Nearest::new(triangles(to), cell);
    let centres = from.faces.iter().map(|f| {
        let c = corners(from, f);
        let n = c.len() as f64;
        std::array::from_fn(|a| c.iter().map(|p| p[a]).sum::<f64>() / n)
    });
    from.positions
        .iter()
        .map(|&p| wide(p))
        .chain(centres)
        .map(|p| near.distance(p))
        .fold(0.0, f64::max)
}

/// The two-sided version of [`fast_hausdorff_from`].
pub fn fast_hausdorff(a: &PlainMesh, b: &PlainMesh, cell: f64) -> f64 {
    fast_hausdorff_from(a, b, cell).max(fast_hausdorff_from(b, a, cell))
}

/// Exact winding number by Girard's theorem: each triangle's solid angle is its spherical
/// excess on the unit sphere around `q`, signed by which side `q` is on.
pub fn winding(tris: &[[P; 3]], q: P) -> f64 {
    let mut sum = 0.0;
    for t in tris {
        let u = t.map(|p| {
            let d = sub(p, q);
            let l = norm(d);
            d.map(|c| c / l)
        });
        let angle = |a: P, b: P, c: P| {
            // Angle at `a` of the spherical triangle abc.
            let n1 = cross(a, b);
            let n2 = cross(a, c);
            libm::atan2(norm(cross(n1, n2)), dot(n1, n2))
        };
        let excess = angle(u[0], u[1], u[2]) + angle(u[1], u[2], u[0]) + angle(u[2], u[0], u[1])
            - std::f64::consts::PI;
        let side = dot(u[0], cross(u[1], u[2]));
        if side.abs() >= 1e-12 {
            sum += excess.copysign(side);
        }
    }
    sum / (4.0 * std::f64::consts::PI)
}
