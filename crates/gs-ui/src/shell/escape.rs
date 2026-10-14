//! The one owner of Escape, Enter, Tab and the arrows (decision 0025, section 5, amended
//! 2026-10-14). One press closes, cancels or answers one thing, the one in front, as a person
//! expects: before any region draws, the window finds the first thing open in [`ORDER`] and
//! tells that one. Escape's press is taken out of the frame so nothing behind it sees the key;
//! the others ([`ROUTED`]) are left for egui's own fields and buttons, and each piece that
//! answers one asks whether it was given the press ([`went_to`]). With nothing open a press is
//! left in the frame, for a key the user bound to it; a given one is not the keymap's.
//!
//! egui acts on the press before the window sees it, and that is left as it is: on Escape it
//! lets go of the text field that has the keyboard (the `Field` row), and ends its own drag of a
//! slider, which keeps the value reached (under the `Drag` row); Enter commits that field, and
//! Tab moves the keyboard on from it. Tab given to anything else moves no keyboard ([`route`]).
//!
//! [`route`]: Shell::route

use egui::{Id, Key, LayerId, Order};

use super::Shell;
use crate::wheel::Wheel;

/// What one Escape closes or cancels, or one Enter answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum To {
    /// The text field that had the keyboard, outside the palette: egui lets go of it, and a
    /// slider's typing field sets nothing, its fine handle shutting with it. Enter commits it.
    Field,
    /// The control a Ctrl+Alt press aimed at: the aim, binding nothing.
    Catch,
    /// The pen's press: a gizmo or deformer handle puts back what it moved, a spring slider's
    /// pull and a swatch's drag pick are given up, and nothing else shuts under the pen.
    Drag,
    /// A held slider key: let go, setting nothing.
    Held,
    /// A slider's fine handle.
    Fine,
    /// egui's popups: a section's menu, a popover.
    Popup,
    /// The brush family opened from the shelf.
    Family,
    /// The brush wheel.
    Wheel,
    /// The quick menu, or a slider shown alone.
    Quick,
    /// The header's menu, a view button's options or the folded menus' list.
    Menu,
    /// The recovery dialog: Cancel on Discard's question, Later on the offer. Enter: Cancel on
    /// the question, the offer's first choice.
    Recovery,
    /// The palette, with its form or its question: shut, asking nothing more. Enter: the
    /// selected row, the form's Run, the question's default.
    Palette,
    /// The adjust panel, with the pointer over it. Enter: the changed field.
    Adjust,
    /// The settings window: Cancel on Reset's question, else Close.
    Settings,
    /// The open deformer: Cancel. Enter: accept.
    Deformer,
}

/// The order, top first: the first row with something open takes the press. In a row of
/// several, the one egui draws on top goes first, so a panel opened over another shuts first.
/// Enter goes the same way; of these the field, the recovery dialog, the palette, the adjust
/// panel and the open deformer answer it, and the rest keep it from what is behind them. Tab
/// and the arrows too: the field and the palette answer them.
const ORDER: [&[To]; 9] = [
    &[To::Field],
    &[To::Catch],
    &[To::Drag],
    &[To::Held],
    &[To::Fine],
    &[To::Popup],
    &[To::Family, To::Wheel, To::Quick, To::Menu],
    &[To::Recovery, To::Palette, To::Adjust, To::Settings],
    &[To::Deformer],
];

/// The keys given as Enter is, left in the frame: Escape is taken out ([`Shell::escape`]).
const ROUTED: [Key; 6] = [
    Key::Enter,
    Key::Tab,
    Key::ArrowUp,
    Key::ArrowDown,
    Key::ArrowLeft,
    Key::ArrowRight,
];

/// Where egui's memory keeps what this frame's `key` went to.
fn given_id(key: Key) -> Id {
    Id::new(("given", key))
}

/// Where egui's memory keeps whether a text field outside the palette had the keyboard.
fn field_id() -> Id {
    Id::new("escape field")
}

/// The palette's layer.
fn palette() -> LayerId {
    LayerId::new(Order::Middle, Id::new(crate::palette::WINDOW))
}

/// What this frame's `key` went to, if it was pressed with something open.
fn went(ctx: &egui::Context, key: Key) -> Option<To> {
    ctx.data(|d| d.get_temp::<To>(given_id(key)))
}

/// Whether this frame's `key` went to `to`: the pieces that answer it ask here.
pub(crate) fn went_to(ctx: &egui::Context, key: Key, to: To) -> bool {
    went(ctx, key) == Some(to)
}

/// Whether this frame's Escape went to `to`: the pieces that cancel on it ask here.
pub(crate) fn given(ctx: &egui::Context, to: To) -> bool {
    went_to(ctx, Key::Escape, to)
}

/// Whether this frame's Enter went to `to`.
pub(crate) fn entered(ctx: &egui::Context, to: To) -> bool {
    went_to(ctx, Key::Enter, to)
}

/// Takes this frame's Enter, Tab and arrows out of `events`, the keymap's, each that went to
/// something open: a key bound to one runs only with nothing open, as one bound to Escape.
pub(super) fn unbind_given(ctx: &egui::Context, events: &mut Vec<egui::Event>) {
    for key in ROUTED {
        if went(ctx, key).is_some() {
            events.retain(|e| !pressed(e, key));
        }
    }
}

/// Ends a pass: keeps whether a text field outside the palette has the keyboard, for the next
/// frame's Escape and Enter, as egui lets go of the field before the window sees the key. A field
/// in the palette is the palette's: Escape there shuts it, as it always has.
pub(super) fn keep_field(ctx: &egui::Context) {
    let focused = ctx
        .memory(egui::Memory::focused)
        .filter(|_| ctx.text_edit_focused());
    let layer = focused
        .and_then(|id| ctx.read_response(id))
        .map(|r| r.layer_id);
    let field = layer.is_some_and(|l| l != palette());
    ctx.data_mut(|d| d.insert_temp(field_id(), field));
}

/// A press of `key`.
fn pressed(e: &egui::Event, key: Key) -> bool {
    matches!(e, egui::Event::Key { key: k, pressed: true, .. } if *k == key)
}

impl Shell {
    /// Gives this frame's Escape, if it was pressed, to the first thing open in [`ORDER`], and
    /// takes the press out of the frame. Call before anything reads the keys.
    pub(super) fn escape(&mut self, ctx: &egui::Context) {
        let Some(to) = self.give(ctx, Key::Escape) else {
            return;
        };
        ctx.input_mut(|i| i.events.retain(|e| !pressed(e, Key::Escape)));
        self.close(ctx, to);
    }

    /// Gives this frame's Enter, Tab and arrows, each if pressed, to the first thing open in
    /// [`ORDER`], for the pieces that answer them to ask ([`went_to`]); the presses stay in the
    /// frame for egui's own fields and buttons. egui moves the keyboard on Tab only from a field
    /// that took it, in the palette or not, so it never lands on a field behind the one in
    /// front. Call before anything reads the keys or draws.
    pub(super) fn route(&self, ctx: &egui::Context) {
        for key in ROUTED {
            self.give(ctx, key);
        }
        let tab = ctx.input(|i| i.key_pressed(Key::Tab));
        if tab && !matches!(went(ctx, Key::Tab), Some(To::Field | To::Palette)) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
    }

    /// Keeps what this frame's `key` goes to, if it was pressed: the first thing open in
    /// [`ORDER`]. Returns it.
    fn give(&self, ctx: &egui::Context, key: Key) -> Option<To> {
        let pressed = ctx.input(|i| i.key_pressed(key));
        let to = pressed.then(|| self.front(ctx)).flatten();
        ctx.data_mut(|d| {
            if let Some(to) = to {
                d.insert_temp(given_id(key), to);
            } else {
                d.remove::<To>(given_id(key));
            }
        });
        to
    }

    /// The first thing open in [`ORDER`].
    fn front(&self, ctx: &egui::Context) -> Option<To> {
        let layers: Vec<LayerId> = ctx.memory(|m| m.layer_ids().collect());
        // How far under the top egui drew it last frame; one not drawn then opened since, and
        // egui puts it on top.
        let depth = |to: To| {
            let layer = self.layer(to)?;
            let shown = ctx.memory(|m| m.areas().visible_last_frame(&layer));
            let at = layers.iter().rev().position(|l| *l == layer);
            at.filter(|_| shown)
        };
        ORDER.iter().find_map(|row| {
            (row.iter().copied())
                .filter(|to| self.open(ctx, *to))
                .min_by_key(|to| depth(*to).unwrap_or_default())
        })
    }

    /// The egui layer `to` draws in, for the panels a row orders by which is on top: the
    /// `Area` or `Window` id each one's file gives it and exports (the header's, by what is
    /// open in it).
    fn layer(&self, to: To) -> Option<LayerId> {
        let (order, id) = match to {
            To::Family => (Order::Foreground, crate::family::AREA),
            To::Wheel => (Order::Foreground, crate::wheel::AREA),
            To::Quick => (Order::Foreground, crate::quick::AREA),
            To::Menu => return self.menus.layer(),
            To::Recovery => (Order::Middle, crate::recovery::WINDOW),
            To::Palette => return Some(palette()),
            To::Adjust => (Order::Middle, crate::adjust::WINDOW),
            To::Settings if self.settings.asking.is_some() => {
                (Order::Foreground, crate::settings::RESET)
            }
            To::Settings => (Order::Middle, crate::settings::WINDOW),
            _ => return None,
        };
        Some(LayerId::new(order, Id::new(id)))
    }

    /// Whether `to` is open, and so can take this frame's press.
    fn open(&self, ctx: &egui::Context, to: To) -> bool {
        match to {
            To::Field => ctx.data(|d| d.get_temp::<bool>(field_id())) == Some(true),
            To::Catch => self.keys.catch.aiming(),
            To::Drag => {
                let down = ctx.input(|i| i.pointer.any_down() || i.pointer.any_released());
                down || self.gizmo.pressing()
            }
            To::Held => self.keys.holding(),
            To::Fine => crate::pieces::fine_shown(ctx),
            To::Popup => egui::Popup::is_any_open(ctx),
            To::Family => crate::family::opened(ctx).is_some(),
            To::Wheel => self.wheel.at.is_some(),
            To::Quick => self.quick.is_some(),
            To::Menu => self.menus.shows(),
            To::Recovery => self.recovery.shows(),
            To::Palette => self.palette.open,
            To::Adjust => {
                let over = ctx.pointer_latest_pos();
                let rect = ctx.memory(|m| m.area_rect(Id::new(crate::adjust::WINDOW)));
                let under = over.zip(rect).is_some_and(|(p, r)| r.contains(p));
                self.adjust.shows() && under
            }
            To::Settings => self.settings.open || self.settings.asking.is_some(),
            To::Deformer => self.state.deformer.is_some(),
        }
    }

    /// What Escape does to `to`, as each did when it read the key itself. A field, a fine
    /// handle and egui's own drag need nothing here: egui let go of the first and the last, and
    /// the slider reads the key as given (`given`), as do a spring slider and a swatch's drag.
    fn close(&mut self, ctx: &egui::Context, to: To) {
        let step = match to {
            To::Field | To::Fine => None,
            To::Catch => {
                self.keys.catch.escape();
                self.status.prompt = None;
                None
            }
            To::Drag => self.gizmo.escape(),
            To::Held => {
                self.keys.escape();
                None
            }
            To::Popup => {
                egui::Popup::close_all(ctx);
                None
            }
            To::Family => {
                crate::family::shut(ctx);
                None
            }
            To::Wheel => {
                self.wheel = Wheel::default();
                None
            }
            To::Quick => {
                self.quick = None;
                None
            }
            To::Menu => {
                self.menus = crate::header::Menus::default();
                None
            }
            To::Recovery => self.recovery.escape(),
            To::Palette => {
                self.palette.escape();
                None
            }
            To::Adjust => {
                self.adjust.escape();
                None
            }
            To::Settings => crate::settings::escaped(&self.settings),
            To::Deformer => Some(crate::pieces::spring::phase("cancel")),
        };
        if let Some(step) = step {
            self.queue(step);
        }
    }
}
