//! The Alpha palette's recently used list (catalog `alpha.palette`) and `alpha.pick`, which
//! picks from it, picks by number (the palette's index slider) and collapses it (its Restore
//! Layout).

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{find, shipped};
use crate::sculpt::BrushSet;
use crate::{Command, Ctx, Example, errors, register_command};

/// How many slots the recent list has at first, and keeps after Restore Layout.
pub const RECENT: usize = 4;

/// The alphas used lately, by library name, slot by slot: the first [`RECENT`] shipped ones at
/// first, then each alpha picked that is not in the list joins it at the front, so the list
/// grows until Restore Layout cuts it back. An alpha picked right after a slot was picked takes
/// that slot instead, as the Alpha palette of the app this one follows does.
#[derive(Clone, Debug)]
pub(crate) struct Recent {
    list: Vec<String>,
    /// The slot picked last, while the last pick was one.
    replacing: Option<usize>,
}

impl Default for Recent {
    fn default() -> Self {
        let first = shipped().iter().take(RECENT);
        Self {
            list: first.map(|s| s.name.to_string()).collect(),
            replacing: None,
        }
    }
}

impl Recent {
    /// The alphas, slot by slot.
    pub fn list(&self) -> &[String] {
        &self.list
    }

    /// The brush now stamps with `image` (a library name, or `blake3:` and a library picture's
    /// hash): it joins the list, or takes the slot picked just before.
    pub fn used(&mut self, image: &str) {
        let replacing = self.replacing.take();
        let Some(name) = named(image) else {
            return;
        };
        if self.list.iter().any(|n| n == name) {
            return;
        }
        match replacing.filter(|&i| i < self.list.len()) {
            Some(i) => name.clone_into(&mut self.list[i]),
            None => self.list.insert(0, name.to_string()),
        }
    }
}

/// The library name of the alpha `image` names, if the library has it.
fn named(image: &str) -> Option<&'static str> {
    let Some(hash) = image.strip_prefix("blake3:") else {
        return find(image).ok().map(|s| s.name);
    };
    let same = |s: &&super::Shipped| s.picture().is_ok_and(|p| p.hash() == hash);
    shipped().iter().find(same).map(|s| s.name)
}

/// Pick the alpha the brush stamps with as the Alpha palette does: by its number in
/// `alpha.list` (`index`, the palette's index slider), or from the recently used list (`slot`);
/// or put that list back to its first four slots (`restore`, the palette's Restore Layout). Any
/// alpha the brush is given, here or by `brush.set`'s `alpha`, joins the front of the list
/// unless it is in it already; one given right after a `slot` pick takes that slot instead.
/// Answers the list and the brush's alpha.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlphaPick {
    /// The alpha's number in `alpha.list`, from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// The recently used list's slot, from 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// True to put the list back to its first four slots, before any pick.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restore: bool,
}

/// The recently used alphas and the brush's alpha.
#[derive(Serialize, JsonSchema)]
pub struct Picked {
    /// By name, slot by slot.
    pub recent: Vec<String>,
    /// The alpha the brush stamps with, by name, if any.
    pub alpha: Option<String>,
}

impl Command for AlphaPick {
    const NAME: &'static str = "alpha.pick";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = Picked;

    fn examples() -> Vec<Example<Self>> {
        let pick = |index, slot, restore| Self {
            index,
            slot,
            restore,
        };
        vec![
            Example::new(
                "the second alpha alpha.list lists",
                pick(Some(1), None, false),
            ),
            Example::new(
                "the alpha in the recent list's first slot",
                pick(None, Some(0), false),
            ),
            Example::new("the recent list back to four slots", pick(None, None, true)),
            Example::new("a number past the last alpha", pick(Some(99), None, false))
                .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Picked, CommandError> {
        if self.index.is_some() && self.slot.is_some() {
            return Err(errors::args_invalid(
                "/slot",
                "give index or slot, not both",
            ));
        }
        let mut recent = cx.sculpt.settings.recent.clone();
        if self.restore {
            recent.list.truncate(RECENT);
            recent.replacing = None;
        }
        let (all, list) = (shipped(), &recent.list);
        let picked = match (self.index, self.slot) {
            (Some(i), _) => Some(nth(
                all.get(i as usize).map(|s| s.name),
                "alpha.list",
                all.len(),
            )?),
            (None, Some(i)) => {
                let name = list.get(i as usize).map(String::as_str);
                Some(nth(name, "the recent list", list.len())?)
            }
            (None, None) => None,
        };
        let alpha = picked.map(str::to_string);
        cx.sculpt.settings.recent = recent;
        if alpha.is_some() {
            let set = BrushSet {
                alpha,
                ..BrushSet::default()
            };
            set.run(cx)?;
            cx.sculpt.settings.recent.replacing = self.slot.map(|s| s as usize);
        }
        Ok(Picked {
            recent: cx.sculpt.settings.recent.list.clone(),
            alpha: cx.sculpt.settings.state().alpha,
        })
    }
}
register_command!(AlphaPick, "Pick alpha");

/// `found`, or `E_NOT_FOUND` saying `what` has `n` alphas and which numbers to give.
fn nth<'a>(found: Option<&'a str>, what: &str, n: usize) -> Result<&'a str, CommandError> {
    found.ok_or_else(|| {
        let last = n.saturating_sub(1);
        let message = format!("{what} has {n} alphas: give 0 to {last}");
        errors::error(ErrorCode::NotFound, message)
    })
}
