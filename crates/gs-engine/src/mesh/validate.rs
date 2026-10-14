//! `mesh.validate`: everything structurally wrong with a mesh.

use std::collections::BTreeMap;

use gs_protocol::{AttrDomain, AttrValues, CommandError, Kind, NO_VERTEX, PartRef, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{corners, face_edges};
use crate::{Command, Ctx, Example, register_command};

/// How many element indices a problem lists.
const FIRST: usize = 10;

/// List what is structurally wrong with a part, a given mesh, or every part.
///
/// Give `part` or `mesh`, not both; with neither, every part is checked.
///
/// Checks finite coordinates, face indices and triangle markers, no vertex repeated within a
/// face, every edge shared by at most two faces that run along it in opposite directions
/// (consistent winding), no unused vertices, and attribute lengths and values. Open boundaries
/// and faces of zero area are allowed: sculpting and imported files make them on purpose.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("not" = {"required": ["part", "mesh"]}))]
pub struct MeshValidate {
    /// The part to check. Omit, with no `mesh`, to check every part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// A mesh to check instead of a part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh: Option<PlainMesh>,
}

/// The result of a check.
#[derive(Serialize, JsonSchema)]
pub struct ValidateOutput {
    /// True if no problems were found.
    pub valid: bool,
    /// Every kind of problem found, with where.
    pub problems: Vec<Problem>,
}

/// One kind of problem in one part or attribute.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Problem {
    /// The part it was found in. Missing for a mesh given directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// What is wrong.
    pub kind: ProblemKind,
    /// The attribute concerned, for attribute problems.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    /// How many elements have the problem.
    pub count: u64,
    /// The first 10: vertex indices, face indices, the lower vertex of each bad edge, or for
    /// `attribute_length` the number of values found.
    pub first: Vec<u64>,
}

/// A kind of structural problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// A vertex coordinate is NaN or infinite.
    NonFinitePosition,
    /// A face has the triangle marker 4294967295 in one of its first three slots.
    MisplacedMarker,
    /// A face uses a vertex index past the end of the vertex list.
    IndexOutOfRange,
    /// A face uses the same vertex twice.
    RepeatedVertex,
    /// An edge is shared by more than two faces.
    NonManifoldEdge,
    /// Two faces run along an edge in the same direction, so their windings disagree.
    FlippedEdge,
    /// No face uses this vertex.
    UnusedVertex,
    /// An attribute has the wrong number of values for its domain.
    AttributeLength,
    /// An attribute value is NaN or infinite.
    NonFiniteAttribute,
}

impl Command for MeshValidate {
    const NAME: &'static str = "mesh.validate";
    const KIND: Kind = Kind::Query;
    type Output = ValidateOutput;

    fn examples() -> Vec<Example<Self>> {
        let every = Self {
            part: None,
            mesh: None,
        };
        let triangle = Self {
            part: None,
            mesh: Some(crate::part::triangle()),
        };
        vec![
            Example::new("check every part", every).after("primitive.sphere", json!({})),
            Example::new("check a mesh before importing it", triangle),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<ValidateOutput, CommandError> {
        let problems: Vec<Problem> = match self.mesh {
            Some(mesh) => problems(&mesh, None),
            None => super::selected(&cx.doc, self.part.as_ref())?
                .into_iter()
                .flat_map(|(name, mesh)| problems(&mesh, Some(&name)))
                .collect(),
        };
        Ok(ValidateOutput {
            valid: problems.is_empty(),
            problems,
        })
    }
}
register_command!(MeshValidate, "Check mesh");

/// Collects problem indices by kind and attribute.
#[derive(Default)]
struct Tally(BTreeMap<(ProblemKind, Option<String>), Vec<u64>>);

impl Tally {
    fn note(&mut self, kind: ProblemKind, attribute: Option<&str>, index: usize) {
        self.0
            .entry((kind, attribute.map(str::to_string)))
            .or_default()
            .push(index as u64);
    }
}

/// Every problem in a mesh, sorted by kind, tagged with the part's name if it is one.
pub(crate) fn problems(mesh: &PlainMesh, part: Option<&str>) -> Vec<Problem> {
    let mut tally = Tally::default();
    check_values(mesh, &mut tally);
    check_faces(mesh, &mut tally);
    tally.problems(part)
}

/// The value problems of a mesh: positions and attribute values that are not finite, and
/// attributes of the wrong length. No edge tables, so it suits meshes of any size (imports).
pub(crate) fn value_problems(mesh: &PlainMesh, part: Option<&str>) -> Vec<Problem> {
    let mut tally = Tally::default();
    check_values(mesh, &mut tally);
    tally.problems(part)
}

/// Position and attribute problems.
fn check_values(mesh: &PlainMesh, tally: &mut Tally) {
    for (i, position) in mesh.positions.iter().enumerate() {
        if !position.iter().all(|c| c.is_finite()) {
            tally.note(ProblemKind::NonFinitePosition, None, i);
        }
    }
    check_attributes(mesh, tally);
}

impl Tally {
    fn problems(self, part: Option<&str>) -> Vec<Problem> {
        self.0
            .into_iter()
            .map(|((kind, attribute), mut first)| {
                let count = first.len() as u64;
                first.truncate(FIRST);
                Problem {
                    part: part.map(str::to_string),
                    kind,
                    attribute,
                    count,
                    first,
                }
            })
            .collect()
    }
}

/// Face, edge and vertex-use problems.
fn check_faces(mesh: &PlainMesh, tally: &mut Tally) {
    let vertices = mesh.positions.len();
    let mut used = vec![false; vertices];
    let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut undirected: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (f, face) in mesh.faces.iter().enumerate() {
        let corners = corners(face);
        let kind = if corners.contains(&NO_VERTEX) {
            ProblemKind::MisplacedMarker
        } else if corners.iter().any(|&v| v as usize >= vertices) {
            ProblemKind::IndexOutOfRange
        } else if corners
            .iter()
            .enumerate()
            .any(|(i, v)| corners[i + 1..].contains(v))
        {
            ProblemKind::RepeatedVertex
        } else {
            for (a, b) in face_edges(corners) {
                used[a as usize] = true;
                *directed.entry((a, b)).or_default() += 1;
                *undirected.entry((a.min(b), a.max(b))).or_default() += 1;
            }
            continue;
        };
        tally.note(kind, None, f);
    }
    for ((a, b), faces) in undirected {
        if faces > 2 {
            tally.note(ProblemKind::NonManifoldEdge, None, a.min(b) as usize);
        }
    }
    for ((a, b), runs) in directed {
        if runs > 1 {
            tally.note(ProblemKind::FlippedEdge, None, a.min(b) as usize);
        }
    }
    for (v, _) in used.iter().enumerate().filter(|(_, used)| !**used) {
        tally.note(ProblemKind::UnusedVertex, None, v);
    }
}

/// Attribute length and value problems.
fn check_attributes(mesh: &PlainMesh, tally: &mut Tally) {
    for (name, attribute) in &mesh.attributes {
        let expected = match attribute.domain {
            AttrDomain::Vertex => mesh.positions.len(),
            AttrDomain::Face => mesh.faces.len(),
        };
        let (len, not_finite): (usize, Vec<usize>) = match &attribute.values {
            AttrValues::F32(values) => (
                values.len(),
                values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !v.is_finite())
                    .map(|(i, _)| i)
                    .collect(),
            ),
            AttrValues::U32(values) => (values.len(), Vec::new()),
        };
        if len != expected {
            tally.note(ProblemKind::AttributeLength, Some(name), len);
        }
        for i in not_finite {
            tally.note(ProblemKind::NonFiniteAttribute, Some(name), i);
        }
    }
}
