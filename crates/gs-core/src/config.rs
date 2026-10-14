//! Node size and id numbering (ARCHITECTURE 3.1, decision 0001).
//!
//! A vertex id is `node << shift | local`, so the owning node is one shift away and every node
//! owns a contiguous id range. A face id is `node << (shift + 2) | local`: a node has room for
//! four times as many faces as vertex slots, which a closed mesh (about two triangles per vertex)
//! cannot outgrow before its vertex slots run out.

use std::fmt;

pub type VertId = u32;
pub type FaceId = u32;
pub type NodeId = u32;

/// Why a mesh could not be stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// A node size outside the supported range.
    Config(String),
    /// Something wrong with the input mesh: bad face, index or attribute.
    Input(String),
    /// More nodes than the id space holds.
    TooLarge(usize),
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeshError::Config(s) => write!(f, "bad node size: {s}"),
            MeshError::Input(s) => write!(f, "bad mesh: {s}"),
            MeshError::TooLarge(n) => write!(f, "mesh needs {n} nodes, more than ids can hold"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Node size: `1 << shift` vertex slots per node, filled with `fill` vertices when the mesh is
/// cut, so nodes have room to grow before they split.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeConfig {
    shift: u32,
    fill: u32,
}

impl Default for NodeConfig {
    /// 8,192 vertices per node with room for 16,384 (decision 0001, node size sweep).
    fn default() -> Self {
        NodeConfig {
            shift: 14,
            fill: 8192,
        }
    }
}

impl NodeConfig {
    /// `shift` from 2 to 20; `fill` from 1 to `1 << shift`.
    pub fn new(shift: u32, fill: u32) -> Result<NodeConfig, MeshError> {
        if !(2..=20).contains(&shift) || fill == 0 || fill > 1 << shift {
            return Err(MeshError::Config(format!("shift {shift}, fill {fill}")));
        }
        Ok(NodeConfig { shift, fill })
    }
    pub fn shift(self) -> u32 {
        self.shift
    }
    pub fn fill(self) -> u32 {
        self.fill
    }
    /// Vertex slots per node.
    pub fn capacity(self) -> u32 {
        1 << self.shift
    }
    pub fn fshift(self) -> u32 {
        self.shift + 2
    }
    /// Face slots per node.
    pub fn face_capacity(self) -> u32 {
        1 << self.fshift()
    }
    /// Most nodes a mesh may have.
    pub fn max_nodes(self) -> usize {
        1 << (32 - self.fshift())
    }
    #[inline]
    pub fn vert(self, node: NodeId, local: u32) -> VertId {
        (node << self.shift) + local
    }
    #[inline]
    pub fn node_of(self, v: VertId) -> NodeId {
        v >> self.shift
    }
    #[inline]
    pub fn local_of(self, v: VertId) -> u32 {
        v & (self.capacity() - 1)
    }
    #[inline]
    pub fn face(self, node: NodeId, local: u32) -> FaceId {
        (node << self.fshift()) + local
    }
    #[inline]
    pub fn face_node(self, f: FaceId) -> NodeId {
        f >> self.fshift()
    }
    #[inline]
    pub fn face_local(self, f: FaceId) -> u32 {
        f & (self.face_capacity() - 1)
    }
}
