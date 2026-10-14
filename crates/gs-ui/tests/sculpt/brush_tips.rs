//! The pointer on a brush's button, as a person meets it in the window (interface.tooltips):
//! the pen glides onto each kind of button that picks a brush (the shelf's family buttons, the
//! family window a held press opens, the brush picker the brush's name opens, the brush wheel
//! F2 opens, the Library's tiles and a favorite pinned from one), and on the frame it arrives,
//! with nothing waited for, the box says the keys that pick the brush first, then its name.

use gs_platform::{Input, Modifiers};
use gs_ui::Keymap;
use serde_json::{Value, json};

use crate::region::middle;
use crate::rig::{Window, pen_at};

/// A person at a window with a sphere in it, and the pen's clock in milliseconds, which runs on
/// with every sample.
pub(crate) struct Person {
    pub(crate) w: Window,
    t: u64,
}

impl Person {
    /// A window big enough that the shelf, the trays and the Library show whole.
    pub(crate) fn new() -> Self {
        let mut w = Window::sized(None, Keymap::default(), [1280, 800], 1.0);
        w.ok("primitive.sphere", json!({ "subdivisions": 16 }));
        Self { w, t: 0 }
    }

    /// One pen sample at `at`, touching with `pressure` above 0, else hovering, and the frame.
    fn sample(&mut self, [x, y]: [f64; 2], pressure: f32) {
        self.t += 16;
        self.w.pen(pen_at(self.t, x as f32, y as f32, pressure));
        self.w.input(&[], Modifiers::default());
    }

    /// The pen glides in onto `at`, hovering, a frame a sample. Returns the box's first two
    /// lines on the frame it arrives.
    pub(crate) fn glide(&mut self, at: [f64; 2]) -> Vec<String> {
        for k in [30.0, 20.0, 10.0, 0.0] {
            self.sample([at[0] - k, at[1] + k], 0.0);
        }
        self.labels("description").into_iter().take(2).collect()
    }

    /// A tap of the pen at `at`.
    pub(crate) fn tap(&mut self, at: [f64; 2]) {
        for pressure in [0.0, 0.5, 0.0] {
            self.sample(at, pressure);
        }
    }

    /// A press of the pen at `at` held a second, then lifted.
    pub(crate) fn hold(&mut self, at: [f64; 2]) {
        self.sample(at, 0.0);
        self.sample(at, 0.5);
        self.w.wait(1.0);
        self.sample(at, 0.0);
    }

    /// The key `name` pressed (`down`) or let go, and the frame.
    pub(crate) fn key(&mut self, name: &str, down: bool) {
        let modifiers = Modifiers::default();
        let key = Input::Key {
            key: name.into(),
            down,
            repeat: false,
            modifiers,
        };
        self.w.input(&[key], modifiers);
    }

    /// The middle of `name` (`panel > label`), where it shows now.
    pub(crate) fn at(&mut self, name: &str) -> [f64; 2] {
        middle(self.w.rect(name))
    }

    /// Panel `id`'s widgets, as `ui.describe` reports them.
    fn widgets(&mut self, id: &str) -> Vec<Value> {
        let d = self.w.ok("ui.describe", json!({}));
        let panels = d["panels"].as_array().cloned().unwrap_or_default();
        let panel = panels.into_iter().find(|p| p["id"] == id);
        (panel.and_then(|p| p["widgets"].as_array().cloned())).unwrap_or_default()
    }

    /// Panel `id`'s widgets' labels.
    fn labels(&mut self, id: &str) -> Vec<String> {
        (self.widgets(id).iter())
            .filter_map(|w| Some(w["label"].as_str()?.to_string()))
            .collect()
    }

    /// The middle of the shelf's widget after its label `title`: a family's button, or the
    /// first favorite.
    pub(crate) fn after(&mut self, title: &str) -> [f64; 2] {
        let widgets = self.widgets("shelf");
        let at = widgets.iter().position(|w| w["label"] == title);
        let next = at.and_then(|i| widgets.get(i + 1));
        let rect = next.unwrap_or_else(|| panic!("nothing after {title}: {widgets:?}"));
        middle(std::array::from_fn(|i| {
            rect["rect"][i].as_f64().unwrap_or_default()
        }))
    }

    /// The middle of the Pin under the Library's tile for `brush`.
    pub(crate) fn pin(&mut self, brush: &str) -> [f64; 2] {
        let widgets = self.widgets("library");
        let pin = (widgets.iter())
            .find(|w| w["label"] == "Pin" && w["command"]["args"]["pin"]["args"]["brush"] == brush);
        let pin = pin.unwrap_or_else(|| panic!("no Pin under {brush}: {widgets:?}"));
        middle(std::array::from_fn(|i| {
            pin["rect"][i].as_f64().unwrap_or_default()
        }))
    }
}

/// The shelf's family buttons, the family window and the brush picker.
fn shelf_and_picker(p: &mut Person) {
    let build = p.after("Build");
    assert_eq!(
        p.glide(build),
        ["B S T", "Draw"],
        "the shelf's Build button"
    );
    let smooth = p.after("Smooth");
    let shown = p.glide(smooth);
    assert_eq!(shown, ["B S M or Shift held", "Smooth"], "a slot's brush");
    p.hold(build);
    let clay = p.at("family > Clay Build");
    assert_eq!(p.glide(clay), ["B C B", "Clay Build"], "the family window");
    p.tap(clay);
    let name = p.at("brush > Clay Build");
    p.tap(name);
    let strong = p.at("brush > Smooth Strong");
    let shown = p.glide(strong);
    assert_eq!(shown, ["B S S", "Smooth Strong"], "the picker's brush.slot");
    let draw = p.at("brush > Draw");
    p.tap(draw);
}

/// The brush wheel, F2 held at the middle of the 3D view, and the Library's tiles and a
/// favorite pinned from one.
fn wheel_and_library(p: &mut Person) {
    let view = p.at("viewport");
    p.glide(view);
    p.key("F2", true);
    let inflate = p.at("wheel > Inflate");
    assert_eq!(p.glide(inflate), ["B I N", "Inflate"], "the brush wheel");
    p.key("F2", false);
    let library = p.at("shelf > Library");
    p.tap(library);
    let tile = p.at("library > Clay Line");
    assert_eq!(p.glide(tile), ["B C L", "Clay Line"], "a Library tile");
    let pin = p.pin("Clay Line");
    p.tap(pin);
    let favorite = p.after("Favorites");
    let shown = p.glide(favorite);
    assert_eq!(shown, ["B C L", "Clay Line"], "a favorite on the shelf");
}

#[test]
fn the_pen_onto_each_kind_of_brush_button_shows_its_keys_first() {
    let mut p = Person::new();
    shelf_and_picker(&mut p);
    wheel_and_library(&mut p);
}
