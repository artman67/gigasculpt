//! The header bar along the top of the 3D view (decision 0060, section 2): the layout's
//! `[header]` menus by title at one end and its pen-end pieces at the other, the two swapped for
//! the left hand. When both do not fit on one row the pen end moves to a row of its own under the
//! menus, wrapping if it must; the bar never scrolls. When the titles do not fit a row of their
//! own they fold into one button, Menus (decision 0060 as amended for H7): a tap lists them under
//! it, as the panel `menus`, and a tap on one opens its menu beside it, toward the view; the pen
//! end then shares the button's row, wrapping beside it when that makes the bar less tall.
//!
//! A tap on a title opens its menu under it (`ui.menu`), lit while open; while one is open, the
//! pointer over another title opens that one. A menu lists its set's controls a row each: the
//! short label, "…" after one that opens the palette's form first, its key at the right, a small
//! dim heading over each group and a thin line between groups (and above a control with
//! `separator`), ▸ for a row that opens a submenu (its `options`, one level deep). A disabled row
//! is gray and its hint says why (rule 7). A pick runs and shuts the menu. A set with a slider,
//! or a piece drawn in code, opens as a dropdown panel instead, drawn as a tray section, which
//! stays open after a change. Escape or a press outside shuts either, and that press does
//! nothing else ([`early`]); Escape while the pen holds a pull in a panel cancels the pull and
//! leaves the panel open.
//!
//! The pen end holds the view buttons (decision 0060 as amended for H5), once the view strip's:
//! each an icon alone where it has one, with its name and hint on hover. A switch with options
//! (Frame, Wire, Floor, Isolate) runs on a tap and opens them on a press held past a click or a
//! right click (rule 8.5); an options button (Look) opens them on a tap. They open under the
//! button in the dropdown panel a menu opens in, as a panel by their own id (`look`), and the
//! button is a section of the header, folded while they are shut. One thing is open at a time,
//! a menu or a button's options, and Escape or a press outside shuts either alike.
//!
//! The bar and when it folds are in `bar`, the menus, the fold and its list in `menu`, the pen
//! end's view buttons in `pen` and the press a menu takes first in `early`.

mod bar;
mod early;
mod menu;
mod pen;

pub use bar::bar;
pub(crate) use bar::{frame, one_row};
pub use early::early;
pub use menu::menu;
pub(crate) use pen::{buttons, tip};

use gs_engine::{MenuInfo, MenuOpen, UiMenu};
use gs_protocol::Step;

use crate::direction::{Direction, HeaderSpec, title};
use crate::pieces::{Now, controls_of};
use crate::shell::Shell;

/// The room above and below the bar's rows, in points: a 32-point control in a 40-point bar.
pub(crate) const PAD: i8 = 4;

/// The least room between the menus and the pen end on one row, and between a row's label and
/// its key, in points.
const GAP: f32 = 16.0;

/// The least width of a menu, in points.
const MENU: f32 = 180.0;

/// The width of a dropdown panel, in points: room for two columns of looks.
const DROPDOWN: f32 = 380.0;

/// The header's menus as they show: the one open and its submenu, by their sets' names, or a
/// view button's options, by their id.
#[derive(Clone, Debug, Default)]
pub struct Menus {
    /// The menu open, or the view button's options open (see `options`).
    pub open: Option<String>,
    /// What is open is a view button's options, not a menu.
    options: bool,
    /// The submenu open in it.
    sub: Option<String>,
    /// The menu was opened by the pointer passing over its title, so a tap there keeps it.
    hovered: bool,
    /// Where the open menu's title, or the open options' button, shows: they open under it.
    at: Option<egui::Rect>,
    /// Where the open submenu's row shows, which the submenu opens beside.
    sub_at: Option<egui::Rect>,
    /// The folded menus' list is open; it opens with a menu opened while they are folded.
    list: bool,
    /// Where the button the menus fold into shows, while they are folded: the list opens under it.
    fold: Option<egui::Rect>,
    /// Where the pen pressed inside what is open, while it has not let go: a slider's pull in a
    /// dropdown panel (Deform's), which Escape cancels without shutting the panel ([`early`]).
    held: Option<egui::Pos2>,
}

impl Menus {
    /// Opens the menu `name`, shutting any other and its submenu, and keeping the folded menus'
    /// list as it is.
    fn show(&mut self, name: &str, hovered: bool) {
        *self = Self {
            open: Some(name.into()),
            hovered,
            list: self.list,
            fold: self.fold,
            ..Self::default()
        };
    }

    /// Opens the view button's options `id`, shutting any menu.
    fn show_options(&mut self, id: &str) {
        *self = Self {
            open: Some(id.into()),
            options: true,
            ..Self::default()
        };
    }

    /// Shuts the menu open and its submenu, or the options open.
    fn shut(&mut self) {
        *self = Self::default();
    }

    /// The menu open, if what is open is a menu.
    fn menu(&self) -> Option<&str> {
        self.open.as_deref().filter(|_| !self.options)
    }

    /// The view button's options open, by their id, if what is open is options.
    fn options(&self) -> Option<&str> {
        self.open.as_deref().filter(|_| self.options)
    }

    /// Whether a menu, a view button's options or the folded menus' list is open.
    pub fn shows(&self) -> bool {
        self.open.is_some() || self.list
    }
}

/// The two ends of the bar: the menus' titles, or the button they fold into, and the pen end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum End {
    Menus,
    Fold,
    /// The pen end, its sets that an options button folds drawn as that button while `true`.
    Pen(bool),
}

/// The layout's header, unless minimal chrome hides it.
fn spec<'a>(now: &Now<'a>) -> Option<&'a HeaderSpec> {
    now.layout.chrome(now.direction?.header.as_ref())
}

/// The fold's label and hint: the layout's `[header] fold`, else Menus with no hint.
fn fold_of(spec: &HeaderSpec) -> (&str, Option<&str>) {
    let fold = spec.fold.as_ref();
    fold.map_or(("Menus", None), |f| (f.label.as_str(), f.hint.as_deref()))
}

impl Shell {
    /// `ui.menu`: opens the header's menu `change.open` names, or shuts the one open (or a view
    /// button's options), and says which menu is open and the header's menus. A name the header
    /// has no menu by is refused, naming those it has.
    pub fn menu_action(&mut self, change: &UiMenu) -> Result<MenuInfo, String> {
        let shown = self.directions.get(self.shown());
        let header = (self.layout.layout).chrome(shown.and_then(|d| d.header.as_ref()));
        let menus = header.map(|h| h.menus.clone()).unwrap_or_default();
        match &change.open {
            Some(MenuOpen::Name(name)) if !menus.contains(name) => {
                let has = if menus.is_empty() {
                    "no menus".to_string()
                } else {
                    menus.join(", ")
                };
                return Err(format!("no menu called {name}; the header has {has}"));
            }
            Some(MenuOpen::Name(name)) => self.menus.show(name, false),
            Some(MenuOpen::Shut(_)) => self.menus.shut(),
            None => {}
        }
        Ok(MenuInfo {
            open: self.menus.menu().map(String::from),
            menus,
        })
    }
}

/// Where each control of `d`'s header lives (see `arrange::places`): a menu's under `Header >
/// File`, then its group's name (`Header > File > Blender`); a submenu's under its row
/// (`Header > File > Open recent`); the pen end's under `Header`, a view button's options under
/// it (`Header > Frame`, a line's under the line's label, `Header > Symmetry`) and an options
/// button's under its label (`Header > Look`).
pub fn places(d: &Direction, now: &Now) -> Vec<(Step, String)> {
    let Some(spec) = d.header.as_ref() else {
        return Vec::new();
    };
    let controls = &now.directions.controls;
    let menus = (spec.menus.iter()).map(|m| (m, Some(title(controls, m)), true));
    let ends = spec.show.iter().map(|p| (p, None, false));
    // An options button that folds a set (Symmetry's) is a way to it, as the menus' fold is.
    let options = (spec.options.iter())
        .filter(|o| controls_of(&o.id, now).is_empty())
        .flat_map(|o| (o.show.iter()).map(|p| (p, Some(o.label.as_str()), false)));
    let mut out = Vec::new();
    for (piece, menu, grouped) in menus.chain(ends).chain(options) {
        for c in controls_of(piece, now) {
            let mut home = vec!["Header"];
            home.extend(menu);
            home.extend(c.group.as_deref().filter(|_| grouped));
            let home = home.join(" > ");
            // A line's options live under the line's label (Symmetry's ▼).
            let shown = (c.line.as_deref().or(c.short.as_deref())).unwrap_or(&c.label);
            let under = format!("{home} > {shown}");
            out.extend(c.step_in(&now.state).map(|step| (step, home)));
            for o in c.options.iter().flat_map(|set| controls_of(set, now)) {
                out.extend(o.step_in(&now.state).map(|step| (step, under.clone())));
            }
        }
    }
    out
}
