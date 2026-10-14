//! The visibility commands (decision 0015): `visibility.hide`, `visibility.show` and
//! `visibility.only` take a selector; `visibility.invert` and `visibility.show_all` act on the
//! whole part; `visibility.grow` and `visibility.shrink` (decision 0026) grow or shrink what is
//! visible. Each is one undo step and runs through gs-core's `visibility`, `select` and `faces`
//! modules. Hidden faces are not drawn, not sculpted and not crossed by smoothing.

mod morph;
mod pick;
mod whole;

use gs_core::select::{FaceRule, Selector};
use gs_core::{MeshStore, NO, NodeId, View, VisibilityEdit};
use gs_protocol::{CommandError, HIDE};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::sculpt::aim_camera;
use crate::view::ViewSpec;
use crate::{Ctx, errors};

/// How a face's mask comes from its corners'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RuleArg {
    /// The mean of its corners.
    #[default]
    Mean,
    /// Its most masked corner: a face counts as masked if any corner is.
    Any,
    /// Its least masked corner: a face counts as masked only if every corner is.
    All,
}

/// Which faces: by mask, by face group, by visibility, by index, or facing the call's camera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "by", rename_all = "lowercase", deny_unknown_fields)]
pub enum SelectArg {
    /// Every face.
    All,
    /// Faces not hidden.
    Visible,
    /// Hidden faces.
    Hidden,
    /// These faces, by index in the part's numbering (the order `part.get` lists them).
    Faces {
        #[schemars(length(min = 1))]
        indexes: Vec<u32>,
    },
    /// Faces turned toward the call's `camera`: the angle between a face's normal and the
    /// direction to the camera is at most `angle` degrees, 0 to 90 (left out, 90: every face
    /// turned toward it). Nothing tests what is in front, so inner surfaces facing the camera
    /// count.
    Facing {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 90.0))]
        angle: Option<f32>,
    },
    /// Faces whose mask is at least `threshold` (left out, 0.5).
    Masked {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 1.0))]
        threshold: Option<f32>,
        /// How a face's mask comes from its corners'. Left out, their mean.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<RuleArg>,
    },
    /// Faces whose mask is below `threshold` (left out, 0.5).
    Unmasked {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(range(min = 0.0, max = 1.0))]
        threshold: Option<f32>,
        /// How a face's mask comes from its corners'. Left out, their mean.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<RuleArg>,
    },
    /// Faces in any of these face groups (0 is no group).
    Groups {
        #[schemars(length(min = 1))]
        ids: Vec<u32>,
    },
}

impl SelectArg {
    /// The selector, for `part` (whose numbering a face list uses), seen from `camera` (the
    /// call's, which only a facing selector takes).
    pub(crate) fn resolve(
        &self,
        cx: &Ctx,
        part: &str,
        camera: Option<View>,
    ) -> Result<Selector, CommandError> {
        if camera.is_some() && !matches!(self, SelectArg::Facing { .. }) {
            let message = "only a facing selector takes a camera";
            return Err(errors::args_invalid("/camera", message));
        }
        let rule = |r: Option<RuleArg>| match r.unwrap_or_default() {
            RuleArg::Mean => FaceRule::Mean,
            RuleArg::Any => FaceRule::Any,
            RuleArg::All => FaceRule::All,
        };
        let threshold = |t: Option<f32>| {
            let t = t.unwrap_or(0.5);
            if (0.0..=1.0).contains(&t) {
                Ok(t)
            } else {
                let message = "threshold must be from 0 to 1";
                Err(errors::args_invalid("/select/threshold", message))
            }
        };
        Ok(match self {
            SelectArg::Facing { angle } => {
                let Some(view) = camera else {
                    let message = "a facing selector needs a camera";
                    return Err(errors::args_invalid("/camera", message));
                };
                let angle = angle.unwrap_or(90.0);
                if !(0.0..=90.0).contains(&angle) {
                    let message = "angle must be from 0 to 90 degrees";
                    return Err(errors::args_invalid("/select/angle", message));
                }
                Selector::Facing { view, angle }
            }
            SelectArg::Faces { indexes } => {
                let found = cx.doc.get_part(part)?;
                let numbering = found.order().numbering(found.store());
                let count = numbering.face_count();
                let mut ids = Vec::with_capacity(indexes.len());
                for (i, &c) in indexes.iter().enumerate() {
                    if c as usize >= count {
                        let message = format!("the part has {count} faces");
                        return Err(errors::args_invalid(
                            &format!("/select/indexes/{i}"),
                            message,
                        ));
                    }
                    ids.push(numbering.face(c).0);
                }
                Selector::Faces(ids)
            }
            SelectArg::Masked {
                threshold: t,
                rule: r,
            } => Selector::Masked {
                threshold: threshold(*t)?,
                rule: rule(*r),
            },
            SelectArg::Unmasked {
                threshold: t,
                rule: r,
            } => Selector::Unmasked {
                threshold: threshold(*t)?,
                rule: rule(*r),
            },
            SelectArg::Groups { ids } => Selector::Groups(ids.clone()),
            SelectArg::All => Selector::All,
            SelectArg::Visible => Selector::Visible,
            SelectArg::Hidden => Selector::Hidden,
        })
    }
}

/// The view `camera` means for an image of `viewport` pixels (as `brush.stroke` and
/// `view.snapshot` take them), if given, seen from part `part`'s own space (decision 0029,
/// section 2), where its face normals are.
pub(crate) fn camera_view(
    cx: &Ctx,
    part: &str,
    camera: Option<&ViewSpec>,
    viewport: Option<[u32; 2]>,
) -> Result<Option<View>, CommandError> {
    camera
        .map(|spec| {
            let (camera, _) = aim_camera(spec, viewport, cx.doc.document(), cx.views)?;
            Ok(cx.doc.transform(part)?.part_camera(&camera).view())
        })
        .transpose()
}

/// The part's faces after the command.
#[derive(Serialize, JsonSchema)]
pub struct Visibility {
    /// The part.
    pub part: String,
    /// Faces hidden.
    pub hidden: u64,
    /// Faces visible.
    pub visible: u64,
}

impl Visibility {
    fn of(part: &str, store: &MeshStore) -> Visibility {
        let hide = store.attr_id(HIDE).map(|h| store.attr_reader(h));
        let (mut hidden, mut visible) = (0, 0);
        for n in 0..store.node_count() as NodeId {
            let node = store.node(n);
            for j in (0..node.faces.len()).filter(|&j| node.faces[j][0] != NO) {
                if hide.is_some_and(|r| r.label(node.first_face + j as u32) != 0) {
                    hidden += 1;
                } else {
                    visible += 1;
                }
            }
        }
        Visibility {
            part: part.to_string(),
            hidden,
            visible,
        }
    }
}

/// The edit `op` makes with the faces `select` picks on `store`.
fn apply(store: &mut MeshStore, op: Op, select: &Selector) -> Vec<NodeId> {
    let set = store.select(select);
    store.edit_visibility(match op {
        Op::Hide => VisibilityEdit::Hide(&set),
        Op::Show => VisibilityEdit::Show(&set),
        Op::Only => VisibilityEdit::Only(&set),
    })
}

/// What a selector command does with the faces it picks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Hide,
    Show,
    Only,
}
