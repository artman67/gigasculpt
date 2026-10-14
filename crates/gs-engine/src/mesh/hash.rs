//! `mesh.hash`: BLAKE3 over a canonical byte form of a mesh or of the whole scene.
//!
//! Canonical form of a mesh, all integers little-endian:
//! `"gigasculpt mesh v1"`, then vertex count (u64) and every coordinate's f32 bits (u32),
//! then face count (u64) and every face's four indices (u32), then attribute count (u64) and,
//! per attribute in name order: name length (u64), name bytes, domain (u8: 0 vertex, 1 face),
//! value type (u8: 0 f32, 1 u32), value count (u64), values as u32 bits.
//! The scene: `"gigasculpt document v1"`, part count (u64), then per part in name order: name
//! length (u64), name bytes, the part's 32-byte mesh hash; for a part with detail levels
//! (decision 0014) then also `"levels"`, the level count and the active level (u64 each) and
//! the top level's 32-byte mesh hash, since the active level alone does not show the top; for
//! a frozen part (decision 0030) then also `"frozen"`, the frozen levels' count and active
//! level (u64 each), and their active and top levels' 32-byte mesh hashes; for a part whose
//! list order or visibility is not the default (decision 0028) then also `"list"`, its rank
//! (u64), its eye (u8: 1 off) and a 0 (u8; it held the Isolate mark until Isolate became
//! session state, which the hash leaves out, decision 0029); for a part in Voxel mode then also
//! `"voxel mode"`; for a part whose paint is hidden (decision 0039, section 10) then also
//! `"paint hidden"`; for a part whose Live Boolean operation or start is not the default
//! (decision 0057) then also `"boolean"`, its operation (u8: 0 union, 1 difference, 2
//! intersect) and its start (u8: 1 on); for a part whose transform is not
//! the identity (decision 0029) then also `"transform"` and the f32 bits (u32) of its
//! translation x, y, z, rotation x, y, z, w and scale. A part's mesh hash in the scene is of
//! its points in its own space: an edit can change a point by less than the rounding of its
//! scene point.
//! Equal hashes mean bit-identical data, so `-0.0` and `0.0` hash differently.

use gs_protocol::{AttrDomain, AttrValues, CommandError, Kind, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::document::Document;
use crate::{Command, Ctx, Example, register_command};

/// Hash a part, a given mesh, or the whole scene, to check whether two states are
/// bit-identical.
///
/// Give `part` or `mesh`, not both. With neither, hashes the whole scene: every part's name and mesh. Equal
/// hashes mean bit-identical data. Used by replay checks and tests.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"required": ["part", "mesh"]}))]
pub struct MeshHash {
    /// The part to hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// A mesh to hash instead of a part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<PlainMesh>,
}

/// A hash.
#[derive(Serialize, JsonSchema)]
pub struct HashOutput {
    /// BLAKE3 hash as 64 hex digits.
    pub hash: String,
}

impl Command for MeshHash {
    const NAME: &'static str = "mesh.hash";
    const KIND: Kind = Kind::Query;
    type Output = HashOutput;

    fn examples() -> Vec<Example<Self>> {
        let all = Self {
            part: None,
            mesh: None,
        };
        vec![Example::new("hash the whole scene", all).after("primitive.cube", json!({}))]
    }

    fn run(self, cx: &mut Ctx) -> Result<HashOutput, CommandError> {
        let hash = match (&self.part, &self.mesh) {
            (Some(PartRef(name)), _) => digest(&*cx.doc.part(name)?),
            (None, Some(mesh)) => digest(mesh),
            (None, None) => document_digest(cx.doc.document()),
        };
        Ok(HashOutput {
            hash: hash.to_hex().to_string(),
        })
    }
}
register_command!(MeshHash, "Hash mesh");

/// The hash of one mesh.
pub(crate) fn digest(mesh: &PlainMesh) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update(b"gigasculpt mesh v1");
    put_words(
        &mut h,
        mesh.positions.len(),
        mesh.positions.iter().flatten().map(|c| c.to_bits()),
    );
    put_words(
        &mut h,
        mesh.faces.len(),
        mesh.faces.iter().flatten().copied(),
    );
    put_len(&mut h, mesh.attributes.len());
    for (name, attribute) in &mesh.attributes {
        put_bytes(&mut h, name.as_bytes());
        let domain = match attribute.domain {
            AttrDomain::Vertex => 0,
            AttrDomain::Face => 1,
        };
        match &attribute.values {
            AttrValues::F32(values) => {
                h.update(&[domain, 0]);
                put_words(&mut h, values.len(), values.iter().map(|v| v.to_bits()));
            }
            AttrValues::U32(values) => {
                h.update(&[domain, 1]);
                put_words(&mut h, values.len(), values.iter().copied());
            }
        }
    }
    h.finalize()
}

/// The hash of the whole scene.
pub(crate) fn document_digest(doc: &Document) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    h.update(b"gigasculpt document v1");
    put_len(&mut h, doc.len());
    for (name, part) in doc.parts() {
        put_bytes(&mut h, name.as_bytes());
        h.update(digest(&part.view()).as_bytes());
        if let Some((count, active, top)) = part.top_view() {
            h.update(b"levels");
            put_len(&mut h, count);
            put_len(&mut h, active);
            h.update(digest(&top).as_bytes());
        }
        if let Some((count, active, held, top)) = part.frozen_view() {
            h.update(b"frozen");
            put_len(&mut h, count);
            put_len(&mut h, active);
            h.update(digest(&held).as_bytes());
            h.update(digest(&top).as_bytes());
        }
        if let Some(marks) = part.props().list_marks() {
            h.update(b"list");
            h.update(&marks);
        }
        if part.props().voxel_mode {
            h.update(b"voxel mode");
        }
        if part.props().paint_hidden {
            h.update(b"paint hidden");
        }
        if let Some(marks) = part.props().boolean_marks() {
            h.update(b"boolean");
            h.update(&marks);
        }
        let t = part.transform();
        if !t.is_identity() {
            h.update(b"transform");
            let numbers = (t.translation.to_array().into_iter())
                .chain(t.rotation.to_array())
                .chain([t.scale]);
            for c in numbers {
                h.update(&c.to_bits().to_le_bytes());
            }
        }
    }
    h.finalize()
}

fn put_len(h: &mut blake3::Hasher, len: usize) {
    h.update(&(len as u64).to_le_bytes());
}

fn put_bytes(h: &mut blake3::Hasher, bytes: &[u8]) {
    put_len(h, bytes.len());
    h.update(bytes);
}

fn put_words(h: &mut blake3::Hasher, count: usize, words: impl Iterator<Item = u32>) {
    put_len(h, count);
    let bytes: Vec<u8> = words.flat_map(u32::to_le_bytes).collect();
    h.update(&bytes);
}
