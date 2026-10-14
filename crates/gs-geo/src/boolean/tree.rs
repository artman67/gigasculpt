//! The group as one lazy CSG expression (decision 0057, section 5): the base, then each run of
//! one operation as one node, each operand's transform on its leaf.

use std::collections::BTreeSet;
use std::sync::Arc;

use gs_core::transform::Similarity;
use manifold_rust::csg_tree::{CsgLeafNode, CsgNode};
use manifold_rust::impl_mesh::ManifoldImpl;
use manifold_rust::linalg::{Mat3x4, Vec3};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{Error, OpType};

use super::{BooleanError, Leaf};
use crate::operation::Operation;

/// A leaf as the library places it: the original id on its triangles, unique in the group, and
/// the transform the library moves its points by.
pub(super) struct Placed {
    pub(super) id: i32,
    pub(super) transform: Mat3x4,
}

/// The group's expression evaluated, and its leaves as placed.
pub(super) fn evaluate(group: &[Leaf<'_>]) -> Result<(ManifoldImpl, Vec<Placed>), BooleanError> {
    let mut seen = BTreeSet::new();
    let mut placed = Vec::with_capacity(group.len());
    let mut nodes = Vec::with_capacity(group.len());
    for l in group {
        let p = l.operand;
        let (p_impl, id) = if seen.insert(p.id) {
            (Arc::clone(&p.imp), p.id)
        } else {
            retag(&p.imp)
        };
        let leaf = CsgLeafNode {
            p_impl,
            transform: matrix(&l.at),
        };
        // The library composes each leaf's transform with the tree's identity once; the same
        // call gives the transform exactly as it will move the points.
        let transform = leaf.apply_transform(Mat3x4::identity()).transform;
        placed.push(Placed { id, transform });
        nodes.push(CsgNode::leaf_node(leaf));
    }
    let operations: Vec<Operation> = group.iter().map(|l| l.operation).collect();
    let Some(tree) = expression(nodes, &operations) else {
        return Ok((ManifoldImpl::new(), placed));
    };
    let result = tree.evaluate();
    if result.status != Error::NoError {
        return Err(BooleanError::Engine(result.status.to_string()));
    }
    Ok((result, placed))
}

/// The left fold as one tree: the first node is the base, and each run of the same operation
/// after it one node over the result so far and the run, which the library flattens.
fn expression(nodes: Vec<CsgNode>, operations: &[Operation]) -> Option<CsgNode> {
    let mut nodes = nodes.into_iter().zip(operations);
    let (mut tree, _) = nodes.next()?;
    let mut run: Vec<CsgNode> = Vec::new();
    let mut run_op = Operation::Union;
    for (node, &operation) in nodes {
        if operation != run_op && !run.is_empty() {
            tree = join(tree, std::mem::take(&mut run), run_op);
        }
        run_op = operation;
        run.push(node);
    }
    if !run.is_empty() {
        tree = join(tree, run, run_op);
    }
    Some(tree)
}

fn join(tree: CsgNode, run: Vec<CsgNode>, operation: Operation) -> CsgNode {
    let op = match operation {
        Operation::Union => OpType::Add,
        Operation::Difference => OpType::Subtract,
        Operation::Intersect => OpType::Intersect,
    };
    CsgNode::op_n(op, std::iter::once(tree).chain(run).collect())
}

/// The similarity as the library's 3 by 4 matrix (columns).
fn matrix(at: &Similarity) -> Mat3x4 {
    let [x, y, z, w] = (at.matrix()).map(|c| Vec3::new(c[0].into(), c[1].into(), c[2].into()));
    Mat3x4::from_cols(x, y, z, w)
}

/// A copy of the mesh under a fresh original id, for an operand standing at a second place in
/// the group, so each place's triangles can be told apart in the result.
fn retag(imp: &ManifoldImpl) -> (Arc<ManifoldImpl>, i32) {
    let id = Manifold::reserve_ids(1).cast_signed();
    let mut copy = imp.clone();
    let relation = copy
        .mesh_relation
        .mesh_id_transform
        .values()
        .next()
        .cloned();
    for r in &mut copy.mesh_relation.tri_ref {
        r.mesh_id = id;
        r.original_id = id;
    }
    copy.mesh_relation.mesh_id_transform = (relation.into_iter())
        .map(|mut r| {
            r.original_id = id;
            (id, r)
        })
        .collect();
    (Arc::new(copy), id)
}
