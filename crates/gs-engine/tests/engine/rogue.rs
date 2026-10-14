//! Commands that break the rules, registered only in this test program: queries that write a
//! part or add one. The executor must refuse them and roll the call back, alone or inside a
//! batch, so a future command with this bug cannot slip a change past undo and the log.

use std::collections::BTreeMap;

use gs_engine::{Command, Ctx, Example, VertexOrder, register_command};
use gs_protocol::{CommandError, ErrorCode, Kind, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use gs_testkit::code;
use gs_testkit::failure;

use crate::support::agent;

/// Moves the first vertex of the first part, although it claims to be a query. Exists only in
/// the engine's test program.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RogueQuery {}

impl Command for RogueQuery {
    const NAME: &'static str = "debug.rogue_query";
    const KIND: Kind = Kind::Query;
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("a query that writes is refused", Self {})
                .after("primitive.cube", json!({}))
                .fails_with(ErrorCode::Internal),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<(), CommandError> {
        let first = cx.doc.part_names().next().map(str::to_string);
        let name = first.unwrap_or_default();
        let mut part = cx.doc.write_part(&name)?;
        part.write_positions(&[0])
            .each(|node| node.positions()[0].x += 1.0);
        Ok(())
    }
}
register_command!(RogueQuery, "Rogue query");

/// Adds a part, although it claims to be a query. Exists only in the engine's test program.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RogueAdd {}

impl Command for RogueAdd {
    const NAME: &'static str = "debug.rogue_add";
    const KIND: Kind = Kind::Query;
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new("a query that adds a part is refused", Self {})
                .fails_with(ErrorCode::Internal),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<(), CommandError> {
        let triangle = PlainMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            faces: vec![[0, 1, 2, u32::MAX]],
            attributes: BTreeMap::new(),
        };
        cx.doc
            .insert_part("sneaked", triangle, VertexOrder::AsGiven)
    }
}
register_command!(RogueAdd, "Rogue add");

#[test]
fn a_query_that_adds_a_part_is_refused() {
    let mut s = agent();
    let err = s.err("debug.rogue_add", json!({}));
    assert_eq!(code(err.code), "E_INTERNAL", "refused: {}", err.message);
    assert_eq!(
        s.query("mesh.stats", json!({}))["parts"],
        json!([]),
        "no part"
    );
}

#[test]
fn a_query_that_writes_is_refused_and_rolled_back() {
    let mut s = agent();
    s.ok("primitive.cube", json!({ "name": "a" }));
    let before = s.hash();
    let err = s.err("debug.rogue_query", json!({}));
    assert_eq!(code(err.code), "E_INTERNAL", "refused: {}", err.message);
    let batch = s.batch(&[
        ("primitive.cube", json!({ "name": "b" })),
        ("debug.rogue_query", json!({})),
    ]);
    let err = failure(batch, "a batch with a rogue query");
    assert_eq!(
        (code(err.code), err.path.as_deref()),
        ("E_INTERNAL".into(), Some("/commands/1")),
        "the query step is named: {}",
        err.message
    );
    let batch = s.batch(&[
        (
            "mesh.translate",
            json!({ "part": "a", "direction": [1, 0, 0], "distance": 1 }),
        ),
        ("debug.rogue_query", json!({})),
    ]);
    let err = failure(
        batch,
        "a rogue query writing a part the batch already wrote",
    );
    assert_eq!(
        code(err.code),
        "E_INTERNAL",
        "still refused: {}",
        err.message
    );
    assert_eq!(s.hash(), before, "and everything rolled back");
}
