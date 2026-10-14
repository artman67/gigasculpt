//! `part.set_visible` and `part.isolate`: the parts' eyes and Isolate (decision 0028, section
//! 2). A part that does not show is not drawn, picked or framed, and commands that read the
//! visible parts skip it; a command that names it still works on it.

use std::collections::BTreeSet;

use gs_protocol::{CommandError, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{names, pin_refs, shown};
use crate::document::{Isolate, Isolation, Part, Props, Showing};
use crate::{Command, Ctx, Example, register_command};

/// Turn the eyes of parts on or off.
///
/// "Show every part" is `{"visible": true}`; "hide every part but these" is `{"parts": [...],
/// "visible": false, "others": true}`. While Isolate is on it decides what shows; the eyes set
/// here show once it ends.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartSetVisible {
    /// The parts. Left out or empty: every part.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartRef>,
    /// Eyes on (true) or off.
    pub visible: bool,
    /// Set the eyes of every part but `parts` instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub others: bool,
}

/// What showing or hiding parts did.
#[derive(Serialize, JsonSchema)]
pub struct Shown {
    /// The parts whose eye or Isolate mark changed, in list order.
    pub changed: Vec<String>,
    /// The parts that show now, in list order.
    pub shown: Vec<String>,
}

impl Command for PartSetVisible {
    const NAME: &'static str = "part.set_visible";
    const KIND: Kind = Kind::Edit;
    const COST: gs_protocol::Cost = gs_protocol::Cost::Instant;
    type Output = Shown;

    fn examples() -> Vec<Example<Self>> {
        let two = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
        };
        vec![
            two(Example::raw(
                "hide the cube",
                json!({ "parts": ["cube"], "visible": false }),
            )),
            two(Example::raw(
                "hide every part but the cube",
                json!({ "parts": ["cube"], "visible": false, "others": true }),
            )),
            two(Example::raw(
                "show every part again",
                json!({ "visible": true }),
            ))
            .after("part.set_visible", json!({ "visible": false })),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_refs(cx, &mut self.parts)
    }

    fn run(self, cx: &mut Ctx) -> Result<Shown, CommandError> {
        let named = names(&self.parts);
        let every = self.parts.is_empty();
        let targets: Vec<String> = (cx.doc.list_names().into_iter())
            .filter(|name| every || named.contains(name) != self.others)
            .collect();
        let hidden = !self.visible;
        let changed = change(cx, &targets, |_, props| props.hidden = hidden)?;
        if changed.is_empty() {
            let eye = if self.visible { "on" } else { "off" };
            cx.noop_reason(format!(
                "the eye of every one of those parts is already {eye}"
            ));
        }
        let shown = shown(cx);
        Ok(Shown { changed, shown })
    }
}
register_command!(PartSetVisible, "Set part visibility");

/// Show only some parts, or end Isolate.
///
/// The other parts are set aside, their eyes left alone, so ending Isolate shows exactly what
/// showed before. Isolating the parts already isolated, or no parts, ends it. A part made while
/// Isolate is on shows if its eye is on; a duplicate takes its source's mark. The active part
/// always shows, so isolating parts that leave it out makes the first of them active, and
/// selecting a part Isolate did not choose isolates it instead (decision 0033). Isolate is
/// session state (decision 0029, section 9): not an undo step, saved with the scene.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartIsolate {
    /// The parts to show alone. Left out or empty: end Isolate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartRef>,
}

impl Command for PartIsolate {
    const NAME: &'static str = "part.isolate";
    const KIND: Kind = Kind::View;
    const COST: gs_protocol::Cost = gs_protocol::Cost::Instant;
    type Output = Shown;

    fn examples() -> Vec<Example<Self>> {
        let two = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
        };
        vec![
            two(Example::raw(
                "show the sphere alone",
                json!({ "parts": ["sphere"] }),
            )),
            two(Example::raw("end Isolate", json!({})))
                .after("part.isolate", json!({ "parts": ["cube"] })),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        pin_refs(cx, &mut self.parts)
    }

    fn run(self, cx: &mut Ctx) -> Result<Shown, CommandError> {
        let doc = cx.doc.document();
        let isolation = doc.isolation();
        let chosen = doc.list().into_iter();
        let chosen = chosen.filter(|(_, part)| isolation.mark(part) == Isolate::Chosen);
        let chosen: BTreeSet<String> = chosen.map(|(name, _)| name.to_string()).collect();
        let asked: BTreeSet<String> = names(&self.parts).into_iter().collect();
        let asked = if asked == chosen {
            BTreeSet::new()
        } else {
            asked
        };
        let changed = isolate_only(cx, &asked);
        if changed.is_empty() {
            cx.noop_reason("Isolate is off");
        }
        let first = cx.doc.list_names().into_iter().find(|n| asked.contains(n));
        if let Some(first) = first
            && !cx.doc.active().is_some_and(|a| asked.contains(&a))
        {
            cx.doc.select(&first);
        }
        let shown = shown(cx);
        Ok(Shown { changed, shown })
    }
}
register_command!(PartIsolate, "Isolate");

/// Whether the 3D view draws `part` now (decision 0028, section 10): what shows, except that
/// while `hiding` (Dynamic isolate on and the camera moving) with Isolate off, only the active
/// part is drawn. Nothing about the part changes, so it is drawn again once the camera stops.
pub(crate) fn drawn(part: &Part, showing: Showing, hiding: bool) -> bool {
    let alone = hiding && showing.isolation.chosen.is_empty() && showing.active.is_some();
    if alone {
        part.props().id == showing.active
    } else {
        part.shown(showing)
    }
}

/// Has Isolate show the parts `asked` alone, or ends it when `asked` is empty, and returns the
/// parts whose mark changed, in list order.
pub(super) fn isolate_only(cx: &mut Ctx, asked: &BTreeSet<String>) -> Vec<String> {
    let (next, changed) = {
        let doc = cx.doc.document();
        let (list, isolation) = (doc.list(), doc.isolation());
        let mut next = Isolation::default();
        for (name, part) in list.iter().filter(|_| !asked.is_empty()) {
            let set = if asked.contains(*name) {
                &mut next.chosen
            } else {
                &mut next.aside
            };
            set.extend(part.props().id);
        }
        let changed: Vec<String> = (list.iter())
            .filter(|(_, part)| next.mark(part) != isolation.mark(part))
            .map(|(name, _)| name.to_string())
            .collect();
        (next, changed)
    };
    cx.doc.isolate(|isolation| *isolation = next);
    changed
}

/// Changes the properties of each part of `targets` by `f`, and returns the names of those it
/// changed, in the order given.
pub(super) fn change(
    cx: &mut Ctx,
    targets: &[String],
    f: impl Fn(&str, &mut Props),
) -> Result<Vec<String>, CommandError> {
    let mut changed = Vec::new();
    for name in targets {
        let before = cx.doc.get_part(name)?.props();
        let mut props = before;
        f(name, &mut props);
        if props != before {
            cx.doc.set_props(name, props)?;
            changed.push(name.clone());
        }
    }
    Ok(changed)
}
