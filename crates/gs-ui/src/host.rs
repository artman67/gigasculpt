//! The window as the engine sees it: a [`WindowHost`] over the shared [`Studio`].
//!
//! The engine may run on its own thread. It only ever waits for the studio's lock, never for
//! the window's thread, and the window never holds the lock while it waits for the engine, so
//! the two cannot wait for each other.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use gs_core::gizmo::{Pose, Style};
use gs_engine::{
    AdjustInfo, CallReport, DeformSettings, GizmoSettings, HeldCurve, KeymapInfo, LayoutInfo,
    LibraryInfo, MenuInfo, Navigation, PaletteInfo, PenReads, PenStroke, PenTaken, QuickInfo,
    Screenshot, SettingsInfo, UiAdjust, UiCurve, UiDeform, UiDescription, UiGizmo, UiKeymap,
    UiLayout, UiLibrary, UiMenu, UiNavigation, UiPalette, UiQuick, UiSettings, UiWheel, WheelInfo,
    WindowHost,
};
use gs_protocol::{CommandError, PenSample, Step};

use crate::shell::Shell;
use crate::studio::Studio;

/// A shared handle on the window's state.
#[derive(Clone)]
pub struct Host(pub Arc<Mutex<Studio>>);

impl Host {
    pub fn new(studio: Studio) -> Self {
        Self(Arc::new(Mutex::new(studio)))
    }

    /// The studio, locked. A panic while it was locked does not lock the window out.
    pub fn lock(&self) -> MutexGuard<'_, Studio> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `change` on the shell, and marks the window for a new frame.
    fn shell<R>(&self, change: impl FnOnce(&mut Shell) -> R) -> R {
        let mut studio = self.lock();
        studio.dirty = true;
        change(&mut studio.shell)
    }

    /// A window setting (`ui.navigation`, `ui.gizmo`): `answer` reads it from the shell after
    /// `change`, and a change marks the window for a new frame.
    fn setting<C, R>(&self, change: Option<&C>, answer: fn(&mut Shell, Option<&C>) -> R) -> R {
        let mut studio = self.lock();
        studio.dirty |= change.is_some();
        answer(&mut studio.shell, change)
    }
}

impl WindowHost for Host {
    fn has_window(&self) -> bool {
        true
    }

    fn pen_input(&mut self, samples: &[PenSample], reads: &PenReads) -> PenTaken {
        self.lock().pen(samples, reads)
    }

    fn describe(&mut self) -> UiDescription {
        self.lock().describe()
    }

    fn screenshot(&mut self) -> Option<Result<Screenshot, String>> {
        Some(self.lock().screenshot())
    }

    fn palette(&mut self, request: &UiPalette) -> Option<Result<PaletteInfo, String>> {
        Some(self.shell(|s| s.palette_request(request)))
    }

    fn layout(&mut self, change: &UiLayout) -> Option<Result<LayoutInfo, String>> {
        Some(self.shell(|s| s.change_layout(change)))
    }

    fn wheel(&mut self, change: &UiWheel) -> Option<Result<WheelInfo, String>> {
        Some(self.shell(|s| s.wheel_action(change)))
    }

    fn library(&mut self, change: &UiLibrary) -> Option<Result<LibraryInfo, String>> {
        Some(self.shell(|s| s.library_action(change)))
    }

    #[expect(
        clippy::result_large_err,
        reason = "the engine's wire error, as in keymap.rs"
    )]
    fn keymap(&mut self, change: &UiKeymap) -> Option<Result<KeymapInfo, CommandError>> {
        Some(self.shell(|s| s.keymap.change(change, s.keymap_file.as_deref())))
    }

    fn settings(
        &mut self,
        change: Option<&UiSettings>,
    ) -> Option<Result<SettingsInfo, CommandError>> {
        Some(self.setting(change, Shell::settings_action))
    }

    fn quick(&mut self, change: &UiQuick) -> Option<Result<QuickInfo, String>> {
        Some(self.shell(|s| s.quick_action(change)))
    }

    fn menu(&mut self, change: &UiMenu) -> Option<Result<MenuInfo, String>> {
        Some(self.shell(|s| s.menu_action(change)))
    }

    fn adjust(&mut self, change: &UiAdjust) -> Option<AdjustInfo> {
        Some(self.shell(|s| s.adjust_action(change)))
    }

    fn shown(&mut self) -> Option<LayoutInfo> {
        self.lock().shell.shown_layout()
    }

    fn viewport(&mut self) -> Option<[u32; 2]> {
        self.lock().viewport_size()
    }

    fn reference_sizes(&mut self) -> Vec<(String, [u32; 2])> {
        self.lock().reference_sizes()
    }

    fn navigation(&mut self, change: Option<&UiNavigation>) -> Option<Navigation> {
        Some(self.setting(change, Shell::navigation))
    }

    fn gizmo(&mut self, change: Option<&UiGizmo>) -> Option<GizmoSettings> {
        Some(self.setting(change, Shell::gizmo_settings))
    }

    fn curve(&mut self, change: &UiCurve) -> Option<HeldCurve> {
        Some(self.setting(Some(change), Shell::curve_held))
    }

    fn deform(&mut self, change: &UiDeform) -> Option<DeformSettings> {
        let mut studio = self.lock();
        studio.dirty = true;
        Some(crate::pieces::deform::settings(&studio.ctx, change))
    }

    fn gizmo_style(&mut self) -> Style {
        self.lock().gizmo_style()
    }

    fn show_gizmo(&mut self, pose: &dyn Fn(&str) -> Option<Pose>) {
        let mut studio = self.lock();
        let shell = &mut studio.shell;
        if shell.gizmo.show(&shell.parts, pose) {
            studio.dirty = true;
        }
    }

    fn controls(&mut self) -> Vec<Step> {
        self.lock().shell.controls()
    }

    fn after_call(&mut self, report: &CallReport) {
        let mut studio = self.lock();
        crate::pieces::deform::heard(&studio.ctx, &report.parts);
        studio.after_call(report);
    }

    fn take_strokes(&mut self) -> Vec<PenStroke> {
        self.lock().take_strokes()
    }
}
