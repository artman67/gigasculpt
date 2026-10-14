//! A layout drawn: its panels on their edges (left and right swapped for the left hand), each
//! showing its pieces in order, a tray's as folding sections under their titles; and where
//! every control lives, for the command palette.

use egui::scroll_area::{DragScroll, ScrollSource};
use egui::{Align2, FontId, Sense, Shape, Stroke, pos2, vec2};
use gs_protocol::Step;

use crate::direction::{Direction, Edge, PanelSpec, ReferenceSpec, title};
use crate::layout::Layout;
use crate::pieces::{Dragging, Fit, HEAD, Now, controls_of, draw, section, summary, within};
use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// What the panels asked for in a frame: commands to run, sections to fold shut or open, and
/// a tray's new width once a drag of its inner edge lets go (neither is a command: like
/// scrolling, they change nothing an agent acts on).
#[derive(Default)]
pub struct Asked {
    pub steps: Vec<Step>,
    /// The sections whose fold flips: the one whose title was tapped and, when that opens it
    /// with one section open at a time, the others open in its tray.
    pub fold: Vec<String>,
    /// The tray, by panel id, and its width in points.
    pub width: Option<(String, f32)>,
}

impl Asked {
    fn add(&mut self, other: Asked) {
        self.steps.extend(other.steps);
        self.fold.extend(other.fold);
        self.width = other.width.or(self.width.take());
    }
}

/// The narrowest a tray drags to, in points: as narrow as the docked Reference View goes.
const LEAST: f32 = 160.0;

/// A side panel's width in points: where its edge was dragged to, else its layout file's.
fn width(panel: &PanelSpec, now: &Now) -> f32 {
    now.layout
        .widths
        .get(&panel.id)
        .copied()
        .unwrap_or(panel.size)
}

/// The panels that show: those not hidden, and none under minimal chrome.
fn visible<'a>(d: &'a Direction, now: &Now) -> impl Iterator<Item = &'a PanelSpec> {
    let layout = now.layout;
    (d.panels.iter()).filter(|p| layout.shows(&p.id))
}

/// The side panels that show, by their edge.
fn sides<'a>(d: &'a Direction, now: &Now) -> impl Iterator<Item = &'a PanelSpec> {
    visible(d, now).filter(|p| matches!(p.edge, Edge::Left | Edge::Right))
}

/// How much the side panels shrink in a window `window` points wide: not at all while the 3D
/// view keeps at least half the width, else all by one factor so that it does (each then
/// rounded down to whole points, so its edge stays crisp). Shelves keep their size, so their
/// labels fit.
fn fit(d: &Direction, now: &Now, window: f32) -> f32 {
    let sides: f32 = sides(d, now).map(|p| width(p, now)).sum();
    if sides <= 0.0 {
        return 1.0;
    }
    (window / 2.0 / sides).clamp(0.0, 1.0)
}

/// The layout's Reference View, docked in a space `width` points wide (what the panels leave):
/// it takes at most half of it, so the 3D view keeps the other half.
pub fn reference(now: &Now, width: f32) -> ReferenceSpec {
    let mut spec = (now.direction.map(|d| d.reference.clone())).unwrap_or_default();
    spec.width = spec.width.min(width / 2.0);
    spec
}

/// Draws the panels of `d`, docked on their edges, the side panels shrunk if the window is too
/// narrow for them (see `fit`). A side panel's inner edge drags it wider or narrower, as far
/// as leaves the 3D view half the window. Returns what they asked for.
pub fn panels(
    ui: &mut egui::Ui,
    d: &Direction,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Asked {
    let window = ui.available_width();
    let fit = fit(d, now, window);
    let shown_width = |p: &PanelSpec| (width(p, now) * fit).floor();
    let room = window / 2.0 - sides(d, now).map(shown_width).sum::<f32>();
    let tallest = ui.available_height() * SHELF_SHARE;
    let mut asked = Asked::default();
    for panel in visible(d, now) {
        let id = egui::Id::new(("layout panel", &d.name, &panel.id));
        let edge = panel.edge.for_hand(now.layout.hand);
        let rows = id.with("rows");
        let shelf = || ui.data(|d| d.get_temp(rows)).unwrap_or(panel.size);
        let size = match edge {
            Edge::Left | Edge::Right => shown_width(panel),
            Edge::Top | Edge::Bottom => shelf(),
        };
        let side = match edge {
            Edge::Left => egui::Panel::left(id),
            Edge::Right => egui::Panel::right(id),
            Edge::Top => egui::Panel::top(id),
            Edge::Bottom => egui::Panel::bottom(id),
        };
        let tray = matches!(edge, Edge::Left | Edge::Right);
        let range = LEAST.min(size)..=size + room.max(0.0);
        let side = if tray {
            // egui drags the edge; the width is ours (the file's or the saved one, fitted), so
            // the size egui remembers from the last frame is dropped.
            ui.data_mut(|d| d.remove::<egui::PanelState>(id));
            let side = side.size_range(range.clone()).default_size(size);
            side.resizable(true)
        } else {
            side.exact_size(size).resizable(false)
        };
        let margin = theme(ui.ctx()).spacing.margin;
        let frame = egui::Frame::side_top_panel(ui.style()).inner_margin(margin);
        let shown = side.frame(frame).show(ui, |ui| {
            // What is wider than the panel is clipped at its edge, never reported as its size:
            // egui would move a panel on the right out by the excess, and the header and the 3D
            // view would take the room under it.
            let room = ui.max_rect();
            let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(room));
            let (more, short) = contents(&mut inside, panel, edge, now, dragging, rec);
            ui.expand_to_include_rect(room);
            asked.add(more);
            short
        });
        if tray {
            if let Some(at) = let_go(ui, id, edge, shown.response.rect) {
                let at = at.clamp(*range.start(), *range.end());
                asked.width = Some((panel.id.clone(), at));
            }
            continue;
        }
        // A shelf is as tall as its rows, from its size up to its share of the window, past
        // which it scrolls. Its rows are known once drawn, so a change shows in the next frame,
        // which it asks for at once.
        let wanted = (size + shown.inner).clamp(panel.size, tallest.max(panel.size));
        if (wanted - size).abs() >= 0.5 {
            ui.data_mut(|d| d.insert_temp(rows, wanted));
            ui.ctx().request_repaint();
        }
    }
    asked
}

/// The width a side panel on `edge`, drawn at `rect`, was left at when the pen let go of its
/// inner edge this frame: from the window's edge to the pen, as egui measures a drag. `None`
/// when no drag of it ended.
fn let_go(ui: &egui::Ui, id: egui::Id, edge: Edge, rect: egui::Rect) -> Option<f32> {
    // egui's id for a panel's edge handle (`resize_widget_id` in its `panel.rs`).
    let handle = ui.ctx().read_response(id.with("__resize"))?;
    let at = handle
        .interact_pointer_pos()
        .filter(|_| handle.drag_stopped())?;
    Some(match edge {
        Edge::Left => at.x - rect.left(),
        _ => rect.right() - at.x,
    })
}

/// The most of the window's height a shelf grows to, to show its rows whole; past it, it
/// scrolls.
const SHELF_SHARE: f32 = 1.0 / 3.0;

/// One panel's title for `ui.describe` and its pieces, in a scroll area the pen can drag. Trays
/// stack their pieces as sections; shelves run them along in rows. Returns what they asked for
/// and how much taller what they drew is than what shows (less than nothing when it all fits).
fn contents(
    ui: &mut egui::Ui,
    panel: &PanelSpec,
    edge: Edge,
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> (Asked, f32) {
    rec.panel(&panel.id, &panel.title, ui.max_rect());
    let room = ui.available_height();
    let source = ScrollSource {
        drag: DragScroll::Always,
        ..ScrollSource::default()
    };
    let mut asked = Asked::default();
    // As wide as the panel, so a shelf's bar sits at its edge as a tray's does (decision 0016,
    // section 17, item 10).
    let scrolled = egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .scroll_source(source)
        .show(ui, |ui| {
            // What scrolled out of sight is not reported where a tap would miss it.
            rec.list(Some(ui.clip_rect()));
            if matches!(edge, Edge::Left | Edge::Right) {
                let tray = sections(panel, now);
                for name in tray {
                    asked.add(fold(ui, (name, tray), now, dragging, rec));
                }
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for name in &panel.show {
                    asked
                        .steps
                        .extend(draw(ui, name, Fit::Shelf, now, dragging, rec));
                }
            });
        });
    rec.list(None);
    bar(ui, &scrolled, rec);
    (asked, scrolled.content_size.y - room)
}

/// A tray's sections: in the tray that shows the active tool's settings (`brush`), the tool's
/// own `tray` when it gives one (the Gizmo tool's header, Basics and Pose, decision 0060 section
/// 6); else the panel's.
fn sections<'a>(panel: &'a PanelSpec, now: &Now<'a>) -> &'a [String] {
    let tool = now.direction.and_then(|d| d.tool(&now.state));
    let own = panel.show.iter().any(|s| s == "brush");
    let tray = tool.map(|t| t.tray.as_slice());
    tray.filter(|t| own && !t.is_empty()).unwrap_or(&panel.show)
}

/// Records the bar of the scroll area `shown`, while what it holds is taller than it is: egui
/// lays it out beside the pieces (decision 0025, rule 11) and senses it under its own id, in
/// what of it the panel shows.
fn bar<R>(ui: &egui::Ui, shown: &egui::scroll_area::ScrollAreaOutput<R>, rec: &mut Recorder) {
    let scrolls = shown.inner_rect.height().ceil() < shown.content_size.y;
    // egui's id for a vertical bar (`id.with(d)` in its `scroll_area.rs`, `d` 1 for vertical).
    let bar = ui.ctx().read_response(shown.id.with(1_usize));
    if let Some(bar) = bar.filter(|_| scrolls) {
        rec.widget(
            Widget::plain("Scroll bar", Role::ScrollBar),
            bar.interact_rect,
        );
    }
}

/// One section of a tray (`name`, among the tray's sections `tray`): its title row, which folds
/// it shut or open and shows the section's summary after the title, then its piece unless it
/// is folded. Recorded with its rectangle and whether
/// it is folded. The tray's header (`head`) has no title row and never folds.
fn fold(
    ui: &mut egui::Ui,
    (name, tray): (&str, &[String]),
    now: &Now,
    dragging: &mut Dragging,
    rec: &mut Recorder,
) -> Asked {
    let mut asked = Asked::default();
    let space = theme(ui.ctx()).spacing.section;
    if name == HEAD {
        ui.horizontal_wrapped(|ui| asked.steps.extend(section(ui, name, now, dragging, rec)));
        ui.add_space(space);
        return asked;
    }
    let folded = now.layout.is_folded(name);
    let title = title(&now.directions.controls, name);
    let drawn = ui.vertical(|ui| {
        let (row, end) = titled_row(ui, title, folded);
        if let Some(words) = summary(name, now) {
            summarize(ui, (row.rect, end), &words, rec);
        }
        if row.clicked() {
            let shift = ui.input(|i| i.modifiers.shift);
            asked.fold = folds(name, tray, now.layout, shift);
        }
        if !folded {
            ui.horizontal_wrapped(|ui| asked.steps.extend(section(ui, name, now, dragging, rec)));
        }
    });
    rec.section(name, title, folded, false, drawn.response.rect);
    ui.add_space(space);
    asked
}

/// The sections a tap on `name`'s title folds shut or opens: `name`, and when the tap opens it
/// with one section open at a time and no Shift held, every other open section of `tray`.
fn folds(name: &str, tray: &[String], layout: &Layout, shift: bool) -> Vec<String> {
    let shuts = layout.one_open && layout.is_folded(name) && !shift;
    let others = tray
        .iter()
        .filter(|s| shuts && *s != name && *s != HEAD && !layout.is_folded(s));
    std::iter::once(name.to_string())
        .chain(others.cloned())
        .collect()
}

/// Paints `words` dim on a section's title row `row`, after its title, which ends at `from`
/// across, cut short at the row's end, and records them as a label.
fn summarize(ui: &egui::Ui, (row, from): (egui::Rect, f32), words: &str, rec: &mut Recorder) {
    let t = theme(ui.ctx());
    let pad = t.spacing.button[0];
    let room = egui::Rect::from_x_y_ranges(from + pad..=row.right() - pad, row.y_range());
    if !room.is_positive() {
        return;
    }
    let font = FontId::proportional(t.sizes.body);
    let mut job = egui::text::LayoutJob::simple_singleline(words.into(), font, t.colors.text_dim);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.width());
    let galley = ui.painter().layout_job(job);
    let rect = Align2::LEFT_CENTER.anchor_size(room.left_center(), galley.size());
    ui.painter().galley(rect.min, galley, t.colors.text_dim);
    rec.widget(Widget::plain(words, Role::Label), rect);
}

/// A section's title row, as wide as the tray: on the header color (the hover color under the
/// pointer), a mark pointing right while folded and down while open, and the title; with where
/// its title ends across.
pub(crate) fn titled_row(ui: &mut egui::Ui, title: &str, folded: bool) -> (egui::Response, f32) {
    let t = theme(ui.ctx());
    let size = vec2(ui.available_width(), t.spacing.control);
    let (rect, row) = ui.allocate_exact_size(size, Sense::click());
    row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, title));
    let fill = if row.hovered() {
        t.colors.hover
    } else {
        t.colors.header
    };
    let painter = ui.painter();
    painter.rect_filled(rect, t.spacing.radius, fill);
    let pad = t.spacing.button[0];
    let mark = egui::Rect::from_center_size(
        pos2(rect.left() + pad + 4.0, rect.center().y),
        vec2(8.0, 8.0),
    );
    let points = if folded {
        vec![mark.left_top(), mark.left_bottom(), mark.right_center()]
    } else {
        vec![mark.left_top(), mark.right_top(), mark.center_bottom()]
    };
    painter.add(Shape::convex_polygon(
        points,
        t.colors.text_dim,
        Stroke::NONE,
    ));
    let at = pos2(mark.right() + pad, rect.center().y);
    let font = FontId::proportional(t.sizes.body);
    let end = painter.text(at, Align2::LEFT_CENTER, title, font, t.colors.text);
    (row, end.right())
}

/// Where each control of `d` lives: its panel's title, then in a tray its section's title, then
/// its group's (along a shelf, its group's or else its set's title), as in `Scene tray >
/// Geometry`, `Brush shelf > Mask` or, for a set drawn inside a piece, `Brush tray > Brush >
/// Gizmo`. A button's options live under it. The header's live in its menus and at its pen end
/// (`header::places`), and the brush wheel's slots in the wheel.
pub fn places(d: &Direction, now: &Now) -> Vec<(Step, String)> {
    let mut out = Vec::new();
    for panel in &d.panels {
        let tray = matches!(panel.edge, Edge::Left | Edge::Right);
        for name in &panel.show {
            let title = title(&now.directions.controls, name);
            let drawn = controls_of(name, now).into_iter();
            for c in drawn.chain(within(d, name, now)) {
                let mut home = vec![panel.title.as_str()];
                if tray || c.group.is_none() {
                    home.push(title);
                }
                home.extend(c.group.as_deref());
                let home = home.join(" > ");
                let under = format!("{home} > {}", c.label);
                out.extend(c.step_in(&now.state).map(|step| (step, home)));
                for o in c.options.iter().flat_map(|set| controls_of(set, now)) {
                    out.extend(o.step_in(&now.state).map(|step| (step, under.clone())));
                }
            }
        }
    }
    out.extend(crate::header::places(d, now));
    for piece in &d.settings {
        for c in controls_of(piece, now) {
            let mut home = vec!["Settings", title(&now.directions.controls, piece)];
            home.extend(c.group.as_deref());
            out.extend(c.step_in(&now.state).map(|step| (step, home.join(" > "))));
        }
    }
    let wheel = d
        .wheel
        .as_ref()
        .map(|w| w.show.as_str())
        .unwrap_or_default();
    for c in controls_of(wheel, now) {
        out.extend(
            c.step_in(&now.state)
                .map(|step| (step, "Brush wheel".to_string())),
        );
    }
    out
}
