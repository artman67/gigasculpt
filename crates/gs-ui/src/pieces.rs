//! The pieces the interface arranges, each drawn by one function wherever it appears: a control
//! (a button that runs its command, lit while the state value it names has its `on` value, or
//! a slider setting one of its command's arguments; drawn disabled, with why on hover, while its
//! command cannot run),
//! the brushes (or a button per family, `families`: `family.rs`), the brush tray's header
//! (`head`: the brush's name, opening the brush picker, and its ⋯), the active tool's settings
//! (`brush`, as the layout's tools name them), the brush's value sliders (size and strength
//! each with its pen toggle, Falloff, and size as a gizmo drag's Reach), the Pressure popover
//! with the brush's pressure curves (`pressure`; the Pen set has the session's size curve), the
//! gizmo's Lock and Sticky (`pose`), the brush's tip (`curve`: its picture, Softness, the Edit
//! shape popover and Exact), the Alpha palette (`alpha`), the colors a brush paints and Fill
//! (`paint`), the MatCaps, the active
//! material's sliders (`material`), the parts with their eyes and paint switches, the level
//! stepper (`detail`), Isolate's button per part, the Library's favorites and the keys cheat
//! sheet.
//! Each records what it drew, with its command and the state value it shows, for `ui.describe`.

mod alpha;
mod buttons;
pub(crate) mod deform;
mod detail;
mod head;
mod key_sheet;
mod material;
mod paint;
mod parts;
pub(crate) mod picker;
mod placement;
mod sliders;
pub(crate) mod spring;

use egui::{Popup, PopupCloseBehavior, RectAlign, RichText, vec2};
use gs_core::brush::BrushPreset;
use gs_core::curve::Curve;
use gs_core::gizmo::Pose;
use gs_engine::{BrushState, Materials, PaletteEntry, StateValues};
use gs_protocol::Step;
use serde_json::json;

use buttons::label_of;
pub use buttons::{
    Face, Press, control, faced, held_past_click, hover, options_mark, press_of, pressed_as,
    run_as, run_button,
};
pub(crate) use detail::ends as level_ends;
pub(crate) use head::HEAD;
use key_sheet::keys;
pub(crate) use parts::part_list;
use parts::{active, isolate, parts};
pub(crate) use sliders::{Reads, fine_shown, knob, number, running};
use sliders::{slider, titled};

use crate::applies::Applies;
use crate::curve;
use crate::direction::{Control, Direction, Directions, split, title};
use crate::held::Held;
use crate::keymap::Keymap;
use crate::layout::Layout;
use crate::pressure;
use crate::record::{Recorder, Role, Widget};
use crate::theme::{Themes, theme};
use crate::values::Value;

/// What the pieces show, as of this frame.
pub struct Now<'a> {
    /// The brush as the window shows it, with any value being set in place.
    pub brush: &'a BrushState,
    /// The brush the pen strokes with, as a stroke uses it, as the engine last reported it;
    /// `None` before it has.
    pub preset: Option<&'a BrushPreset>,
    /// The curve the curve editor's Copy holds to paste; `None` until one is copied.
    pub copied: Option<&'a Curve>,
    /// The session's state values (`session.state`), with the brush as above.
    pub state: StateValues,
    pub looks: &'a Materials,
    /// The palette's materials with their recipes, which their pictures are made from.
    pub materials: &'a [PaletteEntry],
    /// The themes the window can show.
    pub themes: &'a Themes,
    pub parts: &'a [ListedPart],
    pub keymap: &'a Keymap,
    pub directions: &'a Directions,
    pub layout: &'a Layout,
    /// The layout shown; `None` when no layout files are loaded.
    pub direction: Option<&'a Direction>,
    pub held: Option<Held>,
    /// Which controls' commands cannot run now, as the engine last said; `None` before the
    /// window has asked.
    pub applies: Option<&'a Applies>,
    /// The part whose gizmo the 3D view shows and its pose as drawn; `None` while the gizmo
    /// tool is off.
    pub gizmo: Option<(&'a str, Pose)>,
}

/// A slider being dragged: which value, and where it is.
pub type Dragging = Option<(Value, f32)>;

/// A part as the window lists it: its name, whether it shows, its own visibility, which
/// Isolate leaves alone (decision 0028), whether it is the active part (decision 0033), and
/// whether its paint shows (decision 0039, section 10).
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one part, each one a row's light, as PartReport's"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedPart {
    pub name: String,
    pub shown: bool,
    pub visible: bool,
    pub active: bool,
    pub painted: bool,
}

/// The controls a generated piece shows: the brushes, the MatCaps, the shipped alphas (each
/// setting the brush's alpha, saying its name, size and bits on hover), the layouts and the
/// themes, each lit by the state value it sets. `None` for any other piece.
fn generated(name: &str, now: &Now) -> Option<Vec<Control>> {
    let pick = |label: &str, command, args, (shows, on): (&str, &str)| {
        Control::runs(label, command, &args).showing(shows, on)
    };
    Some(match name {
        "brushes" | "families" => now.directions.brush_controls(),
        "looks" => (now.looks.looks.iter())
            .map(|l| {
                pick(
                    &l.title,
                    "view.look",
                    json!({ "matcap": l.name }),
                    ("look", &l.name),
                )
            })
            .collect(),
        "alphas" => (gs_engine::library::alphas().into_iter())
            .map(|(name, title)| Control {
                hint: gs_engine::library::alpha_hover(name),
                ..pick(
                    title,
                    "brush.set",
                    json!({ "alpha": name }),
                    ("brush.alpha", name),
                )
            })
            .collect(),
        "layouts" => (now.directions.list.iter())
            .map(|d| {
                pick(
                    &d.title,
                    "ui.layout",
                    json!({ "name": d.name }),
                    ("layout", &d.name),
                )
            })
            .collect(),
        "themes" => (now.themes.list().into_iter())
            .map(|t| {
                pick(
                    &t.title,
                    "ui.layout",
                    json!({ "theme": t.name }),
                    ("layout.theme", &t.name),
                )
            })
            .collect(),
        _ => return None,
    })
}

/// The controls a piece shows: a set's or a generated piece's, or one group of either
/// (`brushes/Mask`). Empty for the other shared widgets.
pub fn controls_of(piece: &str, now: &Now) -> Vec<Control> {
    let (name, group) = split(piece);
    let Some(all) = generated(name, now) else {
        return (now.directions.controls.piece(piece))
            .map(|(_, picked)| picked.into_iter().cloned().collect())
            .unwrap_or_default();
    };
    let in_group = |c: &Control| group.is_none_or(|g| c.group.as_deref() == Some(g));
    all.into_iter().filter(in_group).collect()
}

/// The controls of the sets a piece draws inside itself: the brush's section holds each of
/// `d`'s tools' settings while that tool is active, and the tray's header the Gizmo tool's ⋯
/// while it is on, so the palette says they live there.
pub fn within(d: &Direction, piece: &str, now: &Now) -> Vec<Control> {
    match piece {
        "brush" => (d.tools.iter().flat_map(|t| &t.show))
            .flat_map(|p| controls_of(p, now))
            .collect(),
        HEAD => controls_of(head::GIZMO, now),
        _ => Vec::new(),
    }
}

/// Every control the layouts know: each set's but a list row's, then the brushes, the MatCaps,
/// the alphas, the layouts and the themes.
pub fn all_controls(now: &Now) -> Vec<Control> {
    let sets = now.directions.controls.sets.iter().filter(|s| !s.row);
    let sets = sets.map(|s| s.name.as_str());
    let pieces = sets.chain(["brushes", "looks", "alphas", "layouts", "themes"]);
    pieces.flat_map(|piece| controls_of(piece, now)).collect()
}

/// Where a piece is drawn, which decides its titles and labels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fit {
    /// Along a shelf: its title, then its controls by their short labels, group labels in the
    /// row.
    Shelf,
    /// In a tray, under its section's title row: each group label on a row of its own.
    Section,
}

/// Draws a piece as a tray section's contents, under the section's title row (`arrange`).
pub fn section(
    ui: &mut egui::Ui,
    piece: &str,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    draw(ui, piece, Fit::Section, now, dragging, rec)
}

/// Pieces stacked in a small window (a view button's options, the settings window, the quick menu),
/// each under its title; one group of a set (`set/Group`) under the group's own label only, or
/// under none when its line's label names it (`controls`), and a brush slider (`SLIDERS`),
/// labelled with its title, under none. A set whose every control
/// waits on a `while` that does not hold shows nothing, not even its title. Returns what they
/// asked to run.
pub fn stacked(
    ui: &mut egui::Ui,
    pieces: &[String],
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Vec<Step> {
    let mut asked = Vec::new();
    for name in pieces {
        let all = controls_of(name, now);
        if !all.is_empty() && !all.iter().any(|c| c.shown(&now.state)) {
            continue;
        }
        if split(name).1.is_none() && !SLIDERS.contains(&name.as_str()) {
            let heading = RichText::new(title(&now.directions.controls, name));
            label(ui, heading.small().weak(), rec);
        }
        ui.horizontal_wrapped(|ui| asked.extend(section(ui, name, now, dragging, rec)));
        ui.add_space(theme(ui.ctx()).spacing.item[1]);
    }
    asked
}

/// Draws a piece where `fit` says: along a shelf (`arrange`), or as a tray section's contents
/// (`section`). Returns what it asked to run.
pub(crate) fn draw(
    ui: &mut egui::Ui,
    piece: &str,
    fit: Fit,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    let headed = fit != Fit::Section;
    match piece {
        HEAD => head::section(ui, now, rec),
        "brush" => brush(ui, fit, now, dragging, rec),
        "size" => slider(ui, Value::Size, now, dragging, rec),
        "strength" => slider(ui, Value::Strength, now, dragging, rec),
        "pressure" => pressure::button(ui, now, rec),
        "falloff" => slider(ui, Value::Falloff, now, dragging, rec),
        "reach" => titled(ui, (Value::Size, "Reach", Some(REACH)), now, dragging, rec),
        "pose" => crate::gizmo::pose_buttons(ui, now, rec),
        // The list alone: its operations are the header's Part menu, and Rows shown is in
        // Settings > Interface. Isolate's options: a button per part, then the set's End Isolate.
        "parts" => parts(ui, now, headed, rec),
        "detail" => detail::section(ui, now, rec),
        "isolate" => {
            let listed = isolate(ui, now, rec);
            controls(ui, piece, fit, false, now, rec).or(listed)
        }
        // The active material's section, whose title draws the set of the same name: the ⋯
        // listing Undo, Redo, Reset, Copy, Rename and Delete.
        "material" => material::section(ui, now, rec),
        "curve" => curve::section(ui, now, dragging, rec),
        "alpha" => alpha::palette(ui, now, rec),
        "paint" => paint::section(ui, now, rec),
        "depth" => placement::section(ui, now, rec),
        "deform" => deform::section(ui, now, rec),
        "pen" => controls(ui, piece, fit, headed, now, rec).or(pressure::global(ui, now, rec)),
        "families" => crate::family::buttons(ui, headed, now, rec),
        "favorites" => crate::library::favorites(ui, headed, headed, now, rec),
        "keys" => keys(ui, now, headed, rec),
        _ => controls(ui, piece, fit, headed, now, rec),
    }
}

/// What a tray section's title row shows after its title: open or folded, Parts' the active
/// part's name (decision 0060 as amended for S1, section 1); folded, Depth's Imbed and Gravity,
/// Detail's level and Paint's two colors (decision 0060, section 6); `None` for the others.
pub fn summary(piece: &str, now: &Now) -> Option<String> {
    let folded = now.layout.is_folded(piece);
    match piece {
        "parts" => active(now),
        "depth" if folded => {
            let n = |name: &str| now.state.get(name).and_then(serde_json::Value::as_f64);
            let (imbed, gravity) = (n("brush.depth.imbed")?, n("brush.post.gravity")?);
            let pull = if gravity == 0.0 {
                "off".into()
            } else {
                brief(gravity)
            };
            Some(format!("Imbed {} · Gravity {pull}", brief(imbed)))
        }
        "detail" if folded => Some(format!("Level {}", detail::readout(now))),
        "paint" if folded => paint::summary(now),
        _ => None,
    }
}

/// `v` to hundredths at most, as a summary says it: 0.1, -0.5, 0.
fn brief(v: f64) -> String {
    // Adding 0 turns a rounded -0 into 0.
    format!("{}", (v * 100.0).round() / 100.0 + 0.0)
}

/// Size's or Strength's slider (`v`, the piece `key`) with its pen toggle at the row's right end
/// (decision 0060, section 6): the rail gives up the toggle's width, so the value stays beside
/// it, and the toggle keeps its place whatever the value reads.
fn penned(
    ui: &mut egui::Ui,
    (v, key): (Value, &str),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    let side = ui.spacing().interact_size.y;
    let toggle = side + ui.spacing().item_spacing.x + 1.0;
    let room = vec2((ui.available_width() - toggle).max(0.0), side);
    let column = egui::Layout::top_down(egui::Align::Min);
    let shown = ui.allocate_ui_with_layout(room, column, |ui| {
        ui.set_min_width(room.x);
        ui.spacing_mut().slider_width -= toggle;
        slider(ui, v, now, dragging, rec)
    });
    shown.inner.or(pressure::pen(ui, key, now, rec))
}

/// A popover for the button `shown` (decision 0060, section 6): a tap on the button opens or
/// shuts it, beside the row the button is on at its height and as wide as that row, inside the
/// window; Escape or a press outside shuts it, and a press inside keeps it open. `ui.describe`
/// lists what it holds with the button's panel, whole. Returns its id and, while it shows, what
/// `add` returned.
pub(crate) fn popover<R>(
    ui: &egui::Ui,
    shown: &egui::Response,
    rec: &mut Recorder,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder) -> R,
) -> (egui::Id, Option<R>) {
    let row = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), shown.rect.y_range());
    let popup = Popup::from_toggle_button_response(shown)
        .anchor(row)
        .align(RectAlign::RIGHT_START)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
    let id = popup.get_id();
    let listing = rec.listing();
    rec.list(None);
    let shows = popup.show(|ui| {
        ui.set_width(row.width());
        add(ui, rec)
    });
    rec.list(listing);
    (id, shows.map(|s| s.inner))
}

/// Reach's hint: the brush's Size, as far as a soft gizmo drag reaches.
const REACH: &str = "How far a soft drag reaches from the gizmo: the brush's Size.";

/// The brush's value sliders, each labelled with its title: on a row of its own wherever it is
/// drawn as a tool's setting, and with no title over it in a small window (`stacked`).
const SLIDERS: [&str; 4] = ["size", "strength", "falloff", "reach"];

/// The active tool's settings (decision 0025, section 3, amended 2026-10-11): the pieces the
/// layout names for its first tool whose state value is true. The Workbench's are the brush's
/// sliders or, while the gizmo tool is on, the gizmo's settings, the brush's Size as the Reach
/// of a soft drag (decision 0027, section 5) and Lock and Sticky.
fn brush(
    ui: &mut egui::Ui,
    fit: Fit,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Option<Step> {
    let tool = now.direction?.tool(&now.state)?;
    let mut asked = None;
    for piece in &tool.show {
        // Size and Strength take their pen toggles here, in Basics; elsewhere (the quick menu, a
        // slider shown alone) they stand bare.
        let drawn = match piece.as_str() {
            "size" => penned(ui, (Value::Size, piece), now, dragging, rec),
            "strength" => penned(ui, (Value::Strength, piece), now, dragging, rec),
            _ => draw(ui, piece, fit, now, dragging, rec),
        };
        asked = drawn.or(asked);
        if SLIDERS.contains(&piece.as_str()) {
            ui.end_row();
        }
    }
    asked
}

/// A set's title when `titled` (never in a tray, where its section's title row stands for it),
/// then its controls under their groups' labels, but for a group whose controls all sit on one
/// labelled line, which that label names. Along a shelf a title starts a new row when it
/// and the control after it would not fit on this one, so a title is never left alone at the
/// end of a row.
fn controls(
    ui: &mut egui::Ui,
    piece: &str,
    fit: Fit,
    titled: bool,
    now: &Now,
    rec: &mut Recorder,
) -> Option<Step> {
    let mut titles = Vec::new();
    if titled {
        titles.push(RichText::new(title(&now.directions.controls, piece)).strong());
    }
    let (mut group, mut asked, mut started) = (None, None, false);
    let mut line = None;
    let shown: Vec<Control> = (controls_of(piece, now).into_iter())
        .filter(|c| c.shown(&now.state))
        .collect();
    for c in &shown {
        let mut headed = false;
        if c.group.is_some() && c.group != group {
            group.clone_from(&c.group);
            // A group on one labelled line is named by the line's label alone (rule 6).
            if !one_line(&shown, c) {
                let name = RichText::new(c.group.as_deref().unwrap_or_default());
                titles.push(name.small().weak());
            }
            if fit == Fit::Section && !titles.is_empty() {
                // In a tray a group's label heads a row of its own.
                if started {
                    ui.end_row();
                }
                titles.drain(..).for_each(|text| label(ui, text, rec));
                ui.end_row();
                headed = true;
            }
        }
        // A line's controls share a row after its label, each by its short label; in a tray
        // the line starts a row and the control after it starts the next.
        if c.line != line {
            line.clone_from(&c.line);
            if fit == Fit::Section {
                if started && !headed {
                    ui.end_row();
                }
                ui.set_row_height(ui.spacing().interact_size.y);
                started = false;
            }
            // A line with an empty label shares a row with no label before it.
            titles.extend(line.as_deref().filter(|l| !l.is_empty()).map(RichText::new));
        }
        let short = fit != Fit::Section || line.is_some();
        if !titles.is_empty() {
            let label = RichText::new(label_of(c, short));
            let texts: Vec<&RichText> = titles.iter().chain([&label]).collect();
            keep_together(ui, &texts);
        }
        titles.drain(..).for_each(|text| label(ui, text, rec));
        // In a tray a slider takes a row of its own: egui lays a slider out with its label as
        // one piece, which a wrapping row never moves down.
        let row = fit == Fit::Section && c.dragged().is_some();
        if row && started {
            ui.end_row();
        }
        asked = control(ui, c, short, now, rec).or(asked);
        started = !row;
        if row {
            ui.end_row();
        }
    }
    asked
}

/// Whether every control of `c`'s group in `shown` sits on `c`'s line and that line has a
/// label, which then names the row, so the group's own label would only say it again.
fn one_line(shown: &[Control], c: &Control) -> bool {
    let labelled = c.line.as_deref().is_some_and(|l| !l.is_empty());
    labelled && (shown.iter().filter(|o| o.group == c.group)).all(|o| o.line == c.line)
}

/// Starts a new row unless `texts` (labels, the last one a button's) fit on this one, or this
/// row is still empty.
pub(crate) fn keep_together(ui: &mut egui::Ui, texts: &[&RichText]) {
    let spacing = ui.spacing().item_spacing.x;
    let padding = 2.0 * ui.spacing().button_padding.x;
    let width = |t: &RichText| {
        let text = egui::WidgetText::from(t.clone());
        let galley = text.into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
        galley.size().x + spacing
    };
    let need: f32 = texts.iter().map(|t| width(t)).sum::<f32>() + padding;
    let left = ui.available_size_before_wrap().x;
    if need > left && left < ui.max_rect().width() - 1.0 {
        ui.end_row();
        // egui starts the row with no height, which would put a label at its top above the
        // buttons beside it; this gives it a first row's height, as `horizontal_wrapped` does.
        ui.set_row_height(ui.spacing().interact_size.y);
    }
}

/// A label, recorded.
pub fn label(ui: &mut egui::Ui, text: RichText, rec: &mut Recorder) {
    let words = text.text().to_string();
    let shown = ui.label(text);
    rec.widget(Widget::plain(words, Role::Label), shown.rect);
}
