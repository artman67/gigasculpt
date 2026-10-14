//! The open deformer's window plumbing (decision 0041, sections 5 and 6): Enter and Escape, the
//! lift when the pen lets go, and a save the engine refuses while the pen is down, which waits
//! for the lift while the status line says so.

use egui::Key;
use gs_engine::StateValues;
use gs_protocol::{CommandError, ErrorCode, Step};

use crate::pieces::spring::phase;

/// What the open deformer asks this frame of Enter, Escape and the pen (sections 5 and 6),
/// before anything else takes the keys: while it is `held`, the lift on the first frame the pen
/// is up; and while one is open, Enter's accept or Escape's cancel, unless the pen is `down` or
/// something else takes the key first (`busy`: typing, a held slider key, the quick menu, the
/// brush wheel; or a brush family's buttons, which Escape shuts). Escape also shuts a slider's
/// fine handle or a menu first, and does only that (amended 2026-10-13 for `DF7c-notes`,
/// section 3). `held` is `None` while none is open.
pub fn keys(ui: &egui::Ui, held: Option<bool>, down: bool, busy: bool) -> Vec<Step> {
    let lift = (held == Some(true) && !down).then(|| phase("lift"));
    let busy = busy || down || crate::family::opened(ui.ctx()).is_some();
    let shuts = crate::pieces::fine_shown(ui.ctx()) || egui::Popup::is_any_open(ui.ctx());
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    let key = match (held.is_some() && !busy, enter, escape && !shuts) {
        (true, true, _) => Some(phase("accept")),
        (true, false, true) => Some(phase("cancel")),
        _ => None,
    };
    lift.into_iter().chain(key).collect()
}

/// Keeps `step`, which the engine refused with `e`, in `waiting` to send again once the pen
/// lifts, when it is busy because the pen holds the session (`gesture`): a save or export while
/// a stroke, a gizmo drag or a deformer is held (section 6). Returns whether it kept it; a
/// refusal while another waits shows as any refusal does.
pub fn held_back(
    waiting: &mut Option<Step>,
    step: Option<&Step>,
    e: &CommandError,
    gesture: bool,
) -> bool {
    let kept = step.filter(|_| gesture && e.code == ErrorCode::Busy && waiting.is_none());
    *waiting = waiting.take().or(kept.cloned());
    kept.is_some()
}

/// The step `waiting` for the lift, to send now that no gesture holds the session.
pub fn lifted(waiting: &mut Option<Step>, gesture: bool) -> Option<Step> {
    if gesture { None } else { waiting.take() }
}

/// What the status line says the window waits for, each with its hover: the step `waiting`
/// for the lift, and autosave while a deformer is open and autosave is on (section 6).
pub fn waits(
    waiting: Option<&Step>,
    state: &StateValues,
) -> Vec<(&'static str, Option<&'static str>)> {
    let words = |s: &Step| {
        if s.command.contains("save") {
            "Saves when the pen lifts"
        } else {
            "Writes the file when the pen lifts"
        }
    };
    let on = |name| state.get(name).is_some_and(|v| !v.is_null());
    let autosave = (on("deformer") && on("autosave.max_minutes")).then_some((
        "Autosave waits for the open deformer",
        Some("Accept or Cancel lets it run"),
    ));
    waiting
        .map(|s| (words(s), None))
        .into_iter()
        .chain(autosave)
        .collect()
}
