//! Mesh storage, spatial index, brush kernels and presets, stroke pipeline, symmetry, masks,
//! undo storage, detail levels and live detail.
//!
//! No GPU, no files, no UI, no clock. Must build and test in under 10 seconds incrementally,
//! because mutation testing rebuilds it hundreds of times.
//!
//! The mesh lives in a [`MeshStore`] (layout B of `docs/decisions/0001-mesh-storage.md`): the
//! vertices are sorted along a Morton curve and cut into nodes of about 8,000, and every node
//! owns its columns as `Arc`s. Kernels run in two phases over the nodes a brush touches
//! ([`kernel`]); history is copy-on-write ([`MeshStore::begin_step`]); topology changes go
//! through the [`topo`] editor or a whole-mesh re-sort ([`MeshStore::resort`]). Detail levels
//! ([`Levels`], decision 0014) keep the top level as an ordinary store and every lower level as
//! a view of it.

pub mod alpha;
mod bits;
pub mod brush;
mod build;
pub mod camera;
mod cavity;
mod collapse;
pub mod columns;
pub mod config;
pub mod curve;
pub mod deform;
mod derived;
mod exact;
pub mod faces;
pub mod falloff;
mod flip;
pub mod gizmo;
pub mod groups;
mod history;
pub mod kernel;
mod kernels;
mod levels;
pub mod live;
pub mod mask;
pub mod material;
pub mod math;
pub mod mirror;
pub mod normals;
mod numbering;
mod par;
pub mod pick;
mod plain;
mod relax;
mod reorder;
pub mod rng;
pub mod select;
mod sink;
mod split;
mod store;
pub mod stroke;
mod subdivide;
pub mod symmetry;
pub mod topo;
pub mod transform;
pub mod tree;
pub mod view;
pub mod visibility;
mod write;

pub use bits::Bits;
pub use brush::{BrushPreset, LiveDetailSpec, LiveMode, PresetError};
pub use columns::{AttrDef, AttrId, AttrKind, AttrValue, AttrVec, ColumnSet, Domain, linear_rgb};
pub use config::{FaceId, MeshError, NodeConfig, NodeId, VertId};
pub use exact::ExactNode;
pub use history::{Allocation, StepId, StepSummary};
pub use kernel::{Kernel, Reach, Scratch, SculptSurface, StepReport};
pub use levels::project as projection;
pub use levels::{Carried, LevelInfo, Levels, Pending, Settings, StoredLevel, StoredLevels};
pub use live::{LiveDetail, LiveReport};
pub use math::{Aabb, NO, Quat, Vec3};
pub use numbering::Numbering;
pub use pick::{Hit, Nearest, RayCast};
pub use plain::Placement;
pub use sink::NodeSink;
pub use store::{MeshStore, NodeQuery, Work};
pub use stroke::{
    PaintColors, Stroke, StrokeError, StrokeParams, StrokeReport, StrokeSample, View, stroke,
};
pub use subdivide::{level_vertex_counts, subdivide_corners, subdivide_faces};
pub use view::{AttrReader, NodeView, PosRead, PosReader, TopoRead, TopoReader};
pub use visibility::{Cover, NodeState, VisibilityEdit, Visible};
pub use write::{NewValues, NodeMut, Writer};
