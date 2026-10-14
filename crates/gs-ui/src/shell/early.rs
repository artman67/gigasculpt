//! What the shell takes out of a frame's events before egui and the keys see them: a Ctrl+Alt
//! press aiming at a control and the key that binds to it (`keys::Catch`, decision 0025
//! section 5), then the wheel notches the keymap binds (`Shell::wheel_keys`), then those the
//! quick menu steps while it shows (`quick::Open::wheel`, section 2). First of all, while a
//! header menu is open, Escape and a press outside it, which only shut it (`header::early`).

use super::Shell;
use crate::quick::Turns;

impl Shell {
    /// Runs on `events`, `ppp` pixels to a point, before the frame is drawn; none of it while
    /// `typing` or with the palette open, as with every key, but what shuts a header menu.
    pub fn early(&mut self, events: &mut Vec<egui::Event>, typing: bool, ppp: f32) {
        crate::header::early(&mut self.menus, events, &self.panels, ppp);
        let keys = !typing && !self.palette.open;
        if keys {
            let (panels, controls) = (&self.panels, &self.directions.controls);
            let asked = self.keys.catch.take(events, panels, controls, ppp);
            self.outbox.extend(asked);
        }
        self.status.prompt = self.keys.catch.prompt();
        self.wheel_keys(events, typing);
        if let Some(open) = self.quick.as_mut().filter(|_| keys) {
            let turns = Turns {
                panels: &self.panels,
                pointer: self.pointer,
                ppp,
                state: &self.state,
                controls: &self.directions.controls,
            };
            self.outbox.extend(open.wheel(events, &turns));
        }
    }
}
