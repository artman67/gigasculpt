//! Step 1 of the ladder on the operand as it comes in: its faces name points it has, its points
//! are somewhere, it is closed and manifold, and whether any of its shells holds a negative
//! volume.

use gs_protocol::{PlainMesh, corners};

use super::Unfit;

/// Every face names points the mesh has (`BadFace`), and every point is finite (`NotFinite`).
pub(super) fn faces_and_points(mesh: &PlainMesh) -> Result<(), Unfit> {
    let n = mesh.positions.len();
    let bad = |f: &[u32; 4]| corners(f).iter().any(|&v| v as usize >= n);
    if let Some(face) = mesh.faces.iter().position(bad) {
        return Err(Unfit::BadFace(face));
    }
    if mesh.positions.iter().flatten().any(|x| !x.is_finite()) {
        return Err(Unfit::NotFinite);
    }
    Ok(())
}

/// Closed and manifold: each directed edge of the triangles used once and its reverse present.
/// A triangle that names a point twice counts as an edge used twice.
pub(super) fn closed(tris: &[[u32; 3]]) -> Result<(), Unfit> {
    let mut keys: Vec<u64> = Vec::with_capacity(3 * tris.len());
    let mut doubled = 0;
    for &[a, b, c] in tris {
        if a == b || b == c || c == a {
            doubled += 1;
            continue;
        }
        keys.extend([(a, b), (b, c), (c, a)].map(|(p, q)| u64::from(p) << 32 | u64::from(q)));
    }
    gs_par::sort_keys(&mut keys);
    doubled += keys.windows(2).filter(|w| w[0] == w[1]).count();
    if doubled > 0 {
        return Err(Unfit::NotManifold { edges: doubled });
    }
    let keys = &keys;
    let open = gs_par::reduce(
        keys.len(),
        || 0,
        |n, i| n + usize::from(keys.binary_search(&keys[i].rotate_left(32)).is_err()),
        |a, b| a + b,
    );
    if open > 0 {
        return Err(Unfit::Open { edges: open });
    }
    Ok(())
}

/// Whether any shell of the mesh (`parts::by_shells`) encloses a negative volume: an inside-out
/// shell, or a cavity, which step 3 tells apart. `tris` are its triangles, `tri_face` each one's
/// face.
pub(super) fn any_shell_negative(mesh: &PlainMesh, tris: &[[u32; 3]], tri_face: &[u32]) -> bool {
    let shell = crate::parts::by_shells(mesh);
    let mut volume = vec![0.0_f64; mesh.faces.len()];
    for (t, &f) in tris.iter().zip(tri_face) {
        let [a, b, c] = t.map(|v| mesh.positions[v as usize].map(f64::from));
        volume[shell[f as usize] as usize] += a[0] * (b[1] * c[2] - b[2] * c[1])
            - a[1] * (b[0] * c[2] - b[2] * c[0])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
    }
    volume.iter().any(|&v| v < 0.0)
}
