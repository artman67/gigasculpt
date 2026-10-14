//! The small parts a live run makes beside its result (decision 0057, section 9, as amended for
//! LB7): each Subtract's and Intersect's cut walls, the result's faces it made, and Show
//! Issues' and Show Coplanar's marks, each operand's faces `boolean.check`'s own code finds,
//! in the operand's own space. Each is a copy of its faces with every column carried
//! (gs-geo `parts::piece`), drawn tinted over the faces it copies.

use gs_geo::parts::piece;
use gs_protocol::PlainMesh;

use super::Member;
use super::draw::Tint;
use crate::boolean::check::{between, coplanar_threshold, part_check};
use crate::boolean::ladder::Operand;
use crate::document::{Operation, Part, VertexOrder};

/// An operand's marked faces.
pub(super) struct Mark {
    /// The operand's place in its group.
    pub(super) member: usize,
    /// `Issue` or `Coplanar`.
    pub(super) tint: Tint,
    pub(super) part: Part,
}

/// The walls of each Subtract and Intersect of `members` in `mesh`, their result: its faces
/// whose owner (`owners`, one per face, from the cut's face sources) is that operand. None
/// without owners (the voxel fold of a group with no columns).
pub(super) fn walls(
    members: &[Member],
    mesh: &PlainMesh,
    owners: &[u32],
) -> Vec<(Operation, Part)> {
    let mut faces = vec![Vec::new(); members.len()];
    for (f, &owner) in owners.iter().enumerate() {
        if let Some(list) = faces.get_mut(owner as usize) {
            list.push(f as u32);
        }
    }
    let cuts = (members.iter().zip(faces)).filter(|(m, _)| m.operation != Operation::Union);
    cuts.filter_map(|(m, f)| Some((m.operation, small(mesh, &f, &m.name)?)))
        .collect()
}

/// The marks of `operands`, a group's in order, for the checks `checks` asks for: Show Issues
/// (each operand's open and non-manifold edges, inside-out shells and self-crossings) and Show
/// Coplanar (the faces each two operands have lying on each other, at `boolean.check`'s
/// default threshold).
pub(super) fn marks(operands: &[Operand], checks: [bool; 2]) -> Vec<Mark> {
    let mut faces: Vec<[Vec<u32>; 2]> = vec![[Vec::new(), Vec::new()]; operands.len()];
    if checks[0] {
        for (o, found) in operands.iter().zip(&mut faces) {
            let c = part_check(&o.name, &o.world);
            let issues = [
                c.open_edges,
                c.non_manifold_edges,
                c.inside_out_shells,
                c.self_crossings,
            ];
            found[0] = issues.into_iter().flat_map(|i| i.faces).collect();
        }
    }
    if checks[1] {
        let threshold = f64::from(coplanar_threshold());
        for i in 0..operands.len() {
            for j in i + 1..operands.len() {
                let (a, b) = (&operands[i], &operands[j]);
                let pair = [a.name.clone(), b.name.clone()];
                if let Some(c) = between(pair, [&a.world, &b.world], threshold) {
                    let [on_a, on_b] = c.faces;
                    faces[i][1].extend(on_a);
                    faces[j][1].extend(on_b);
                }
            }
        }
    }
    let mut out = Vec::new();
    for (member, (o, kinds)) in operands.iter().zip(faces).enumerate() {
        for (tint, mut f) in [Tint::Issue, Tint::Coplanar].into_iter().zip(kinds) {
            f.sort_unstable();
            f.dedup();
            if let Some(part) = small(&o.local, &f, &o.name) {
                out.push(Mark { member, tint, part });
            }
        }
    }
    out
}

/// The faces `faces` of `mesh` as a part of their own, or `None` for no faces.
fn small(mesh: &PlainMesh, faces: &[u32], name: &str) -> Option<Part> {
    if faces.is_empty() {
        return None;
    }
    Part::new(piece(mesh, faces), VertexOrder::Spatial, name).ok()
}
