//! `part.create`.

use gs_protocol::{CommandError, Kind, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{NewPart, add_part, triangle};
use crate::command::PartArg;
use crate::document::VertexOrder;
use crate::mesh::problems;
use crate::{Command, Ctx, Example, errors, register_command};

/// Add a part from plain mesh data: positions, faces and attribute columns.
///
/// The mesh must pass `mesh.validate`; an invalid mesh is refused with its problems listed.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartCreate {
    /// Name of the new part.
    #[schemars(length(min = 1))]
    pub name: String,
    /// The mesh.
    pub mesh: PlainMesh,
}

impl Command for PartCreate {
    const NAME: &'static str = "part.create";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::New("part");
    type Output = NewPart;

    fn examples() -> Vec<Example<Self>> {
        let name = "triangle".into();
        let mesh = triangle();
        vec![Example::new("add a single triangle", Self { name, mesh })]
    }

    fn run(self, cx: &mut Ctx) -> Result<NewPart, CommandError> {
        let found = problems(&self.mesh, None);
        if found.is_empty() {
            return add_part(cx, Some(self.name), self.mesh, VertexOrder::AsGiven);
        }
        let kinds: Vec<String> = found
            .iter()
            .map(|p| {
                format!(
                    "{} x{}",
                    serde_json::to_value(p.kind).unwrap_or_default(),
                    p.count
                )
            })
            .collect();
        Err(CommandError {
            hint: Some("run mesh.validate with the same mesh for details".into()),
            ..errors::args_invalid(
                "/mesh",
                format!("the mesh is invalid: {}", kinds.join(", ")),
            )
        })
    }
}
register_command!(PartCreate, "Create part");
