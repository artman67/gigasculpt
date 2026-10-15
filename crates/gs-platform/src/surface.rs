//! A stand-in for the window that a GPU surface can hold.
//!
//! wgpu wants the window as a `Send + Sync` value, and SDL's window type is neither, because SDL
//! windows may only be used on the thread that made them. Rather than claim otherwise with
//! `unsafe`, the window is kept for the life of the process in a thread-local, and
//! [`SurfaceHandle`] (which holds nothing) looks it up there. On any other thread it answers
//! "unavailable", so no handle ever leaves the window's thread.

use std::cell::Cell;

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use sdl3::video::Window;

thread_local! {
    static WINDOW: Cell<Option<&'static Window>> = const { Cell::new(None) };
}

/// Keeps `window` until the process ends and makes it the one [`SurfaceHandle`] answers for on
/// this thread.
pub(crate) fn keep(window: Window) -> &'static Window {
    let window: &'static Window = Box::leak(Box::new(window));
    WINDOW.with(|w| w.set(Some(window)));
    window
}

/// The window, if this thread has one and its video driver is the system's own, with native
/// handles: `cocoa` on macOS, `windows` on Windows, `wayland` or `x11` on Linux. On the others,
/// such as `offscreen`, the `sdl3` crate panics or reads a handle the window does not have.
fn window() -> Result<&'static Window, HandleError> {
    let window = WINDOW.with(Cell::get).ok_or(HandleError::Unavailable)?;
    match window.subsystem().current_video_driver() {
        "cocoa" | "windows" | "wayland" | "x11" => Ok(window),
        _ => Err(HandleError::NotSupported),
    }
}

/// The window's display and window handles, for creating a GPU surface on the window's thread.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceHandle(());

impl SurfaceHandle {
    pub(crate) fn new() -> Self {
        Self(())
    }
}

impl HasWindowHandle for SurfaceHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        window()?.window_handle()
    }
}

impl HasDisplayHandle for SurfaceHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        window()?.display_handle()
    }
}
