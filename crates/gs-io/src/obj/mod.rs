//! Wavefront OBJ, following the public format description.
//!
//! Read: vertices (`v x y z`, with an optional weight, or a color as three more numbers from 0
//! to 1), UVs (`vt`), faces of any size with 1-based or negative indices (polygons split, see
//! [`crate::mesh`]), objects (`o`, one [`crate::mesh::MeshObject`] each), face groups from `g`
//! (or from `usemtl` when asked), and `#MRGB` lines (8 hex digits per vertex: mask,
//! red, green, blue; 255 is not masked). Normals, materials, smoothing groups and other lines
//! are counted in the notes and dropped; normals are recomputed. A vertex belongs to the object
//! whose block defines it; a face that uses a vertex of another block gets a copy. Faces must
//! use vertices defined before them.
//!
//! Group ids: when every group name of an object is `Group<n>`, the ids are those numbers (the
//! form sculpting apps and our writer use, so ids survive a round trip); otherwise groups are
//! numbered in the order they first appear.
//!
//! Write: one `o` per object (none for a single unnamed one), positions as the shortest
//! decimal text that reads back to the same `f32`, vertex colors as three numbers after the
//! position, mask and color as `#MRGB` lines, UVs once each, `g Group<id>` where the face group
//! changes, and faces.

mod read;
mod write;

pub use read::read;
pub use write::write;
