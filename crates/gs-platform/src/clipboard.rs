//! The system clipboard's text: read when a text field pastes, written when it copies or cuts.

use crate::Platform;

impl Platform {
    /// The clipboard's text, or `None` when it is empty or holds no text (SDL reads both as an
    /// empty text).
    pub fn clipboard_text(&self) -> Option<String> {
        let text = self.video().clipboard().clipboard_text().ok()?;
        (!text.is_empty()).then_some(text)
    }

    /// Puts `text` on the clipboard. SDL takes it as a C string, which a NUL would end (and the
    /// sdl3 crate panics on one), so a NUL is left out.
    pub fn set_clipboard_text(&self, text: &str) -> Result<(), String> {
        let clipboard = self.video().clipboard();
        (clipboard.set_clipboard_text(&text.replace('\0', ""))).map_err(|e| e.to_string())
    }
}
