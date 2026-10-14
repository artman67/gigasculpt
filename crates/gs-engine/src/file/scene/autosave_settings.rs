//! `file.autosave` (decision 0016, section 17): autosave's Max Duration, Rest Duration and
//! copies kept per scene, the settings the catalog's `interface.autosave` names. They are kept
//! in `autosave.json` in the state folder beside `recent.json` and read when a session's
//! autosave starts, so every session with autosave on shares them.

use std::path::Path;
use std::time::Duration;

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Landing, StateFolder, write_whole};
use crate::errors::{error, internal};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Max Duration's lowest and highest minutes; the highest turns it off.
const MAX_MINUTES: [f64; 2] = [1.0, 60.0];
/// Rest Duration's lowest and highest minutes; the highest turns it off.
const REST_MINUTES: [f64; 2] = [0.5, 30.0];
/// The fewest and most copies kept per scene.
const KEEP: [f64; 2] = [1.0, 100.0];

/// The settings file's name in the state folder.
const FILE: &str = "autosave.json";

/// Autosave's settings, as `file.autosave` answers them and `autosave.json` keeps them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutosaveSettings {
    /// Max Duration: minutes since the last save of any kind; 60 is off.
    pub max_minutes: f64,
    /// Rest Duration: minutes without a call from anyone; 30 is off.
    pub rest_minutes: f64,
    /// Copies kept of each scene.
    pub keep: usize,
}

impl StateFolder {
    /// Its settings; a duration past the top of its range shows as the top, off.
    pub fn settings(&self) -> AutosaveSettings {
        let minutes = |d: Duration, [_, top]: [f64; 2]| (d.as_secs_f64() / 60.0).min(top);
        AutosaveSettings {
            max_minutes: minutes(self.max_duration, MAX_MINUTES),
            rest_minutes: minutes(self.rest_duration, REST_MINUTES),
            keep: self.keep,
        }
    }

    /// The same folder with settings `s`. A duration at the top of its range never fires.
    #[must_use]
    pub fn with(self, s: AutosaveSettings) -> Self {
        let duration = |m: f64, [_, top]: [f64; 2]| {
            if m >= top {
                Duration::MAX
            } else {
                Duration::from_secs_f64(m * 60.0)
            }
        };
        Self {
            max_duration: duration(s.max_minutes, MAX_MINUTES),
            rest_duration: duration(s.rest_minutes, REST_MINUTES),
            keep: s.keep,
            ..self
        }
    }
}

/// The settings kept in state folder `state`, if its file reads as settings in range. A file
/// that does not is left for the next `file.autosave` to replace, and the defaults stand.
pub(crate) fn kept(state: &Path) -> Option<AutosaveSettings> {
    let text = std::fs::read_to_string(state.join(FILE)).ok()?;
    let s: AutosaveSettings = serde_json::from_str(&text).ok()?;
    let ranges = [
        (s.max_minutes, MAX_MINUTES),
        (s.rest_minutes, REST_MINUTES),
        (s.keep as f64, KEEP),
    ];
    let fits = |(v, [low, high]): (f64, [f64; 2])| (low..=high).contains(&v);
    ranges.into_iter().all(fits).then_some(s)
}

/// Show or change autosave's settings (catalog `interface.autosave`): Max Duration, the minutes
/// since the last save of any kind after which it saves (1 to 60, default 10; 60 turns this
/// trigger off); Rest Duration, the minutes without a call from anyone after which it saves
/// (0.5 to 30, default 2; 30 turns it off); and the copies kept of each scene (1 to 100,
/// default 10; a fraction is rounded). Left out, a setting stays. Kept in `autosave.json` in the
/// state folder, so every session with autosave on starts with them; a session with autosave
/// off has none and is refused. Answers every setting. Not logged: a user's setting, not the
/// scene's.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileAutosave {
    /// Minutes since the last save, 1 to 60 (60: off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 60.0))]
    pub max_minutes: Option<f64>,
    /// Minutes without a call, 0.5 to 30 (30: off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 0.5, max = 30.0))]
    pub rest_minutes: Option<f64>,
    /// Copies kept of each scene, 1 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1.0, max = 100.0))]
    pub keep: Option<f64>,
}

impl Command for FileAutosave {
    const NAME: &'static str = "file.autosave";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    /// The settings land on the engine thread when the call commits, as a save's file does.
    const BACKGROUND: bool = true;
    type Output = AutosaveSettings;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "keep three copies of each scene (refused here: autosave is off)",
                Self {
                    keep: Some(3.0),
                    ..Self::default()
                },
            )
            .fails_with(ErrorCode::Precondition),
            Example::raw(
                "a Rest Duration under half a minute",
                json!({ "rest_minutes": 0.1 }),
            )
            .fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn check(&self, cx: &Ctx) -> Result<(), Unavailable> {
        match cx.config.autosave {
            Some(_) => Ok(()),
            None => Err(Unavailable::new("autosave is off in this session")
                .fix("the window has it; gigasculpt serve has it with --autosave")),
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<AutosaveSettings, CommandError> {
        let config = (cx.config.autosave.clone()).ok_or_else(|| internal("autosave is off"))?;
        let now = config.settings();
        let s = AutosaveSettings {
            max_minutes: self.max_minutes.unwrap_or(now.max_minutes),
            rest_minutes: self.rest_minutes.unwrap_or(now.rest_minutes),
            keep: self.keep.map_or(now.keep, |k| k.round() as usize),
        };
        if s != now {
            let text = serde_json::to_string_pretty(&s).map_err(|e| internal(&e.to_string()))?;
            write_whole(&config.state_dir, FILE, &text).map_err(|e| {
                let file = config.state_dir.join(FILE);
                let message = format!("cannot write {}: {e}", file.display());
                error(ErrorCode::Precondition, message)
            })?;
            cx.land(Landing::Autosave(config.with(s)));
        }
        Ok(s)
    }
}
register_command!(FileAutosave, "Autosave settings");
