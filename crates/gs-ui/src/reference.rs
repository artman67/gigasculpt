//! The Reference Views in the window (decisions 0020, 0024): where the layout places them (a
//! floating window, a docked panel, or half the 3D view), one bar in the header's style (`bar`,
//! for the active reference) and a picture of every open reference under it, the area split
//! along its longer side. The studio draws each picture through the renderer into a frame of
//! its own and the interface shows that frame as a texture ([`picture`]). Drags over a picture
//! run the 3D view's gesture rows (`navigate`), each sent to the view that owns the move: with
//! rotation linked, a turn over the reference turns main, and both turn.

mod bar;

use std::collections::BTreeMap;

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, TextureId};
use gs_engine::{Hand, Link, MainView, Navigation, ReferenceState};
use gs_protocol::Step;
use serde_json::Value;

use crate::direction::{Edge, RefPlace, ReferenceSpec};
use crate::navigate::{Context, Navigator};
use crate::pieces::Now;
use crate::record::{Recorder, Role, Widget};

/// The interface texture that is Reference View `view`'s picture, `TextureId::User(n)` for
/// `ref:<n>`: a user texture, which egui never makes or frees itself.
pub fn picture(view: &str) -> TextureId {
    TextureId::User(number(view))
}

/// The number of `ref:<n>`; 0 for any other name.
fn number(view: &str) -> u64 {
    (view.strip_prefix("ref:"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Gestures over the references are numbered from here, apart from the 3D view's; each
/// reference's apart from the others' (`ref:<n>` from here plus n times 2^32).
const GESTURES: u64 = 1 << 40;

/// The view name a control gives to mean a reference the window picks when it runs: the one
/// whose picture is under the pointer, else the active one ([`RefPanel::resolve`]).
const PICKED: &str = "reference";

/// The view name a key gives to mean the view under the pointer when it is pressed: a
/// reference's picture, else `main` (decision 0025, section 5).
pub(crate) const POINTED: &str = "pointed";

/// Points between two pictures.
const GAP: f32 = 4.0;

/// A Reference View the window shows, as the engine last reported it.
#[derive(Clone, Debug)]
pub struct Shown {
    /// Its name (for example `ref:1`), links and look.
    pub setup: ReferenceState,
    /// Its camera, lock and pivot.
    pub camera: MainView,
    /// It shows the scene's own parts, live; else parts of its own.
    pub live: bool,
    /// It is the active one: the header's controls act on it.
    pub active: bool,
}

/// The Reference Views' state between frames.
#[derive(Debug, Default)]
pub struct RefPanel {
    /// Every open reference, by number; empty while none is open.
    pub shown: Vec<Shown>,
    /// Each picture's area in the last frame, in points, by view name; empty while none shows.
    pub pictures: Vec<(String, Rect)>,
    /// Each picture's gesture map, by view name. Kept for the window's life, so a reference
    /// opened again under a name never hands the engine a gesture id it has seen.
    navs: BTreeMap<String, Navigator>,
}

/// What drawing the reference reads besides the pieces' `Now`.
pub struct Sight<'a> {
    pub spec: &'a ReferenceSpec,
    pub settings: &'a Navigation,
    pub hand: Hand,
}

impl RefPanel {
    /// The active reference, if one is open.
    pub fn active(&self) -> Option<&Shown> {
        self.shown.iter().find(|s| s.active)
    }

    /// Names the view in every step of `steps` whose `view` says the window picks it. For
    /// `reference` (a control that acts on whichever reference the window points at): the one
    /// whose picture is under `pointer` (a key bound to such a step, pressed over it), else the
    /// active one. For `pointed` (a view key): the reference under `pointer`, else `main`.
    pub fn resolve(&self, steps: &mut [Step], pointer: Option<Pos2>) {
        let under = pointer.and_then(|at| self.pictures.iter().find(|(_, r)| r.contains(at)));
        let under = under.map(|(view, _)| view.as_str());
        let active = self.active().map(|s| s.setup.view.as_str());
        for step in steps {
            let Some(view) = step.args.get_mut("view") else {
                continue;
            };
            let picked = match view.as_str() {
                Some(PICKED) => under.or(active),
                Some(POINTED) => Some(under.unwrap_or("main")),
                _ => None,
            };
            if let Some(picked) = picked {
                *view = Value::from(picked);
            }
        }
    }

    /// Draws the references docked on an edge, or as half the 3D view, as `sight` places it;
    /// nothing for a floating one (see [`RefPanel::floating`]) or while none is open. Call
    /// before the 3D view takes what is left. Returns what it asked to run.
    pub fn docked(
        &mut self,
        ui: &mut egui::Ui,
        sight: &Sight,
        now: &Now,
        rec: &mut Recorder,
    ) -> Vec<Step> {
        self.pictures.clear();
        let (edge, split) = match sight.spec.place {
            RefPlace::Float => return Vec::new(),
            RefPlace::Left => (Edge::Left, false),
            RefPlace::Right => (Edge::Right, false),
            RefPlace::Split => (Edge::Right, true),
        };
        if self.shown.is_empty() {
            return Vec::new();
        }
        let id = egui::Id::new("reference panel");
        let side = match edge.for_hand(sight.hand) {
            Edge::Left => egui::Panel::left(id),
            _ => egui::Panel::right(id),
        };
        let side = if split {
            side.exact_size(ui.available_width() / 2.0).resizable(false)
        } else {
            let most = (ui.available_width() - sight.spec.width).max(sight.spec.width);
            (side.default_size(sight.spec.width))
                .size_range(160.0..=most)
                .resizable(true)
        };
        let shown = side.show(ui, |ui| {
            rec.panel("reference", "Reference", ui.max_rect());
            self.contents(ui, sight.settings, now, rec)
        });
        shown.inner
    }

    /// Draws the references as a window over the 3D view `view` (in points), if the layout
    /// floats them and one is open: at least as wide as its bar on one row, and as the layout
    /// asks, as far as the view has room. Call after the 3D view, so it lies on top. Returns
    /// what it asked to run.
    pub fn floating(
        &mut self,
        ui: &mut egui::Ui,
        (sight, view): (&Sight, Rect),
        now: &Now,
        rec: &mut Recorder,
    ) -> Vec<Step> {
        if sight.spec.place != RefPlace::Float || self.shown.is_empty() || !view.is_positive() {
            return Vec::new();
        }
        let size = egui::vec2(sight.spec.width, sight.spec.height);
        // A window's least size is its outside's, its frame included.
        let frame = egui::Frame::window(ui.style()).total_margin().sum().x;
        let least = (bar::width(ui, now) + frame).max(200.0).min(view.width());
        let window = egui::Window::new("Reference")
            .id(egui::Id::new("reference window"))
            .default_pos(view.min + egui::vec2(12.0, 12.0))
            .default_size(size)
            .min_size(egui::vec2(least, 200.0))
            .constrain_to(view)
            .collapsible(false)
            .resizable(true)
            .scroll(false);
        let shown = window.show(ui.ctx(), |ui| {
            rec.panel("reference", "Reference", ui.max_rect());
            self.contents(ui, sight.settings, now, rec)
        });
        let Some(shown) = shown else {
            return Vec::new();
        };
        rec.fit(shown.response.rect);
        shown.inner.unwrap_or_default()
    }

    /// The bar (`bar`, for the active reference) and every reference's picture under it, the
    /// area split along its longer side, the active one outlined while several show, with
    /// navigation over each picture.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        settings: &Navigation,
        now: &Now,
        rec: &mut Recorder,
    ) -> Vec<Step> {
        let mut asked = bar::bar(ui, now, rec);
        let area = ui.available_rect_before_wrap();
        if !area.is_positive() {
            return asked;
        }
        let several = self.shown.len() > 1;
        let outline = Stroke::new(2.0, ui.visuals().selection.bg_fill);
        let full = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        for (shown, rect) in self.shown.iter().zip(split(area, self.shown.len())) {
            let view = shown.setup.view.as_str();
            self.pictures.push((view.to_string(), rect));
            let response = ui.allocate_rect(rect, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.image(picture(view), rect, full, Color32::WHITE);
            if several && shown.active {
                painter.rect_stroke(rect, 0.0, outline, StrokeKind::Inside);
            }
            rec.widget(Widget::plain(view, Role::Label), rect);
            let cx = Context {
                rect,
                ppp: ui.ctx().pixels_per_point(),
                main: &shown.camera,
                settings,
                // Nothing to sculpt here: a tip drag navigates, as on empty canvas.
                canvas: true,
                figure: None,
                pads: Vec::new(),
            };
            let first = GESTURES + (number(view).min(0xFF_FFFF) << 32);
            let nav = (self.navs.entry(view.to_string()))
                .or_insert_with(|| Navigator::numbered_from(first));
            let steps = nav.frame(ui, &response, &cx);
            asked.extend(route(steps, view, shown.setup.link));
        }
        asked
    }
}

/// `area` cut into `count` equal pictures side by side along its longer side, `GAP` apart.
fn split(area: Rect, count: usize) -> Vec<Rect> {
    let n = count.max(1) as f32;
    let across = area.width() >= area.height();
    let long = if across { area.width() } else { area.height() };
    let each = ((long - GAP * (n - 1.0)) / n).max(1.0);
    (0..count)
        .map(|i| {
            let at = (each + GAP) * i as f32;
            if across {
                let min = area.min + egui::vec2(at, 0.0);
                Rect::from_min_size(min, egui::vec2(each, area.height()))
            } else {
                let min = area.min + egui::vec2(0.0, at);
                Rect::from_min_size(min, egui::vec2(area.width(), each))
            }
        })
        .collect()
}

/// Each move sent to the view that owns it: a linked one to main (without the pixels it
/// started at, which are the reference's), the rest to the reference.
fn route(steps: Vec<Step>, view: &str, link: Link) -> Vec<Step> {
    (steps.into_iter())
        .map(|mut step| {
            let to_main = match step.command.as_str() {
                "view.orbit" | "view.roll" => link.rotation,
                "view.pan" => link.pan,
                "view.dolly" => link.zoom,
                _ => false,
            };
            if let Some(args) = step.args.as_object_mut() {
                if to_main {
                    args.remove("at");
                    args.remove("toward");
                } else {
                    args.insert("view".into(), view.into());
                }
            }
            step
        })
        .collect()
}
