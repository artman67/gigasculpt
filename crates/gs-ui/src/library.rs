//! The Library (decision 0025, section 13): a drawer along the bottom of the 3D view, between
//! the trays, showing one of the layout's tabs at a time (`library` in the layout file) as a
//! row of tiles scrolled sideways. A tile is its piece's control drawn as a picture
//! (`thumbs`), or as text while it has none: a tap runs its command, and it is lit and
//! disabled exactly as the control would be, its label under it, then its Pin toggle, which
//! makes it a favorite on the shelf (`favorites`). `ui.library` opens and shuts the drawer,
//! picks the tab and pins and unpins, so an agent can do all a hand can; the tab buttons, Close
//! and Pin run it too.
//!
//! The materials' tab (decision 0043, step (d)) has two toggles beside Close: Show Used, which
//! keeps the materials the scene uses, and Preview: while it is on, a pen resting `REST` on a
//! material's tile draws the 3D view in that material until it moves off (`previewing`), and
//! runs and logs nothing.

mod favorites;
pub(crate) mod thumbs;

use std::time::Duration;

use egui::{Align, Color32, Layout, RichText};
use gs_engine::{LibraryInfo, UiLibrary};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::direction::{Control, Direction, LibraryTab};
use crate::icons;
use crate::pieces::{
    Face, Now, Press, controls_of, hover, press_of, pressed_as, run_as, run_button,
};
use crate::record::{Recorder, with_args};
use crate::theme::{Theme, theme};
pub use favorites::favorites;
use thumbs::{Source, picture};

/// A tile's picture, in points across.
pub const TILE: f32 = 88.0;

/// How long the pen rests on a material's tile before the view previews it, in seconds.
const REST: f64 = 0.4;

/// The Library's state between frames; the favorites are the layout's, saved with it.
pub struct Library {
    pub open: bool,
    /// The tab shown, by id; one the layout lacks shows its first.
    pub tab: String,
    /// The materials' tab shows only the materials the scene uses.
    pub used: bool,
    /// A pen resting on a material's tile previews it.
    pub preview: bool,
    /// The material whose tile the pen is on, and since when (egui's time).
    hovered: Option<(String, f64)>,
    /// The material the 3D view draws in for now, the pen having rested on its tile.
    pub(crate) previewing: Option<String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            open: false,
            tab: String::new(),
            used: false,
            preview: true,
            hovered: None,
            previewing: None,
        }
    }
}

/// `ui.library` running `args`.
fn step(args: serde_json::Value) -> Step {
    with_args("ui.library", args)
}

/// `s` with no arguments given as none: `{}`, as a control's own step has them.
fn plain(s: &Step) -> Step {
    let args = if s.args.is_null() {
        json!({})
    } else {
        s.args.clone()
    };
    with_args(&s.command, args)
}

/// `s` as messages name it.
fn named(s: &Step) -> String {
    format!("`{} {}`", s.command, s.args)
}

/// The controls `d`'s Library tabs show as tiles, tab by tab.
pub fn tiles(d: &Direction, now: &Now) -> Vec<Control> {
    (d.library.iter())
        .flat_map(|t| controls_of(&t.show, now))
        .collect()
}

impl Library {
    /// The tab `d`'s Library shows: the one picked, or its first.
    fn shown<'a>(&self, d: &'a Direction) -> Option<&'a LibraryTab> {
        (d.library.iter().find(|t| t.id == self.tab)).or(d.library.first())
    }

    /// `ui.library` in the layout `d`, whose tiles run `tiles`: opens or shuts the drawer and
    /// shows a tab (which opens it unless `open` is false), and pins a tile to `favorites` or
    /// unpins one. All or nothing: a tab the layout lacks, a pin no tile runs or an unpin of
    /// what is not a favorite changes nothing.
    pub fn change(
        &mut self,
        change: &UiLibrary,
        d: Option<&Direction>,
        tiles: &[Step],
        favorites: &mut Vec<Step>,
    ) -> Result<LibraryInfo, String> {
        let tabs: Vec<&str> = d.map_or(Vec::new(), |d| {
            d.library.iter().map(|t| t.id.as_str()).collect()
        });
        let opens = change.open.unwrap_or(change.tab.is_some());
        if tabs.is_empty() && (opens || change.tab.is_some()) {
            return Err("the layout shown has no Library".into());
        }
        if let Some(tab) = change.tab.as_deref().filter(|t| !tabs.contains(t)) {
            return Err(format!(
                "the Library has no tab `{tab}`; it has {}",
                tabs.join(", ")
            ));
        }
        let pin = change.pin.as_ref().map(plain);
        if let Some(pin) = pin.as_ref().filter(|p| !tiles.contains(p)) {
            return Err(format!("no Library tile runs {}", named(pin)));
        }
        let unpin = change.unpin.as_ref().map(plain);
        if let Some(unpin) = unpin.as_ref().filter(|u| !favorites.contains(u)) {
            return Err(format!("{} is not a favorite", named(unpin)));
        }
        if let Some(tab) = &change.tab {
            self.tab.clone_from(tab);
        }
        self.open = change.open.unwrap_or(self.open || change.tab.is_some());
        self.used = change.used.unwrap_or(self.used);
        self.preview = change.preview.unwrap_or(self.preview);
        if let Some(pin) = pin {
            // Pinned again, it keeps its place unless given one; a new one goes last.
            let was = favorites.iter().position(|f| *f == pin);
            favorites.retain(|f| *f != pin);
            let at = (change.at.or(was)).map_or(favorites.len(), |at| at.min(favorites.len()));
            favorites.insert(at, pin);
        }
        favorites.retain(|f| Some(f) != unpin.as_ref());
        Ok(self.info(d, favorites))
    }

    /// The Library as `ui.library` answers.
    pub fn info(&self, d: Option<&Direction>, favorites: &[Step]) -> LibraryInfo {
        LibraryInfo {
            open: self.open,
            tab: (d.and_then(|d| self.shown(d))).map_or(String::new(), |t| t.id.clone()),
            tabs: d.map_or(Vec::new(), |d| {
                d.library.iter().map(|t| t.id.clone()).collect()
            }),
            favorites: favorites.to_vec(),
            used: self.used,
            preview: self.preview,
        }
    }

    /// Draws the drawer while it is open, along the bottom of what the trays and a docked
    /// Reference View leave: call after them and before the 3D view, so the view ends above
    /// it. Its tab buttons, then Close (and on the materials' tab, Show Used
    /// and Preview), then the tab's tiles. Returns what it asked to run.
    pub fn drawer(&mut self, ui: &mut egui::Ui, now: &Now, rec: &mut Recorder) -> Vec<Step> {
        let hovered = self.hovered.take();
        self.previewing = None;
        let Some(d) = now.direction.filter(|_| self.open) else {
            return Vec::new();
        };
        let Some(shown) = self.shown(d) else {
            return Vec::new();
        };
        let materials = shown.show == "looks";
        let used = self.used && materials;
        let s = &theme(ui.ctx()).spacing;
        let label = ui.text_style_height(&egui::TextStyle::Small);
        // With room under the tiles for their scroll bar (decision 0025, rule 11).
        let bar = ui.spacing().scroll.allocated_width();
        let height = 2.0 * s.margin + 2.0 * s.control + TILE + label + 3.0 * s.item[1] + bar;
        let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(s.margin);
        let drawer = egui::Panel::bottom("library")
            .exact_size(height)
            .resizable(false);
        let mut asked = Vec::new();
        drawer.frame(frame).show(ui, |ui| {
            rec.panel("library", "Library", ui.max_rect());
            asked = self.header(ui, (d, shown), now, rec);
            // Until material paint, every part draws in the active material.
            let in_use = |c: &Control| {
                let step = c.step();
                let name = step.as_ref().and_then(|s| s.args.get("matcap")?.as_str());
                !used || name == Some(now.looks.selected.as_str())
            };
            let mut rested = None;
            let source = egui::scroll_area::ScrollSource {
                drag: egui::scroll_area::DragScroll::Always,
                ..Default::default()
            };
            egui::ScrollArea::horizontal()
                .scroll_source(source)
                .show(ui, |ui| {
                    // What scrolled out of sight is not reported where a tap would miss it.
                    rec.list(Some(ui.clip_rect()));
                    ui.horizontal(|ui| {
                        for c in controls_of(&shown.show, now).iter().filter(|c| in_use(c)) {
                            let (tapped, on) = Self::tile(ui, c, now, rec);
                            asked.extend(tapped);
                            if on && materials {
                                let step = c.step();
                                let name = step.as_ref().and_then(|s| s.args.get("matcap"));
                                rested = name.and_then(Value::as_str).map(str::to_string);
                            }
                        }
                    });
                });
            rec.list(None);
            self.rest(ui.ctx(), hovered, rested);
        });
        asked
    }

    /// The drawer's top row: a button for each of `d`'s tabs, `shown`'s lit, then from the
    /// right Close and, on the materials' tab, Show Used and Preview. Returns what was tapped.
    fn header(
        &self,
        ui: &mut egui::Ui,
        (d, shown): (&Direction, &LibraryTab),
        now: &Now,
        rec: &mut Recorder,
    ) -> Vec<Step> {
        let mut asked = Vec::new();
        ui.horizontal(|ui| {
            for tab in &d.library {
                let press = Press {
                    label: &tab.title,
                    step: step(json!({ "tab": tab.id })),
                    key: None,
                    lit: tab.id == shown.id,
                    shows: None,
                };
                asked.extend(run_button(ui, press, now, rec));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close = Press {
                    label: "Close",
                    step: step(json!({ "open": false })),
                    key: None,
                    lit: false,
                    shows: None,
                };
                let x = Face::of(icons::of_arg("ui.library", "open", false));
                asked.extend(run_as(ui, close, x, now, rec));
                let toggles = [
                    ("Show Used", "used", self.used),
                    ("Preview", "preview", self.preview),
                ];
                for (label, key, on) in toggles.into_iter().filter(|_| shown.show == "looks") {
                    let toggle = Press {
                        label,
                        step: step(json!({ key: !on })),
                        key: None,
                        lit: on,
                        shows: None,
                    };
                    asked.extend(run_button(ui, toggle, now, rec));
                }
            });
        });
        asked
    }

    /// The pen is on the tile of material `on` (`None`: on none), and was on `before`'s since
    /// its time: once it has rested `REST` there with Preview on, the view previews it.
    fn rest(&mut self, ctx: &egui::Context, before: Option<(String, f64)>, on: Option<String>) {
        let time = ctx.input(|i| i.time);
        self.hovered = on.map(|name| match before {
            Some((was, since)) if was == name => (name, since),
            _ => (name, time),
        });
        let Some((name, since)) = self.hovered.as_ref().filter(|_| self.preview) else {
            return;
        };
        let left = REST - (time - since);
        // A millisecond's grace: the clock's nanoseconds and egui's seconds round apart.
        if left <= 1e-3 {
            self.previewing = Some(name.clone());
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(left));
        }
    }

    /// One tile: the control `c` as a square button with its picture, or its stand-in (its
    /// group, a brush's family, else its label, in `text_dim`), its label under it, then Pin,
    /// lit while it is a favorite. Returns what was tapped and whether the pen is on it; a
    /// control waiting for a feature has no tile.
    fn tile(ui: &mut egui::Ui, c: &Control, now: &Now, rec: &mut Recorder) -> (Option<Step>, bool) {
        let Some(press) = press_of(c, false, now) else {
            return (None, false);
        };
        let (step, lit, label) = (press.step.clone(), press.lit, press.label.to_string());
        let off = now.applies.and_then(|a| a.why(&step)).is_some();
        let t = theme(ui.ctx());
        let colors = &t.colors;
        let ink = match (off, lit) {
            (true, _) => colors.text_off,
            (false, true) => colors.on_lit,
            (false, false) => colors.text_dim,
        };
        let Some(favorite) = c.step() else {
            return (None, false);
        };
        let pinned = now.layout.favorites.contains(&favorite);
        let pin = Press {
            label: "Pin",
            step: self::step(json!({ (if pinned { "unpin" } else { "pin" }): favorite })),
            key: None,
            lit: pinned,
            shows: None,
        };
        ui.vertical(|ui| {
            ui.set_width(TILE);
            let shown = pressed_as(ui, press, Face::Square(TILE), now, rec);
            let shown = hover(shown, c.hint.as_deref(), rec);
            let face = shown.rect.shrink(t.spacing.item[1]);
            let seen = ui.is_rect_visible(shown.rect);
            let swatch = Source::theme(&step, now.themes);
            let source = seen.then(|| Source::of(&step, now.materials)).flatten();
            match source.and_then(|s| picture(ui.ctx(), &s)) {
                _ if swatch.is_some() => swatch.into_iter().for_each(|t| colors_of(ui, face, t)),
                Some(picture) => {
                    let tint = if off { colors.text_off } else { Color32::WHITE };
                    let image = egui::Image::from_texture((picture, face.size())).tint(tint);
                    image.paint_at(ui, face);
                }
                None => stand_in(ui, face, c.group.as_deref().unwrap_or(&label), ink),
            }
            let under = if off { colors.text_off } else { colors.text };
            ui.add(egui::Label::new(RichText::new(&label).small().color(under)).truncate());
            let icon = Face::of(icons::of_arg("ui.library", "pin", "@tile"));
            let pinning = run_as(ui, pin, icon, now, rec);
            (shown.clicked().then_some(step).or(pinning), shown.hovered())
        })
        .inner
    }
}

/// A theme's tile: its own colors with no image, a panel holding a control and a lit control,
/// each with text on it as the theme draws it.
fn colors_of(ui: &egui::Ui, rect: egui::Rect, t: &Theme) {
    let c = &t.colors;
    let painter = ui.painter_at(rect);
    let radius = t.spacing.radius;
    painter.rect_filled(rect, radius, c.panel);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let half = rect.height() / 2.0;
    let inset = theme(ui.ctx()).spacing.item[1];
    for (row, fill, ink) in [(0.0, c.control, c.text), (half, c.lit, c.on_lit)] {
        let at = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, row),
            egui::vec2(rect.width(), half),
        );
        let control = at.shrink(inset);
        painter.rect_filled(control, radius, fill);
        painter.text(
            control.center(),
            egui::Align2::CENTER_CENTER,
            "Aa",
            font.clone(),
            ink,
        );
    }
}

/// A tile's text in place of a picture: `text` wrapped to `rect` and centred in it.
fn stand_in(ui: &egui::Ui, rect: egui::Rect, text: &str, ink: Color32) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let painter = ui.painter_at(rect);
    let galley = painter.layout(text.to_string(), font, ink, rect.width());
    let at = rect.center() - galley.size() / 2.0;
    painter.galley(at, galley, ink);
}
