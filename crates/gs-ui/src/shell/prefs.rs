//! The shell's part in preferences (decision 0052): `ui.settings`, with the settings window's
//! pages, search and Reset, and keeping `preferences.toml` as the settings change. Each setting
//! is set only by its own command: at start and on Reset the window runs it, as a press of its
//! control would.

use gs_engine::errors::{error, not_found};
use gs_engine::{PreferencesInfo, ResetScope, SettingsInfo, UiSettings};
use gs_protocol::{CommandError, ErrorCode};
use serde_json::{Value, json};

use super::Shell;
use crate::preferences::same;
use crate::record::with_args;
use crate::settings::listed::{self, Setting};

/// `startup.brush` when no brush is pinned: the brush used last starts the window.
const LAST: &str = "last";

impl Shell {
    /// `ui.settings`: opens or closes the settings window, changes the window's own settings,
    /// shows a page, searches, pins the brush at start, and asks Reset's question or answers
    /// it, after `change`. A page or setting that does not exist, or an answer with no question,
    /// is refused before anything changes. Without `change` (the session's state, after every
    /// call) the lists are left empty.
    #[expect(
        clippy::result_large_err,
        reason = "the engine's wire error, as in keymap.rs"
    )]
    pub fn settings_action(
        &mut self,
        change: Option<&UiSettings>,
    ) -> Result<SettingsInfo, CommandError> {
        let Some(change) = change else {
            return Ok(self.settings_info(PreferencesInfo::default()));
        };
        let (listed, pages, values) = {
            let now = self.now_plain();
            let pages = listed::pages(&listed::paged(&now));
            (listed::settings(&now), pages, now.state)
        };
        let page_names = || pages.iter().map(String::as_str);
        if let Some(page) = change.page.as_ref().filter(|p| !pages.contains(p)) {
            return Err(not_found("page", page, page_names()));
        }
        match &change.reset {
            Some(ResetScope::Setting(name)) if !listed.iter().any(|s| &s.name == name) => {
                let names = listed.iter().map(|s| s.name.as_str());
                return Err(not_found("setting", name, names));
            }
            Some(ResetScope::Page(page)) if !pages.contains(page) => {
                return Err(not_found("page", page, page_names()));
            }
            _ => {}
        }
        let window = &mut self.settings;
        if change.confirm.is_some() && change.reset.is_none() && window.asking.is_none() {
            let message = "there is no Reset to answer: give `reset` with `confirm`";
            return Err(error(ErrorCode::Precondition, message));
        }
        window.open = change.open.unwrap_or(window.open);
        window.page = change.page.clone().or(window.page.take());
        if let Some(search) = &change.search {
            search.clone_into(&mut window.search);
        }
        window.asking = change.reset.clone().or(window.asking.take());
        let answered = change
            .confirm
            .and_then(|yes| window.asking.take().filter(|_| yes));
        self.layout.layout.settings = change.apply(self.layout.layout.settings);
        if let Some(brush) = &change.launch_brush {
            self.prefs.put("startup.brush", &json!(brush));
        }
        // Each setting not at its default runs its own command with it.
        let mut reset = Vec::new();
        for s in listed
            .iter()
            .filter(|s| answered.as_ref().is_some_and(|a| s.reset_by(a)))
        {
            if let Some(step) = s.changed(&values) {
                self.queue(step);
                reset.push(s.name.clone());
            }
        }
        let window = &self.settings;
        let page = (window.page.clone())
            .filter(|p| pages.contains(p))
            .or_else(|| pages.first().cloned())
            .unwrap_or_default();
        let found = listed::found(&listed, &window.search);
        let prefs = PreferencesInfo {
            page,
            search: window.search.clone(),
            found: found.into_iter().map(Setting::info).collect(),
            listed: listed.iter().map(Setting::info).collect(),
            asking: (window.asking.as_ref()).map(|a| listed::question(a, &listed)),
            pages,
            reset,
            ..PreferencesInfo::default()
        };
        Ok(self.settings_info(prefs))
    }

    /// `ui.settings`' answer with `prefs`, its brush at start filled in.
    fn settings_info(&self, prefs: PreferencesInfo) -> SettingsInfo {
        let launch = self.prefs.get("startup.brush");
        let launch_brush = launch.as_ref().and_then(Value::as_str).unwrap_or(LAST);
        SettingsInfo {
            open: self.settings.open,
            settings: self.layout.layout.settings,
            prefs: PreferencesInfo {
                launch_brush: launch_brush.to_string(),
                ..prefs
            },
        }
    }

    /// Keeps `preferences.toml` as the session's settings are (decision 0052, sections 3 and
    /// 4), once a call was heard since the last time. The first time, it starts the window from
    /// the file instead: each setting the file holds at another value runs its own command with
    /// the file's, and the brush pinned, or else the one used last, is picked; what the file
    /// lacks is written from the session. After that each setting is kept as it is, and the
    /// brush in use as the one used last while no held key's slot holds another. Autosave's
    /// settings are kept in the state folder, not here. A file that is never written says why.
    pub(super) fn keep_preferences(&mut self) {
        if !std::mem::take(&mut self.prefs.heard) || self.state.settings.is_none() {
            return;
        }
        let (listed, values) = {
            let now = self.now_plain();
            (listed::settings(&now), now.state)
        };
        let start = !self.prefs.started;
        let kept = listed.iter().filter(|s| !s.name.starts_with("autosave."));
        for s in kept {
            let Some(value) = values.get(&s.name).filter(|v| !v.is_null()) else {
                continue;
            };
            match self.prefs.get(&s.name) {
                Some(file) if start && !same(&file, value) => {
                    self.outbox.extend(s.step_to(&file));
                }
                _ => self.prefs.put(&s.name, value),
            }
        }
        let brush = values.get("brush").and_then(Value::as_str);
        if start {
            let pinned = self.prefs.get("startup.brush").filter(|b| b != LAST);
            let wanted = pinned.or_else(|| self.prefs.get("startup.last"));
            let wanted = wanted.as_ref().and_then(Value::as_str);
            if let Some(name) = wanted.filter(|w| Some(*w) != brush) {
                self.queue(with_args("brush.set", json!({ "brush": name })));
            }
            self.prefs.started = true;
        } else if values.get("brush.slot.held").is_some_and(Value::is_null)
            && let Some(brush) = brush
        {
            self.prefs.put("startup.last", &json!(brush));
        }
        if let Err(problem) = self.prefs.save_if_changed() {
            self.status.says(&problem);
        }
    }
}
