//! `part.set_operation` (decision 0057, section 5): how a part combines with the parts above it
//! in Live Boolean, and whether it starts a group. Setting them moves no point; `boolean.apply`
//! and the live result read them (LB5, LB6).

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::document::{Operation, Props, pinned};
use crate::{Command, Ctx, Example, register_command};

/// Set a part's boolean operation and whether it starts a group.
///
/// Parts combine from the top of the list down, in groups: a group runs from a part that starts
/// one (the first part always does) to the part before the next. In its group a part is added
/// to what the parts above it make (`union`), cut out of it (`difference`), or keeps only what
/// both share (`intersect`); the group's first part that shows is its base, whose operation is
/// ignored. No point moves: the marks are one undo step and are saved with the scene. A field
/// left out keeps its value. A duplicate or mirror copy of the part is a `union` that starts no
/// group; split pieces keep its operation and start no group.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartSetOperation {
    #[doc = crate::command::part_doc!("The part to mark.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// How it combines with the parts above it in its group. Left out: kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<Operation>,
    /// It starts a group (true) or belongs to the group above it (false). Left out: kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<bool>,
}

/// The part's marks after the call.
#[derive(Serialize, JsonSchema)]
pub struct Marks {
    /// The part.
    pub part: String,
    /// How it combines with the parts above it in its group.
    pub operation: Operation,
    /// It starts a group.
    pub start: bool,
}

impl Command for PartSetOperation {
    const NAME: &'static str = "part.set_operation";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Instant;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"["preserves_topology", {"local": {"part": "/part"}}]"#;
    type Output = Marks;

    fn examples() -> Vec<Example<Self>> {
        let cube = || Some(PartRef("cube".into()));
        let mark = |summary: &'static str, part, operation, start| {
            let args = Self {
                part,
                operation,
                start,
            };
            (Example::new(summary, args))
                .after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
        };
        let cut = Some(Operation::Difference);
        let intersect = Some(Operation::Intersect);
        vec![
            mark("cut the cube out of the sphere", cube(), cut, None),
            mark("start a new group at the cube", cube(), None, Some(true)),
            mark("keep what the active part shares", None, intersect, None),
            Example::raw(
                "subtract is a brush's word",
                json!({ "operation": "subtract" }),
            )
            .after("primitive.cube", json!({}))
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Marks, CommandError> {
        let part = pinned(self.part)?;
        let was = cx.doc.get_part(&part)?.props();
        let now = Props {
            operation: self.operation.unwrap_or(was.operation),
            start: self.start.unwrap_or(was.start),
            ..was
        };
        cx.noop_reason(format!("{part} already has that operation and start mark"));
        cx.doc.set_props(&part, now)?;
        Ok(Marks {
            part,
            operation: now.operation,
            start: now.start,
        })
    }
}
register_command!(PartSetOperation, "Set boolean operation");
