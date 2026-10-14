//! `symmetry.map` (decision 0021, section 10): find, or release, the mirror map of a part that
//! is already symmetric, so mirrored Live Detail strokes keep its topology exactly symmetric.
//!
//! A part's mirror planes are its own (decision 0029, section 7): `x = 0`, `y = 0` and `z = 0`
//! in part space, through the part's origin and square to its axes, for strokes, transforms
//! and this map alike. The map pairs part-space points, and so does Live Detail's mirror map,
//! so a change of the part's transform (a move, turn or even scale) moves every point and every
//! plane together: neither map changes and neither is built again.

use gs_core::mirror::Fault;
use gs_core::{MeshStore, NO};
use gs_protocol::{CommandError, Kind, Length, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::command::PartArg;
use crate::ctx::PartLength;
use crate::document::{Numbering, pinned};
use crate::sculpt::Axis;
use crate::{Command, Ctx, Example, Unavailable, errors, register_command};

/// Find the mirror map of a part that is already symmetric across a plane through its own
/// origin, or release it. The plane turns and moves with the part: for a part never moved it is
/// the scene's plane through the scene origin.
///
/// Each point's partner is the point nearest its mirror image within `tolerance`; points that
/// are their own partner move exactly onto the plane. The part is refused, unchanged, naming
/// the first point without a partner or the first face without a mirror image;
/// `mesh.symmetrize` makes any part symmetric. While a part holds the map, Live Detail strokes
/// mirrored across its plane refine in mirrored pairs, so the two sides keep exactly matching
/// topology. A part with detail levels is refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymmetryMap {
    #[doc = crate::command::part_doc!("The part.")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// The axis square to the plane: `x` maps across the plane `x = 0`.
    pub axis: Axis,
    /// How far a point may be from its partner's mirror image: scene units, or a percentage
    /// such as "0.01%" (the default) of the part's bounding-box diagonal. From 0 (exact) to
    /// 0.1% of the diagonal: further than that, points would pair with their neighbours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<Length>,
    /// Release the map instead of finding it. `tolerance` means nothing then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear: bool,
}

/// The map a part holds now.
#[derive(Serialize, JsonSchema)]
pub struct MapState {
    pub part: String,
    /// Whether the part holds a map across the axis.
    pub held: bool,
    /// Points on the plane, each its own partner: 0 after a release.
    pub on_plane: u64,
}

/// The default and the largest tolerance, percent of the bounding-box diagonal.
const TOLERANCE: f32 = 0.01;
const MOST: f32 = 0.1;

impl Command for SymmetryMap {
    const NAME: &'static str = "symmetry.map";
    const KIND: Kind = Kind::Edit;
    const PART: PartArg = PartArg::Existing;
    const CLAIMS: &'static str = r#"[{"local": {"part": "/part"}}]"#;
    type Output = MapState;

    fn examples() -> Vec<Example<Self>> {
        let map = |clear| Self {
            part: None,
            axis: Axis::X,
            tolerance: None,
            clear,
        };
        vec![
            Example::new("map a sphere across x", map(false))
                .after("primitive.sphere", json!({ "subdivisions": 4 })),
            Example::new("release a symmetrized sphere's map", map(true))
                .after("primitive.sphere", json!({ "subdivisions": 4 }))
                .after("mesh.symmetrize", json!({ "axis": "x" })),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MapState, CommandError> {
        let part = pinned(self.part)?;
        let axis = self.axis as usize;
        if self.clear {
            cx.noop_reason(format!("part '{part}' holds no map across {}", name(axis)));
            cx.doc.edit_store(&part, |s| s.clear_mirror(axis))?;
            return Ok(MapState {
                part,
                held: false,
                on_plane: 0,
            });
        }
        cx.doc.single_level(&part, "a mirror map")?;
        let tolerance = self.tolerance.unwrap_or(Length::Percent(TOLERANCE));
        let (t, diagonal) = (cx.doc.transform(&part)?, cx.doc.diagonal(&part)?);
        let tolerance = PartLength::new(tolerance, t, || diagonal);
        let most = PartLength::new(Length::Percent(MOST), t, || diagonal).scene;
        if !(0.0..=most).contains(&tolerance.scene) {
            return Err(errors::args_invalid(
                "/tolerance",
                format!(
                    "the tolerance runs from 0 to {MOST}% of the part's diagonal ({most} here); \
                     to pair points further apart, make the part symmetric with mesh.symmetrize"
                ),
            ));
        }
        // The map pairs points in the part's own space (decision 0029, section 7).
        let within = tolerance.part;
        let mut found = Ok(0);
        let store = cx
            .doc
            .edit_store(&part, |s| match s.map_mirror(axis, within) {
                Ok((nodes, on_plane)) => {
                    found = Ok(on_plane);
                    nodes
                }
                Err(fault) => {
                    found = Err(fault);
                    Vec::new()
                }
            })?;
        match found {
            Ok(on_plane) => Ok(MapState {
                part,
                held: true,
                on_plane: on_plane as u64,
            }),
            Err(fault) => Err(refusal(&part, axis, fault, &store, &store.numbering())),
        }
    }
}
register_command!(SymmetryMap, "Map symmetry");

fn name(axis: usize) -> &'static str {
    ["x", "y", "z"][axis]
}

/// Why `part` has no map across `axis`, naming the point or face at fault as the part numbers
/// them.
fn refusal(
    part: &str,
    axis: usize,
    fault: Fault,
    store: &MeshStore,
    at: &Numbering,
) -> CommandError {
    let cfg = store.config();
    let symmetrize = format!(
        "make the part symmetric with mesh.symmetrize {{\"axis\": \"{}\"}}",
        name(axis)
    );
    let reason = match fault {
        Fault::NoPartner(v) => Unavailable::new(format!(
            "point {} of part '{part}' has no partner across {}: no point lies within the \
             tolerance of its mirror image, or that point pairs with another",
            at.index(cfg.node_of(v), cfg.local_of(v)),
            name(axis)
        ))
        .fix(format!("raise the tolerance, or {symmetrize}")),
        Fault::NoMirrorFace(f) => {
            let fc = store.node(cfg.face_node(f)).faces[cfg.face_local(f) as usize];
            let points: Vec<String> = (fc.iter().filter(|&&v| v != NO))
                .map(|&v| at.index(cfg.node_of(v), cfg.local_of(v)).to_string())
                .collect();
            Unavailable::new(format!(
                "the face on points {} of part '{part}' has no mirror image across {}: the \
                 points pair up but the faces do not",
                points.join(", "),
                name(axis)
            ))
            .fix(symmetrize)
        }
        Fault::NameTaken => Unavailable::new(format!(
            "part '{part}' has a column named '{}' that is not a mirror map",
            gs_core::mirror::def(axis).name
        )),
    };
    errors::unavailable(reason)
}
