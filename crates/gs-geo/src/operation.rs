//! The boolean operation an operand takes in its group (decision 0057 section 5), in Blender's
//! words. The exact engine (`boolean`) and the voxel fold (`voxel`) both read it.

/// How an operand joins the result so far, left to right in its group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Inside the result or the operand ("or" per voxel sample).
    #[default]
    Union,
    /// Inside the result and not the operand ("and not").
    Difference,
    /// Inside the result and the operand ("and").
    Intersect,
}
