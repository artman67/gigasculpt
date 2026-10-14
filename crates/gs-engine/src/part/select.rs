//! `part.select` (decision 0033): which part is active.

use std::collections::BTreeSet;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind, PartRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::pin_ref;
use super::visible::isolate_only;
use crate::document::{Isolate, pinned};
use crate::pinning::active_part;
use crate::{Command, Ctx, Example, errors, register_command};

/// Which way `neighbor` goes in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Neighbor {
    /// The part above the active one.
    Previous,
    /// The part below it.
    Next,
}

/// Make a part the active one: the part a command works on when none is named, which always
/// shows (decision 0033).
///
/// Name the part, or give `neighbor` to select the part above or below the active one; at
/// either end of the list that changes nothing. While Isolate is on, selecting a part it did
/// not choose isolates that part instead. The active part is session state: logged, never an
/// undo step, saved with the scene. Adding a part makes it active too, and deleting the active
/// part makes the one after it active.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartSelect {
    /// The part: `#id`, `@place` or its name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    /// Select the part above (`previous`) or below (`next`) the active one instead. It is
    /// pinned to that part's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbor: Option<Neighbor>,
}

/// What selecting did.
#[derive(Serialize, JsonSchema)]
pub struct Selected {
    /// The active part now.
    pub part: String,
    /// The part that was active before.
    pub was: Option<String>,
    /// Isolate moved to it: Isolate was on and had not chosen it.
    pub isolated: bool,
}

impl Command for PartSelect {
    const NAME: &'static str = "part.select";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = Selected;

    fn examples() -> Vec<Example<Self>> {
        let two = |e: Example<Self>| {
            e.after("primitive.sphere", json!({}))
                .after("primitive.cube", json!({}))
        };
        vec![
            two(Example::raw(
                "select the sphere",
                json!({ "part": "sphere" }),
            )),
            two(Example::raw(
                "select the part above the active one",
                json!({ "neighbor": "previous" }),
            )),
            two(Example::raw(
                "next on the last part changes nothing",
                json!({ "neighbor": "next" }),
            )),
            two(Example::raw(
                "a part that does not exist",
                json!({ "part": "head" }),
            ))
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn pin(&mut self, cx: &Ctx) -> Result<(), CommandError> {
        let way = self.neighbor.take();
        if let (None, Some(way)) = (&self.part, way) {
            self.part = Some(PartRef(neighbor(cx, way)?));
            return Ok(());
        }
        match (self.part.as_mut(), way) {
            (Some(_), Some(_)) => Err(errors::args_invalid(
                "/neighbor",
                "give part or neighbor, not both",
            )),
            (Some(r), None) => pin_ref(cx, r),
            _ => Err(errors::args_invalid(
                "/part",
                "name the part to select, or give neighbor",
            )),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<Selected, CommandError> {
        let part = pinned(self.part)?;
        let was = cx.doc.active();
        cx.doc.select(&part);
        let isolated = {
            let doc = cx.doc.document();
            let isolation = doc.isolation();
            !isolation.chosen.is_empty()
                && doc
                    .get(&part)
                    .is_some_and(|p| isolation.mark(p) != Isolate::Chosen)
        };
        if isolated {
            isolate_only(cx, &BTreeSet::from([part.clone()]));
        } else if was.as_deref() == Some(part.as_str()) {
            cx.noop_reason(format!("'{part}' is already the active part"));
        }
        Ok(Selected {
            part,
            was,
            isolated,
        })
    }
}
register_command!(PartSelect, "Select part");

/// The name of the part above or below the active one, or the active one's at that end of the
/// list.
fn neighbor(cx: &Ctx, way: Neighbor) -> Result<String, CommandError> {
    let active = active_part(&cx.doc)?;
    let list = cx.doc.list_names();
    let at = list.iter().position(|name| *name == active).unwrap_or(0);
    let to = match way {
        Neighbor::Previous => at.checked_sub(1),
        Neighbor::Next => Some(at + 1),
    };
    Ok(to.and_then(|i| list.get(i)).cloned().unwrap_or(active))
}
