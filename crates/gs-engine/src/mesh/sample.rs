//! `mesh.sample`: a few vertices of a part, listed or nearest a point.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, CommandError, ErrorCode, Kind, PartRef, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::pinned;
use crate::errors::args_invalid;
use crate::math::{dot, sub};
use crate::{Command, Ctx, Example, register_command};

/// Read a few vertices of a part without exporting it: their positions, normals and attribute
/// values.
///
/// Give either `ids` (returned in the order given) or `near` with `k` (the `k` vertices
/// nearest the point, nearest first, equally near ones lowest id first). Ids are the vertex
/// numbers `part.get` uses. Normals are area-weighted, as the mesh store keeps them. At most
/// 1,000 vertices per call.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshSample {
    #[doc = crate::command::part_doc!("The part to read.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The vertices to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 1000))]
    pub ids: Option<Vec<u32>>,
    /// Read the vertices nearest this point instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<[f64; 3]>,
    /// How many vertices nearest `near` to read (fewer if the part has fewer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 1000))]
    pub k: Option<u32>,
    /// The vertex attribute columns to read. Left out: every vertex column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<String>>,
}

/// The vertices read.
#[derive(Serialize, JsonSchema)]
pub struct Sampled {
    /// One entry per vertex.
    pub vertices: Vec<SampledVertex>,
}

/// One vertex.
#[derive(Serialize, JsonSchema)]
pub struct SampledVertex {
    /// Its id.
    pub id: u32,
    /// Its position.
    pub position: Vec3,
    /// Its unit normal.
    pub normal: Vec3,
    /// The value of each column read.
    pub attributes: BTreeMap<String, Value>,
}

/// One attribute value.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum Value {
    /// From a float column.
    F32(f32),
    /// From an integer column.
    U32(u32),
}

impl Command for MeshSample {
    const NAME: &'static str = "mesh.sample";
    const KIND: Kind = Kind::Query;
    const PART: PartArg = PartArg::Existing;
    type Output = Sampled;

    fn examples() -> Vec<Example<Self>> {
        let call = |ids, near, k, attributes| Self {
            part: Some(PartRef("masked".into())),
            ids,
            near,
            k,
            attributes,
        };
        let tetrahedron = || {
            let mesh = json!({
                "positions": [[0, 0, 0], [1, 0, 0], [0, 1, 0], [0, 0, 1]],
                "faces": [
                    [0, 2, 1, 4_294_967_295_u32],
                    [0, 1, 3, 4_294_967_295_u32],
                    [0, 3, 2, 4_294_967_295_u32],
                    [1, 2, 3, 4_294_967_295_u32]
                ],
                "attributes": {
                    "mask": {"domain": "vertex", "values": {"f32": [0, 0.5, 0, 0]}},
                    "group": {"domain": "face", "values": {"u32": [1, 1, 2, 2]}}
                }
            });
            json!({ "name": "masked", "mesh": mesh })
        };
        vec![
            Example::new(
                "read three vertices of a tetrahedron",
                call(Some(vec![0, 1, 2]), None, None, None),
            )
            .after("part.create", tetrahedron()),
            Example::new(
                "read the mask of the 2 vertices nearest the top",
                call(
                    None,
                    Some([0.0, 1.0, 0.0]),
                    Some(2),
                    Some(vec!["mask".into()]),
                ),
            )
            .after("part.create", tetrahedron()),
            Example::new(
                "a column the part does not have is refused",
                call(Some(vec![0]), None, None, Some(vec!["color".into()])),
            )
            .after("part.create", tetrahedron())
            .fails_with(ErrorCode::ArgsInvalid),
            Example::new(
                "a face column is refused: it has no value per vertex",
                call(Some(vec![0]), None, None, Some(vec!["group".into()])),
            )
            .after("part.create", tetrahedron())
            .fails_with(ErrorCode::ArgsInvalid),
            Example::new(
                "near without k is refused",
                call(None, Some([0.0; 3]), None, None),
            )
            .after("part.create", tetrahedron())
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Sampled, CommandError> {
        let part = pinned(self.part)?;
        let mesh = cx.doc.part(&part)?;
        let count = mesh.positions.len();
        let ids = match (self.ids, self.near, self.k) {
            (Some(ids), None, None) => {
                if let Some(i) = ids.iter().position(|&v| v as usize >= count) {
                    let message = format!("the part has {count} vertices");
                    return Err(args_invalid(&format!("/ids/{i}"), message));
                }
                ids
            }
            (None, Some(near), Some(k)) => {
                let distance = |p: [f32; 3]| {
                    let d = sub(p.map(f64::from), near);
                    dot(d, d)
                };
                let mut order: Vec<(f64, u32)> = (mesh.positions.iter().zip(0..))
                    .map(|(&p, v)| (distance(p), v))
                    .collect();
                order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                order.into_iter().take(k as usize).map(|(_, v)| v).collect()
            }
            _ => return Err(args_invalid("", "give either ids, or near with k")),
        };
        let names = match self.attributes {
            Some(names) => names,
            None => (mesh.attributes.iter())
                .filter(|(_, a)| a.domain == AttrDomain::Vertex)
                .map(|(name, _)| name.clone())
                .collect(),
        };
        let mut columns = Vec::new();
        for (i, name) in names.into_iter().enumerate() {
            match mesh.attributes.get(&name) {
                Some(a) if a.domain == AttrDomain::Vertex => columns.push((name, &a.values)),
                _ => {
                    let message = format!("the part has no vertex column named '{name}'");
                    return Err(args_invalid(&format!("/attributes/{i}"), message));
                }
            }
        }
        let normals = cx.doc.normals(&part)?;
        let vertices = ids
            .into_iter()
            .map(|id| {
                let v = id as usize;
                let attributes = (columns.iter())
                    .map(|(name, values)| {
                        let value = match values {
                            AttrValues::F32(x) => Value::F32(x[v]),
                            AttrValues::U32(x) => Value::U32(x[v]),
                        };
                        (name.clone(), value)
                    })
                    .collect();
                SampledVertex {
                    id,
                    position: Vec3(mesh.positions[v]),
                    normal: Vec3(normals[v]),
                    attributes,
                }
            })
            .collect();
        Ok(Sampled { vertices })
    }
}
register_command!(MeshSample, "Sample points");
