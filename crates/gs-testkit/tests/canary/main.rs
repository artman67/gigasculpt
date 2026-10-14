//! Canaries (ARCHITECTURE 10.2): small commands with realistic, deliberate bugs, registered
//! only in this test binary, which the claims harness must catch. A check that only an engine
//! bug can trip would otherwise never be seen to fire. Each canary breaks one rule the way a
//! real command plausibly would.

use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::{Command, Ctx, Example, PartArg, VertexOrder, register_command};
use gs_protocol::{CommandError, Kind, PartRef, PlainMesh};
use gs_testkit::Session;
use gs_testkit::claims::run::Harness;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The arguments every canary takes: a part.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OnPart<const N: u8> {
    /// The part.
    #[serde(default)]
    part: Option<PartRef>,
}

impl<const N: u8> OnPart<N> {
    fn part(&self) -> String {
        self.part
            .clone()
            .map(|PartRef(name)| name)
            .unwrap_or_default()
    }
}

/// Crashes after starting to edit, with no example saying it crashes.
type Crash = OnPart<0>;
/// Moves a part by an amount that grows on every call: a hidden counter, not the seed.
type Drift = OnPart<1>;
/// Rebuilds a part with one vertex no face uses. (Flattening faces, the bug this canary used
/// to plant, the engine now refuses on its own.)
type Orphan = OnPart<2>;
/// Claims to touch only its part, but nudges every other part too.
type Spill = OnPart<3>;
/// Claims to keep topology, but rotates the corners of the first face.
type Rewire = OnPart<4>;

static DRIFT: AtomicU32 = AtomicU32::new(1);

impl<const N: u8> Command for OnPart<N> {
    const NAME: &'static str = [
        "canary.crash",
        "canary.drift",
        "canary.orphan",
        "canary.spill",
        "canary.rewire",
    ][N as usize];
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const DOCS: &'static str = "A canary: a command with a deliberate bug.";
    const CLAIMS: &'static str = [
        "[]",
        "[]",
        "[]",
        r#"[{"local": {"part": "/part"}}]"#,
        r#"["preserves_topology"]"#,
    ][N as usize];
    type Output = ();

    fn examples() -> Vec<Example<Self>> {
        vec![Example::raw("on the only part", json!({}))]
    }

    fn run(self, cx: &mut Ctx) -> Result<(), CommandError> {
        let name = self.part();
        if N == 2 || N == 4 {
            let mut mesh = PlainMesh::clone(&*cx.doc.part(&name)?);
            if N == 2 {
                mesh.positions.push([0.0; 3]);
            } else {
                mesh.faces[0][..3].rotate_left(1);
            }
            return cx.doc.insert_part(&name, mesh, VertexOrder::AsGiven);
        }
        if N == 3 {
            // The first vertex of every part moves up, not only this part's.
            let all: Vec<String> = cx.doc.part_names().map(String::from).collect();
            for part in all {
                let mut part = cx.doc.write_part(&part)?;
                part.write_positions(&[0])
                    .each(|node| node.positions()[0].y += 0.5);
            }
            return Ok(());
        }
        let mut part = cx.doc.write_part(&name)?;
        assert!(N != 0, "canary.crash: a bug");
        let step = DRIFT.fetch_add(1, Ordering::Relaxed) as f32 / 64.0;
        let nodes = part.all_nodes();
        part.write_positions(&nodes)
            .each(|node| node.positions().iter_mut().for_each(|p| p.x += step));
        Ok(())
    }
}

register_command!(Crash);
register_command!(Drift);
register_command!(Orphan);
register_command!(Spill);
register_command!(Rewire);

/// The harness's verdict on random plans that call only `command`.
fn verdict(command: &str) -> String {
    let catalog = Session::new().engine.catalog().clone();
    let harness = Harness::new(&catalog).unwrap_or_else(|e| panic!("{e}"));
    match harness.only(&[command]).run() {
        Ok(_) => format!("{command} passed"),
        Err(report) => report,
    }
}

#[test]
fn a_crash_is_caught() {
    let v = verdict("canary.crash");
    assert!(
        v.contains("atomic broken by canary.crash") && v.contains("crashed"),
        "{v}"
    );
}

#[test]
fn hidden_state_is_caught_on_replay() {
    let v = verdict("canary.drift");
    assert!(v.contains("deterministic broken by canary.drift"), "{v}");
}

#[test]
fn a_broken_part_is_caught() {
    let v = verdict("canary.orphan");
    assert!(
        v.contains("valid broken by canary.orphan") && v.contains("UnusedVertex"),
        "{v}"
    );
}

#[test]
fn an_edit_that_spills_into_other_parts_is_caught() {
    let v = verdict("canary.spill");
    assert!(v.contains("local broken by canary.spill"), "{v}");
}

#[test]
fn a_rewiring_that_claims_to_keep_topology_is_caught() {
    let v = verdict("canary.rewire");
    assert!(
        v.contains("preserves_topology broken by canary.rewire"),
        "{v}"
    );
}
