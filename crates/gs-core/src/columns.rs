//! What a node stores, as columns shared with history (ARCHITECTURE 3.1 and 3.2, decision 0001).
//!
//! Every column of every node is its own `Arc`. The only way to change one is `Arc::make_mut`,
//! which copies the node's column first when an open history step (or an older one) still holds
//! it. So undo capture cannot be forgotten: history keeps the old `Arc`s and swaps them back.
//!
//! Attribute columns (mask, face group, color, hidden faces, ...) are generic and allocated
//! lazily, per node: a node that was never painted holds `None`, which reads as the column's
//! empty value: zero, but white for a color (decision 0039, section 2).

use std::sync::Arc;

use gs_protocol::{COLOR, GROUP, HIDE, MASK};

use crate::bits::Bits;
use crate::math::Vec3;

/// A set of columns, as bits: position, normal, topology, then one bit per attribute.
/// Kernels and write guards declare the columns they write; that one declaration drives undo
/// capture checks, dirty tracking and the effect report.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ColumnSet(u64);

impl ColumnSet {
    pub const EMPTY: ColumnSet = ColumnSet(0);
    pub const POSITION: ColumnSet = ColumnSet(1);
    pub const NORMAL: ColumnSet = ColumnSet(2);
    /// Faces, vertex rings and slot bookkeeping, which always change together.
    pub const TOPOLOGY: ColumnSet = ColumnSet(4);

    /// The set holding one attribute column.
    pub fn attr(id: AttrId) -> ColumnSet {
        ColumnSet(8 << id.0)
    }
    pub fn contains(self, o: ColumnSet) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for ColumnSet {
    type Output = ColumnSet;
    fn bitor(mut self, o: ColumnSet) -> ColumnSet {
        self |= o;
        self
    }
}

impl std::ops::BitOrAssign for ColumnSet {
    fn bitor_assign(&mut self, o: ColumnSet) {
        self.0 |= o.0;
    }
}

/// Most attribute columns a mesh can hold (61 bits of a [`ColumnSet`] are left for them).
pub const MAX_ATTRS: usize = 61;

/// Where Auto UV is told to put seams, from protect (-1) to attract (1) (decision 0061).
pub const UV_HINT: &str = "uv_hint";
/// How dense Auto UV's texels are told to be, from divide by 4 (-1) to times 4 (1).
pub const UV_DENSITY: &str = "uv_density";

/// Steps per unit of a ranged column's grid.
const GRID: f32 = 16_777_216.0;

/// The values a ranged scalar column keeps (decision 0061, amended 2026-10-13, step UV7): from
/// `lo` to `hi` on a grid of 2^-24, exact everywhere from -1 to 1, so `1 - m` and `-m` are too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Range {
    pub lo: f32,
    pub hi: f32,
    /// What not a number stores as.
    nan: f32,
}

impl Range {
    /// The mask: 0 to 1, not a number fully masked, the way the influence stage reads it.
    pub const MASK: Range = Range {
        lo: 0.0,
        hi: 1.0,
        nan: 1.0,
    };
    /// A UV hint: -1 to 1, not a number no hint.
    pub const HINT: Range = Range {
        lo: -1.0,
        hi: 1.0,
        nan: 0.0,
    };
    /// `x` clamped to the range on its grid. Adding 0 turns -0 into 0, so a cleared value is all
    /// zero bits.
    pub fn settle(self, x: f32) -> f32 {
        if x.is_nan() {
            return self.nan;
        }
        (x.clamp(self.lo, self.hi) * GRID).round() / GRID + 0.0
    }
}

/// What an attribute column has one value for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Vertex,
    Face,
}

/// How an attribute is stored, and how values blend when topology changes (an edge split, a
/// subdivision step).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    /// `f32`, blended as the mean (mask and UV hints, then settled in their [`Range`]).
    Scalar,
    /// `u32` id, copied from the first source (face group).
    Label,
    /// `u32` packed RGBA, 8 bits per channel, blended per channel (color).
    Rgba8,
    /// One bit per element, packed (hidden faces, decision 0015). Reads as 0 or 1; any
    /// non-zero write sets it. Copied from the first source, like a label.
    Flag,
}

/// A registered attribute column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrDef {
    pub name: String,
    pub domain: Domain,
    pub kind: AttrKind,
}

impl AttrDef {
    pub fn new(name: &str, domain: Domain, kind: AttrKind) -> AttrDef {
        AttrDef {
            name: name.to_owned(),
            domain,
            kind,
        }
    }
    /// The sculpting mask: 0 free, 1 fully masked.
    pub fn mask() -> AttrDef {
        AttrDef::new(MASK, Domain::Vertex, AttrKind::Scalar)
    }
    /// Face groups: a group id per face.
    pub fn face_group() -> AttrDef {
        AttrDef::new(GROUP, Domain::Face, AttrKind::Label)
    }
    /// Vertex color.
    pub fn color() -> AttrDef {
        AttrDef::new(COLOR, Domain::Vertex, AttrKind::Rgba8)
    }
    /// Hidden faces, one bit each. A vertex is hidden when every face using it is.
    pub fn hide() -> AttrDef {
        AttrDef::new(HIDE, Domain::Face, AttrKind::Flag)
    }
    /// Where Auto UV puts seams ([`UV_HINT`]).
    pub fn uv_hint() -> AttrDef {
        AttrDef::new(UV_HINT, Domain::Vertex, AttrKind::Scalar)
    }
    /// How dense Auto UV's texels are ([`UV_DENSITY`]).
    pub fn uv_density() -> AttrDef {
        AttrDef::new(UV_DENSITY, Domain::Vertex, AttrKind::Scalar)
    }
    /// Whether this is the mask's column, read with no allocation: the kind and domain first,
    /// so most columns stop there.
    pub fn is_mask(&self) -> bool {
        self.range() == Some(Range::MASK)
    }
    /// The values this column keeps: the mask's, a UV hint's, or none for any other column.
    /// The kind and domain are read first, so most columns stop there.
    pub fn range(&self) -> Option<Range> {
        if self.kind != AttrKind::Scalar || self.domain != Domain::Vertex {
            return None;
        }
        match self.name.as_str() {
            MASK => Some(Range::MASK),
            UV_HINT | UV_DENSITY => Some(Range::HINT),
            _ => None,
        }
    }
    /// `value` as this column keeps it: settled in its range (decision 0015, amended
    /// 2026-10-13, and 0061), any other column's value as it is.
    pub fn stored(&self, value: AttrValue) -> AttrValue {
        match (value, self.range()) {
            (AttrValue::F32(x), Some(r)) => AttrValue::F32(r.settle(x)),
            (v, _) => v,
        }
    }
    /// A node's `values` as this column keeps them, each [`AttrDef::stored`].
    pub fn settled(&self, values: AttrVec) -> AttrVec {
        match (values, self.range()) {
            (AttrVec::F32(v), Some(r)) => {
                AttrVec::F32(v.into_iter().map(|x| r.settle(x)).collect())
            }
            (v, _) => v,
        }
    }
    /// A node's `values` as a saved scene loads them: a UV hint settled, so a hand-edited file
    /// opens with its hints in range, and the mask as saved, bit for bit (decision 0016; one not
    /// a number still refuses the Part) (decision 0061, amended 2026-10-13, step UV7).
    pub fn loaded(&self, values: AttrVec) -> AttrVec {
        if self.is_mask() {
            values
        } else {
            self.settled(values)
        }
    }
}

/// Index of an attribute in a mesh's registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AttrId(pub(crate) u8);

impl AttrId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Opaque white, a color column's empty value (red in the low byte, alpha in the high one).
pub(crate) const WHITE: u32 = u32::MAX;

/// A color column's value (sRGB bytes, red in the low byte) as red, green and blue in linear
/// light, 0 to 1.
pub fn linear_rgb(color: u32) -> [f32; 3] {
    [0, 8, 16].map(|shift| srgb_to_linear(f32::from((color >> shift) as u8) / 255.0))
}

/// An sRGB-encoded value, 0 to 1, in linear light, through `libm` so it is the same
/// everywhere: the one copy of the curve on the CPU, which gs-render's color math uses too.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        libm::powf((c + 0.055) / 1.055, 2.4)
    }
}

/// Linear light, 0 to 1, as an sRGB-encoded value: [`srgb_to_linear`]'s inverse, and like it
/// the one copy on the CPU.
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * libm::powf(c, 1.0 / 2.4) - 0.055
    }
}

/// One attribute value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AttrValue {
    F32(f32),
    U32(u32),
}

impl AttrValue {
    /// The value an unallocated column reads as, its empty value: zero, but opaque white for a
    /// color (decision 0039, section 2), so a point nobody painted is white wherever a column
    /// is made or grown from nothing.
    pub fn zero(kind: AttrKind) -> AttrValue {
        match kind {
            AttrKind::Scalar => AttrValue::F32(0.0),
            AttrKind::Label | AttrKind::Flag => AttrValue::U32(0),
            AttrKind::Rgba8 => AttrValue::U32(WHITE),
        }
    }
    /// The empty value of `kind`, bit for bit (so `-0.0` is not zero).
    pub fn is_zero(self, kind: AttrKind) -> bool {
        self.bits() == AttrValue::zero(kind).bits()
    }
    /// The value as raw bits: an `f32`'s bit pattern or a `u32`.
    pub fn bits(self) -> u32 {
        match self {
            AttrValue::F32(x) => x.to_bits(),
            AttrValue::U32(x) => x,
        }
    }
    /// Blend values of column `def`, as it keeps them ([`AttrDef::stored`]: the mask settled):
    /// the mean for scalars and colors, the first for labels.
    pub fn blend(def: &AttrDef, vals: &[AttrValue]) -> AttrValue {
        def.stored(Self::mean(def.kind, vals))
    }

    /// [`AttrValue::blend`]'s arithmetic, by kind alone.
    fn mean(kind: AttrKind, vals: &[AttrValue]) -> AttrValue {
        let n = vals.len() as u32;
        let u = |v: &AttrValue| match *v {
            AttrValue::U32(x) => x,
            AttrValue::F32(x) => x.to_bits(),
        };
        match kind {
            AttrKind::Scalar => {
                let f = |v: &AttrValue| f32::from_bits(u(v));
                AttrValue::F32(vals.iter().map(f).sum::<f32>() * (1.0 / n as f32))
            }
            AttrKind::Label | AttrKind::Flag => AttrValue::U32(u(&vals[0])),
            AttrKind::Rgba8 => {
                let mut out = 0;
                for shift in [0, 8, 16, 24] {
                    let sum: u32 = vals.iter().map(|v| u(v) >> shift & 255).sum();
                    out |= ((sum + n / 2) / n) << shift;
                }
                AttrValue::U32(out)
            }
        }
    }

    /// Blend values of column `def` with weights (decision 0037, section 3), the one rule for
    /// carrying a column from one surface to another: the weighted mean for scalars, per
    /// channel and rounded for colors, the value with the largest weight for labels and flags
    /// (ties to the first), as the column keeps it ([`AttrDef::stored`]: the mask settled).
    /// Weights of one give exactly [`AttrValue::blend`]. Zero weights in all give the first
    /// value.
    pub fn blend_weighted(def: &AttrDef, vals: &[AttrValue], weights: &[f32]) -> AttrValue {
        def.stored(Self::weighted(def.kind, vals, weights))
    }

    /// [`AttrValue::blend_weighted`]'s arithmetic, by kind alone.
    fn weighted(kind: AttrKind, vals: &[AttrValue], weights: &[f32]) -> AttrValue {
        let u = |v: &AttrValue| match *v {
            AttrValue::U32(x) => x,
            AttrValue::F32(x) => x.to_bits(),
        };
        let total: f32 = weights.iter().sum();
        if total.is_nan() || total <= 0.0 {
            return vals[0];
        }
        match kind {
            AttrKind::Scalar => {
                let f = |v: &AttrValue| f32::from_bits(u(v));
                let sum: f32 = vals.iter().zip(weights).map(|(v, w)| f(v) * w).sum();
                AttrValue::F32(sum * (1.0 / total))
            }
            AttrKind::Label | AttrKind::Flag => {
                let mut top = 0;
                for (i, w) in weights.iter().enumerate().take(vals.len()) {
                    if *w > weights[top] {
                        top = i;
                    }
                }
                AttrValue::U32(u(&vals[top]))
            }
            AttrKind::Rgba8 => {
                let mut out = 0;
                for shift in [0, 8, 16, 24] {
                    let sum: f32 = (vals.iter().zip(weights))
                        .map(|(v, w)| (u(v) >> shift & 255) as f32 * w)
                        .sum();
                    let channel = (sum / total + 0.5).floor().clamp(0.0, 255.0) as u32;
                    out |= channel << shift;
                }
                AttrValue::U32(out)
            }
        }
    }
}

/// One node's values of one attribute.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrVec {
    F32(Vec<f32>),
    U32(Vec<u32>),
    /// A `Flag` column.
    Bits(Bits),
}

impl AttrVec {
    /// `len` slots of the kind's empty value ([`AttrValue::zero`]).
    pub(crate) fn zeros(kind: AttrKind, len: usize) -> AttrVec {
        match kind {
            AttrKind::Scalar => AttrVec::F32(vec![0.0; len]),
            AttrKind::Label => AttrVec::U32(vec![0; len]),
            AttrKind::Rgba8 => AttrVec::U32(vec![WHITE; len]),
            AttrKind::Flag => AttrVec::Bits(Bits::zeros(len)),
        }
    }
    /// How many slots the column has.
    #[expect(
        clippy::len_without_is_empty,
        reason = "a node's column always has its slots"
    )]
    pub fn len(&self) -> usize {
        match self {
            AttrVec::F32(v) => v.len(),
            AttrVec::U32(v) => v.len(),
            AttrVec::Bits(b) => b.len(),
        }
    }
    pub(crate) fn get(&self, i: usize) -> AttrValue {
        match self {
            AttrVec::F32(v) => AttrValue::F32(v[i]),
            AttrVec::U32(v) => AttrValue::U32(v[i]),
            AttrVec::Bits(b) => AttrValue::U32(u32::from(b.get(i))),
        }
    }
    /// Slot `i` as raw bits: an `f32`'s bit pattern, a `u32`, or a flag's 0 or 1.
    pub fn bits(&self, i: usize) -> u32 {
        self.get(i).bits()
    }
    /// Write one value. A value of the other type is a caller bug.
    pub(crate) fn set(&mut self, i: usize, x: AttrValue) {
        match (self, x) {
            (AttrVec::F32(v), AttrValue::F32(x)) => v[i] = x,
            (AttrVec::U32(v), AttrValue::U32(x)) => v[i] = x,
            (AttrVec::Bits(b), AttrValue::U32(x)) => b.set(i, x != 0),
            _ => panic!("attribute value of the wrong type"),
        }
    }
    /// Grow or shrink to `len` slots; new slots hold `kind`'s empty value.
    pub(crate) fn resize(&mut self, len: usize, kind: AttrKind) {
        match (self, AttrValue::zero(kind)) {
            (AttrVec::F32(v), AttrValue::F32(x)) => v.resize(len, x),
            (AttrVec::U32(v), AttrValue::U32(x)) => v.resize(len, x),
            (AttrVec::Bits(b), _) => b.resize(len),
            _ => panic!("attribute column of the wrong kind"),
        }
    }
    /// Bytes the values take.
    pub(crate) fn bytes(&self) -> usize {
        match self {
            AttrVec::Bits(b) => b.bytes(),
            other => other.len() * 4,
        }
    }
}

/// A packed ring header: the ring starts at `start` in the pool and has `count` faces.
pub(crate) fn ring_head(start: usize, count: usize) -> u32 {
    ((start as u32) << 8) + count as u32
}

/// One node's topology: faces, vertex rings and slot bookkeeping. One `Arc`, because these
/// always change together. Two are equal when their faces, rings and free lists are: where in
/// the pool a ring sits, and the garbage between rings, do not count.
#[derive(Clone, Debug, Default)]
pub(crate) struct NodeTopo {
    /// Face slots. A dead slot is `[NO; 4]`.
    pub faces: Vec<[u32; 4]>,
    /// Per vertex slot: `start << 8 | count` into `pool`. A dead slot has an empty ring.
    pub ring: Vec<u32>,
    /// Face ids of every ring, each ring sorted ascending. That order makes normal sums
    /// canonical, so normals are bit-identical however they are recomputed.
    pub pool: Vec<u32>,
    /// Dead vertex and face slots (local indices), reused last in, first out.
    pub vfree: Vec<u32>,
    pub ffree: Vec<u32>,
}

impl PartialEq for NodeTopo {
    fn eq(&self, o: &NodeTopo) -> bool {
        self.faces == o.faces
            && self.vfree == o.vfree
            && self.ffree == o.ffree
            && self.ring.len() == o.ring.len()
            && (0..self.ring.len()).all(|l| self.ring_of(l) == o.ring_of(l))
    }
}

impl NodeTopo {
    /// The ring of local vertex slot `l`: every face using it, ascending.
    #[inline]
    pub fn ring_of(&self, l: usize) -> &[u32] {
        let p = self.ring[l];
        let s = (p >> 8) as usize;
        &self.pool[s..s + (p & 255) as usize]
    }
    /// Replace the ring of slot `l` (sorted by the caller). The old entries stay in the pool
    /// as garbage until [`NodeTopo::compact`].
    pub fn set_ring(&mut self, l: usize, r: &[u32]) {
        assert!(r.len() < 256, "vertex valence over 255");
        self.ring[l] = ring_head(self.pool.len(), r.len());
        self.pool.extend_from_slice(r);
    }
    /// Whether the pool holds more garbage (rings since replaced) than live ring entries: when
    /// a topology edit compacts it, so a run of edits costs amortized constant time per ring.
    pub fn mostly_garbage(&self) -> bool {
        let live: usize = self.ring.iter().map(|&p| (p & 255) as usize).sum();
        self.pool.len() > 2 * live
    }
    /// Rewrite the pool with only the live rings, in slot order.
    pub fn compact(&mut self) {
        let mut pool = Vec::new();
        for l in 0..self.ring.len() {
            let r = self.ring_of(l);
            let head = ring_head(pool.len(), r.len());
            pool.extend_from_slice(r);
            self.ring[l] = head;
        }
        self.pool = pool;
    }
}

/// Every node's columns. Node `k` is entry `k` of each vector. Cloning is a reference-count bump
/// per node and column: that is how a history step opens.
#[derive(Clone, Default)]
pub(crate) struct Columns {
    pub pos: Vec<Arc<Vec<Vec3>>>,
    /// Stored, not derived on demand, so undo restores them with the positions for free.
    pub nrm: Vec<Arc<Vec<Vec3>>>,
    pub topo: Vec<Arc<NodeTopo>>,
    /// `attrs[a][k]`: attribute `a` of node `k`, `None` until first written.
    pub attrs: Vec<Vec<Option<Arc<AttrVec>>>>,
}

/// Bit patterns of a node's values of one attribute; a column it does not hold reads as the
/// kind's empty value.
fn attr_bits(col: Option<&AttrVec>, len: usize, kind: AttrKind) -> Vec<u32> {
    match col {
        Some(v) => (0..v.len()).map(|i| v.bits(i)).collect(),
        None => vec![AttrValue::zero(kind).bits(); len],
    }
}

impl Columns {
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// The same allocation for every column of every node.
    pub fn same(&self, o: &Columns) -> bool {
        fn all<T>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y))
        }
        let attr = |a: &Vec<Option<Arc<AttrVec>>>, b: &Vec<Option<Arc<AttrVec>>>| {
            a.len() == b.len()
                && (a.iter().zip(b)).all(|pair| match pair {
                    (Some(x), Some(y)) => Arc::ptr_eq(x, y),
                    (x, y) => x.is_none() && y.is_none(),
                })
        };
        all(&self.pos, &o.pos)
            && all(&self.nrm, &o.nrm)
            && all(&self.topo, &o.topo)
            && self.attrs.len() == o.attrs.len()
            && self.attrs.iter().zip(&o.attrs).all(|(a, b)| attr(a, b))
    }

    /// The same positions, topology and attribute values in every node, bit for bit. `defs`
    /// gives each attribute's kind, so a column a node does not hold reads as its empty value.
    pub fn equal(&self, o: &Columns, defs: &[AttrDef]) -> bool {
        let bits = |p: &[Vec3]| -> Vec<[u32; 3]> { p.iter().map(|q| q.bits()).collect() };
        let nodes = |k: usize| {
            let (t, u) = (&self.topo[k], &o.topo[k]);
            let same_pos =
                Arc::ptr_eq(&self.pos[k], &o.pos[k]) || bits(&self.pos[k]) == bits(&o.pos[k]);
            same_pos && (Arc::ptr_eq(t, u) || t == u)
        };
        let values = |a: usize, k: usize| {
            let (x, y) = (self.attrs[a][k].as_deref(), o.attrs[a][k].as_deref());
            let len = x.or(y).map_or(0, AttrVec::len);
            x.zip(y).is_some_and(|(x, y)| std::ptr::eq(x, y))
                || attr_bits(x, len, defs[a].kind) == attr_bits(y, len, defs[a].kind)
        };
        self.len() == o.len()
            && self.attrs.len() == o.attrs.len()
            && (0..self.len()).all(nodes)
            && (0..self.attrs.len()).all(|a| (0..self.len()).all(|k| values(a, k)))
    }
    /// Remove and return nodes `at..`.
    pub fn split_off(&mut self, at: usize) -> Columns {
        Columns {
            pos: self.pos.split_off(at),
            nrm: self.nrm.split_off(at),
            topo: self.topo.split_off(at),
            attrs: self.attrs.iter_mut().map(|a| a.split_off(at)).collect(),
        }
    }
    /// Append nodes. Attributes registered after `tail` was split off read as unallocated.
    pub fn append(&mut self, tail: Columns) {
        let n = tail.len();
        self.pos.extend(tail.pos);
        self.nrm.extend(tail.nrm);
        self.topo.extend(tail.topo);
        let mut tail_attrs = tail.attrs.into_iter();
        for a in &mut self.attrs {
            match tail_attrs.next() {
                Some(t) => a.extend(t),
                None => a.extend(std::iter::repeat_n(None, n)),
            }
        }
    }
}
