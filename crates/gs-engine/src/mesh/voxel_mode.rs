//! `mesh.voxel_mode` and `mesh.voxel_update`: Voxel mode (catalog `geometry.voxel-remesh`,
//! decision 0002 as amended 2026-10-12), a switch on the part that remeshes it when turned on
//! and lets the window's update gesture remesh it again at the cell size it keeps. And
//! `mesh.voxel_settings`, the session's hole closing for every voxel call that leaves it out
//! (decision 0052 as amended 2026-10-12).

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::voxel_remesh::{MeshVoxelRemesh, VoxelRemeshed};
use crate::command::PartArg;
use crate::document::{Props, pinned};
use crate::errors::error;
use crate::{Command, Ctx, Example, register_command};

/// Turn a part's Voxel mode on or off.
///
/// On: the part is voxel remeshed at the cell size it keeps (resolution 128 on a part never
/// remeshed), as `mesh.voxel_remesh` with no arguments does, and from then on
/// `mesh.voxel_update` (in the window, a Ctrl drag on empty canvas with the mask clear)
/// remeshes it again at that size. Off: the mesh stays as it is. The part keeps its cell size
/// either way, so turning it on again remeshes at the same size. A part with detail levels is
/// refused as a voxel remesh refuses it. The switch is saved with the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshVoxelMode {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// On (true) or off.
    pub on: bool,
    /// Hole closing for the remesh that turning it on runs: 0 none or 1 large triangles, as
    /// `mesh.voxel_remesh` takes it. Left out: the session's (`mesh.voxel_settings`), written
    /// into the logged call. Turning it off remeshes nothing and uses none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: Option<u8>,
}

/// What turning Voxel mode on or off did.
#[derive(Serialize, JsonSchema)]
pub struct VoxelMode {
    /// The part.
    pub part: String,
    /// Its switch now.
    pub on: bool,
    /// The remesh that turning it on ran; null when it was turned off or already on.
    pub remesh: Option<VoxelRemeshed>,
}

impl Command for MeshVoxelMode {
    const NAME: &'static str = "mesh.voxel_mode";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    // Replaces only its own part, as a voxel remesh does.
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = VoxelMode;

    fn examples() -> Vec<Example<Self>> {
        let sphere = |e: Example<Self>| e.after("primitive.sphere", json!({ "subdivisions": 4 }));
        vec![
            sphere(Example::raw(
                "turn Voxel mode on, remeshing the sphere",
                json!({ "on": true }),
            )),
            sphere(Example::raw(
                "turn Voxel mode off, leaving the mesh as it is",
                json!({ "on": false }),
            ))
            .after("mesh.voxel_mode", json!({ "on": true })),
            sphere(Example::raw(
                "turn Voxel mode on, leaving any hole open",
                json!({ "on": true, "close_holes": 0 }),
            )),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        if self.on {
            self.close_holes.get_or_insert(session_closing(cx));
        }
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<VoxelMode, CommandError> {
        let (part, on) = (pinned(self.part)?, self.on);
        if cx.doc.get_part(&part)?.props().voxel_mode == on {
            let state = if on { "on" } else { "off" };
            cx.noop_reason(format!("Voxel mode is already {state} for '{part}'"));
            return Ok(VoxelMode {
                part,
                on,
                remesh: None,
            });
        }
        let remesh = if on {
            Some(remesh(cx, &part, self.close_holes)?)
        } else {
            None
        };
        let props = cx.doc.get_part(&part)?.props();
        cx.doc.set_props(
            &part,
            Props {
                voxel_mode: on,
                ..props
            },
        )?;
        Ok(VoxelMode { part, on, remesh })
    }
}
register_command!(MeshVoxelMode, "Voxel mode");

/// Remesh a part in Voxel mode again, at the cell size it keeps: in the window, a Ctrl drag on
/// empty canvas with the mask clear.
///
/// The same remesh as `mesh.voxel_remesh` with only `close_holes`. Refused for a part whose
/// Voxel mode is off (`mesh.voxel_mode`), and, as a voxel remesh is, for a part with detail
/// levels.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshVoxelUpdate {
    #[doc = crate::command::part_doc!("The part to remesh.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Hole closing: 0 none or 1 large triangles, as `mesh.voxel_remesh` takes it. Left out:
    /// the session's (`mesh.voxel_settings`), written into the logged call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: Option<u8>,
}

impl Command for MeshVoxelUpdate {
    const NAME: &'static str = "mesh.voxel_update";
    const KIND: Kind = Kind::Edit;
    const COST: Cost = Cost::Long;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    // Changes something only on a part in Voxel mode: its claims are checked on plans that
    // start by turning Voxel mode on for each part.
    const SETUP: &'static str = r#"[{"command": "mesh.voxel_mode", "args": {"on": true}}]"#;
    type Output = VoxelRemeshed;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::raw(
                "remesh a sphere in Voxel mode again after stretching it",
                json!({}),
            )
            .after("primitive.sphere", json!({ "subdivisions": 4 }))
            .after("mesh.voxel_mode", json!({ "on": true }))
            .after("transform.apply", json!({ "op": { "scale": [1.5, 1, 1] } })),
            Example::raw("refused for a part not in Voxel mode", json!({}))
                .after("primitive.sphere", json!({}))
                .fails_with(ErrorCode::Precondition),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        self.close_holes.get_or_insert(session_closing(cx));
        Ok(())
    }

    fn run(self, cx: &mut Ctx) -> Result<VoxelRemeshed, CommandError> {
        let part = pinned(self.part)?;
        if !cx.doc.get_part(&part)?.props().voxel_mode {
            return Err(CommandError {
                hint: Some(
                    "turn Voxel mode on with mesh.voxel_mode, or remesh once with \
                     mesh.voxel_remesh"
                        .into(),
                ),
                ..error(
                    ErrorCode::Precondition,
                    format!(
                        "'{part}' is not in Voxel mode: the update remeshes a part in Voxel mode"
                    ),
                )
            });
        }
        remesh(cx, &part, self.close_holes)
    }
}
register_command!(MeshVoxelUpdate, "Voxel update");

/// `mesh.voxel_remesh` on `part` with only `close_holes`: at the cell size it keeps.
fn remesh(
    cx: &mut Ctx,
    part: &str,
    close_holes: Option<u8>,
) -> Result<VoxelRemeshed, CommandError> {
    let button = MeshVoxelRemesh {
        part: Some(PartRef(part.into())),
        resolution: None,
        voxel_size: None,
        keep_groups: false,
        project: false,
        blur: 0.0,
        close_holes,
    };
    button.run(cx)
}

/// The session's hole closing: what a voxel call that leaves `close_holes` out uses.
pub(crate) fn session_closing(cx: &Ctx) -> u8 {
    cx.sculpt.settings.voxel.close_holes
}

/// The session's Voxel Remesh settings (`mesh.voxel_settings`), which every voxel call that
/// leaves one out takes into its logged call.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct VoxelSettings {
    /// Hole closing: 0 none, 1 large triangles.
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: u8,
}

impl Default for VoxelSettings {
    fn default() -> Self {
        Self { close_holes: 1 }
    }
}

/// Set the hole closing that `mesh.voxel_remesh`, `mesh.voxel_mode` and `mesh.voxel_update`
/// use when a call leaves `close_holes` out.
///
/// Each such call writes the value it used into its logged call, so a log replays the same
/// whatever the hole closing where it replays. In the window, the preferences row Voxel Remesh
/// hole closing, kept across restarts. It lasts the session and changes no part.
#[derive(Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshVoxelSettings {
    /// Hole closing: 0, none (a hole stays open about where its rim was), or 1, large
    /// triangles (each hole closed by a patch across its rim). 1 at first. Left out:
    /// unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0, max = 1))]
    pub close_holes: Option<u8>,
}

impl Command for MeshVoxelSettings {
    const NAME: &'static str = "mesh.voxel_settings";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = VoxelSettings;

    fn examples() -> Vec<Example<Self>> {
        let closing = |close_holes| Self { close_holes };
        let open = "leave holes open in every voxel remesh that does not say";
        vec![
            Example::new(open, closing(Some(0))),
            Example::new(
                "report the settings after closing holes again",
                closing(None),
            )
            .after("mesh.voxel_settings", json!({ "close_holes": 1 })),
            Example::raw("hole closing 2 is later work", json!({ "close_holes": 2 }))
                .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<VoxelSettings, CommandError> {
        let voxel = &mut cx.sculpt.settings.voxel;
        voxel.close_holes = self.close_holes.unwrap_or(voxel.close_holes);
        Ok(*voxel)
    }
}
register_command!(MeshVoxelSettings, "Voxel Remesh settings");
