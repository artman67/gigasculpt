//! `brush.slot`: the brushes the held keys stroke with (`brush-settings.modifier-key-slots`),
//! one per slot: Smooth (Shift by default), Mask (Ctrl) and Select (Shift+Ctrl), which also
//! holds the clip and trim brushes. Any brush can sit in any slot. The main brush stays the
//! session's brush while a slot strokes, and comes back when the key is let go.
//!
//! The window says which slot its keys hold (`hold`, `release`). While one is held, `brush.set`
//! works on the slot: a brush chosen goes on the slot, and a changed setting changes the slot's
//! brush only; `session.state`'s `brush` values are the slot brush's. The slots last for the
//! session.

use gs_core::BrushPreset;
use gs_core::brush::{AttrSpec, Column, Family, KernelSpec, StrokeSpec};
use gs_protocol::{CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{BrushState, presets};
use crate::{Command, Ctx, Example, errors, register_command};

/// A held key's slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// The Smooth brushes (Shift).
    Smooth,
    /// The mask brushes (Ctrl).
    Mask,
    /// The select, hide, clip and trim brushes: those drawn as a shape (Shift+Ctrl).
    Select,
}

impl Slot {
    /// Whether `preset` belongs to this slot's family: what stepping goes through, and the slot
    /// a brush goes to when no slot is named or held.
    fn fits(self, preset: &BrushPreset) -> bool {
        let column = |c| matches!(preset.kernel, KernelSpec::Attribute(AttrSpec { column, .. }) if column == c);
        let shape = matches!(preset.stroke, StrokeSpec::Gesture { .. });
        match self {
            Slot::Smooth => preset.family == Family::Smooth,
            Slot::Mask => column(Column::Mask),
            Slot::Select => !column(Column::Mask) && (shape || column(Column::Hide)),
        }
    }

    /// The brushes of this slot's family, in `brush.list`'s order.
    fn family(self) -> impl Iterator<Item = &'static str> {
        (presets::all().iter())
            .filter(move |s| self.fits(&s.preset))
            .map(|s| s.preset.name.as_str())
    }
}

/// The brush on each slot, and the slot the window's keys hold.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Slots {
    pub smooth: String,
    pub mask: String,
    pub select: String,
    /// The slot held now; `None` while the main brush strokes.
    #[serde(default)]
    pub held: Option<Slot>,
}

impl Default for Slots {
    fn default() -> Self {
        Self {
            smooth: "Smooth".into(),
            mask: "Mask Pen".into(),
            select: "Select Rect".into(),
            held: None,
        }
    }
}

impl Slots {
    /// The brush on `slot`.
    pub fn brush(&self, slot: Slot) -> &str {
        match slot {
            Slot::Smooth => &self.smooth,
            Slot::Mask => &self.mask,
            Slot::Select => &self.select,
        }
    }

    /// Puts `brush` (a display name) on `slot`.
    pub(crate) fn put(&mut self, slot: Slot, brush: String) {
        match slot {
            Slot::Smooth => self.smooth = brush,
            Slot::Mask => self.mask = brush,
            Slot::Select => self.select = brush,
        }
    }

    /// The brush on the held slot, if one is held.
    pub(crate) fn held_brush(&self) -> Option<&str> {
        self.held.map(|s| self.brush(s))
    }

    /// The next brush of `slot`'s family after the one on it, around to the first; the first
    /// when the slot holds a brush of another family.
    fn next(&self, slot: Slot) -> Option<String> {
        let family: Vec<&str> = slot.family().collect();
        let at = family.iter().position(|b| *b == self.brush(slot));
        let next = at.map_or(0, |i| (i + 1) % family.len());
        family.get(next).map(|b| (*b).to_string())
    }
}

/// Work on the held keys' brush slots: put a brush on a slot (any brush fits any slot; with no
/// slot named, the slot held, else the one its family belongs to), step a slot to the next
/// brush of its family (`brush.list`'s order, around to the first), or hold a slot and let go
/// of it, as the window does while its key is down. While a slot is held, `brush.set` chooses
/// the slot's brush and changes its settings, and `session.state`'s brush values are the slot
/// brush's (`brush.slot.held` says which). Answers with every brush setting.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushSlot {
    /// The slot: `smooth` (Shift), `mask` (Ctrl) or `select` (Shift+Ctrl; select, hide, clip
    /// and trim brushes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
    /// Put this brush on the slot, by name or id (`brush.list`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    /// Put the next brush of the slot's family on it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub step: bool,
    /// Hold the slot, as its key does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hold: bool,
    /// Let go of the slot held: the main brush again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub release: bool,
}

impl Command for BrushSlot {
    const NAME: &'static str = "brush.slot";
    const KIND: Kind = Kind::View;
    const COST: Cost = Cost::Instant;
    type Output = BrushState;

    fn examples() -> Vec<Example<Self>> {
        let hold = json!({ "slot": "smooth", "hold": true });
        vec![
            Example::raw(
                "Shift smooths with Flat Polish",
                json!({ "slot": "smooth", "brush": "Flat Polish" }),
            ),
            Example::raw(
                "the next select or clip brush",
                json!({ "slot": "select", "step": true }),
            ),
            Example::raw("hold the Smooth slot, as Shift does", hold.clone()),
            Example::raw("let go of it", json!({ "release": true })).after("brush.slot", hold),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<BrushState, CommandError> {
        let settings = &mut cx.sculpt.settings;
        let mut slots = settings.slots.clone();
        if self.hold && self.release {
            return Err(errors::args_invalid(
                "/release",
                "give hold or release, not both",
            ));
        }
        let brush = match &self.brush {
            Some(_) if self.step => {
                return Err(errors::args_invalid(
                    "/step",
                    "give brush or step, not both",
                ));
            }
            Some(b) => Some(presets::find(b)?.preset.clone()),
            None => None,
        };
        let fitting = brush.as_ref().and_then(|p| {
            [Slot::Smooth, Slot::Mask, Slot::Select]
                .into_iter()
                .find(|s| s.fits(p))
        });
        let named = self.slot.or(slots.held).or(fitting);
        let needs = brush.is_some() || self.step || self.hold;
        let slot = match named {
            Some(slot) => slot,
            None if needs => {
                let why = "name the slot: smooth, mask or select";
                return Err(errors::args_invalid("/slot", why));
            }
            None => slots.held.unwrap_or(Slot::Smooth),
        };
        if let Some(p) = brush {
            slots.put(slot, p.name);
        }
        if self.step
            && let Some(next) = slots.next(slot)
        {
            slots.put(slot, next);
        }
        if self.hold {
            slots.held = Some(slot);
        }
        if self.release {
            slots.held = None;
        }
        settings.slots = slots;
        Ok(settings.state())
    }
}
register_command!(BrushSlot, "Set brush slot");
