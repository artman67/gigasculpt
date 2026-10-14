//! What a stroke pins before it runs, so its log line replays alone (decision 0018): the
//! session's pressure curves it strokes with, the pen's feel, Live Detail's settings, and the
//! hashes of its preset and alpha picture. Also the brush a stroke resolves to: its preset with
//! the stroke's settings. A brush retuned since a stroke was recorded does not break its log
//! (decision 0042): the log carries each preset in full on its first stroke, and a stroke
//! pinning a preset that is no longer today's sculpts with the one it was recorded with.

use std::borrow::Cow;
use std::collections::BTreeMap;

use gs_core::BrushPreset;
use gs_protocol::{CommandError, LogRecord, method};
use serde_json::Value;

use super::BrushStroke;
use crate::Unavailable;
use crate::errors;
use crate::sculpt::patch::{self, Settings};
use crate::sculpt::presets::{self, Shipped};
use crate::sculpt::{BrushSettings, detail};

/// Preset files by hash: the ones strokes carried (`Sculpting::recorded`).
type Recorded = BTreeMap<String, String>;

impl BrushStroke {
    /// The settings this stroke gives, its shortcuts folded in, and the names of the
    /// shortcuts it used.
    fn given(&self) -> Result<(Settings, Vec<&'static str>), CommandError> {
        let numbers = [
            ("strength", self.strength),
            ("falloff", self.falloff),
            ("modifier", self.modifier),
        ];
        let given: Vec<_> = (numbers.into_iter())
            .filter_map(|(name, x)| Some((name, patch::number(x?))))
            .collect();
        let settings = patch::with_shortcuts(self.settings.as_ref(), &given)?;
        Ok((settings, given.iter().map(|g| g.0).collect()))
    }

    /// The preset file this stroke sculpts with: the one it carries (`preset`), else today's
    /// shipped one. A carried file must read as a preset and be the stroke's brush.
    fn shipped(&self) -> Result<Cow<'static, Shipped>, CommandError> {
        let Some(text) = &self.preset else {
            return presets::find(&self.brush).map(Cow::Borrowed);
        };
        let preset = BrushPreset::from_toml(text).map_err(|e| {
            errors::args_invalid("/preset", format!("the preset does not read: {e}"))
        })?;
        let wanted = self.brush.trim();
        if !(preset.name.eq_ignore_ascii_case(wanted) || preset.id == wanted) {
            let message = format!("the preset is {}, the stroke's brush {wanted}", preset.name);
            return Err(errors::args_invalid("/preset", message));
        }
        let text = Cow::Owned(text.clone());
        Ok(Cow::Owned(Shipped { preset, text }))
    }

    /// The brush with this stroke's settings: what it sculpts with.
    pub(crate) fn resolve(&self) -> Result<BrushPreset, CommandError> {
        let shipped = self.shipped()?;
        let (settings, shortcuts) = self.given()?;
        patch::resolve(&shipped, &settings, &shortcuts)
    }

    /// A stroke that pins a preset other than today's, and carries none, takes the one an
    /// earlier stroke of the session carried with that hash, if any (decision 0042).
    pub(crate) fn recall(&mut self, recorded: &Recorded) {
        let Some(hash) = self.preset_hash.as_ref().filter(|_| self.preset.is_none()) else {
            return;
        };
        if today(&self.brush).as_ref() != Some(hash) {
            self.preset = recorded.get(hash).cloned();
        }
    }

    /// Keeps the preset this stroke carries for the session's later strokes, when it is not
    /// today's, and says that the stroke sculpts with it (decision 0042).
    pub(crate) fn remember(&self, recorded: &mut Recorded) -> Option<String> {
        let (text, hash) = (self.preset.as_ref()?, self.preset_hash.as_ref()?);
        let today = today(&self.brush);
        if today.as_ref() == Some(hash) {
            return None;
        }
        recorded.insert(hash.clone(), text.clone());
        let now = today.map_or_else(|| "none ships".into(), |h| format!("today's is {h}"));
        Some(format!(
            "the brush {} changed since this stroke was recorded: it sculpted with the preset \
             it was recorded with, {hash} ({now})",
            self.brush
        ))
    }

    /// Pins what the stroke depends on outside its own arguments, so its log line replays
    /// alone (decision 0018): the session's pressure curves ([`pin_pressure`]); the pen's feel
    /// as `pen` when the session's is not the default and the call gave none (decision 0036,
    /// section 3 as amended); every field of `live_detail` the call left out is filled from
    /// the brush, then the session (decision 0021); the preset's hash goes into `preset_hash`,
    /// and a hash that matches neither the brush nor a preset the stroke carries (decision
    /// 0042) is refused.
    pub(crate) fn pin_brush(&mut self, session: &BrushSettings) -> Result<(), CommandError> {
        let global = session.global_pressure();
        let preset = self.resolve()?;
        let shipped = self.shipped()?;
        self.live_detail = detail::pin(self.live_detail, &preset, &session.detail)?;
        let settings = self.settings.clone().unwrap_or_default();
        if let Some(pinned) = pin_pressure(&settings, &preset, &shipped, global) {
            self.settings = Some(pinned);
        }
        if self.pen.is_none() && !session.pen.is_default() {
            self.pen = Some(session.pen);
        }
        let now = shipped.preset.hash();
        if self.preset.is_some() && self.preset_hash.as_ref().is_some_and(|h| *h != now) {
            let message = format!("the preset given hashes to {now}, not to its preset_hash");
            return Err(errors::args_invalid("/preset", message));
        }
        let what = format!("the brush {}", shipped.preset.name);
        let hash = unchanged(self.preset_hash.as_deref(), now, &what)?;
        self.preset_hash = Some(hash);
        match &preset.tip.alpha {
            Some(spec) => {
                let picture = crate::alpha::picture(&spec.image)?;
                let what = format!("the alpha {}", spec.image);
                let hash = unchanged(self.alpha_hash.as_deref(), picture.hash().into(), &what)?;
                self.alpha_hash = Some(hash);
            }
            None if self.alpha_hash.is_some() => {
                let message = format!("{} stamps no alpha", preset.name);
                return Err(errors::args_invalid("/alpha_hash", message));
            }
            None => {}
        }
        Ok(())
    }
}

/// `settings` with the session's `global` pressure curves pinned in for `preset` (read from
/// `shipped`), or `None` when nothing is pinned: for a brush with `pressure.global`, every
/// curve `settings` leaves out, the global one or else the default; for any other brush, the
/// global size curve, unless the brush's file or `settings` name its own (decision 0036).
pub(crate) fn pin_pressure(
    settings: &Settings,
    preset: &BrushPreset,
    shipped: &Shipped,
    global: &Settings,
) -> Option<Settings> {
    let mut settings = settings.clone();
    let unset = |s: &Settings, name: &str| patch::get(s, &["pressure", name]).is_none();
    if preset.pressure.global {
        let defaults =
            serde_json::to_value(gs_core::brush::Pressure::default()).unwrap_or_default();
        for name in patch::CURVES {
            if unset(&settings, name) {
                let curve = global.get(name).or_else(|| defaults.get(name));
                let path = ["pressure", name];
                patch::put(&mut settings, &path, curve.cloned().unwrap_or_default());
            }
        }
        return Some(settings);
    }
    let size = global.get("size")?;
    if !unset(&settings, "size") || shipped.names("pressure", "size") {
        return None;
    }
    patch::put(&mut settings, &["pressure", "size"], size.clone());
    Some(settings)
}

/// The hash of today's shipped preset for `brush`, if one ships.
fn today(brush: &str) -> Option<String> {
    presets::find(brush).ok().map(|s| s.preset.hash())
}

/// `now`, the hash of `what` as it is, unless the call `expected` another: then
/// `E_PRECONDITION` naming it, as `file.import` refuses a changed file (ARCHITECTURE 8).
fn unchanged(expected: Option<&str>, now: String, what: &str) -> Result<String, CommandError> {
    match expected {
        Some(expected) if expected != now => {
            let reason = Unavailable::new(format!(
                "{what} changed: its hash is {now}, the call expected {expected}, and no \
                 stroke of this session carried that preset"
            ))
            .fix("replay a log that carries the preset in full (decision 0042), or replay with the build and library the call was made with, or leave the hash out to use it as it is now");
            Err(errors::unavailable(reason))
        }
        _ => Ok(now),
    }
}

/// `record` as the log writes it (decision 0042): each `brush.stroke` in it, or in its batch,
/// whose preset `first` has not seen in this log run carries that preset's file in full, from
/// today's brushes or the ones `recorded`. `None` when no stroke needs it. `first` says
/// whether a hash is new to the run, and remembers it.
pub(crate) fn carry_presets(
    record: &LogRecord,
    recorded: &Recorded,
    mut first: impl FnMut(&str) -> bool,
) -> Option<LogRecord> {
    const STROKE: &str = "brush.stroke";
    if record.command != STROKE && record.command != method::BATCH {
        return None;
    }
    let mut carried = record.clone();
    let strokes: Vec<&mut Value> = if carried.command == STROKE {
        vec![&mut carried.args]
    } else {
        let steps = carried
            .args
            .get_mut("commands")
            .and_then(Value::as_array_mut);
        (steps.into_iter().flatten())
            .filter(|step| step["command"] == STROKE)
            .filter_map(|step| step.get_mut("args"))
            .collect()
    };
    let mut changed = false;
    for args in strokes {
        let (Some(hash), Some(brush)) = (args["preset_hash"].as_str(), args["brush"].as_str())
        else {
            continue;
        };
        let text = match presets::find(brush) {
            Ok(s) if s.preset.hash() == hash => Some(s.text.to_string()),
            _ => recorded.get(hash).cloned(),
        };
        if let Some(text) = text.filter(|_| first(hash)) {
            args["preset"] = text.into();
            changed = true;
        }
    }
    changed.then_some(carried)
}
