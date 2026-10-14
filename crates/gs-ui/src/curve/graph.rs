//! A curve graph the pen edits (decision 0048): dragging a point moves it (the two ends only up
//! and down), a tap away from the points adds one there, a tap on a point between the ends turns
//! it from smooth to sharp or back, and dragging such a point off the graph takes it away, unless
//! the pen brings it back on before it lifts, which turns it instead. Each runs the curve's
//! command once, when the pen lifts. On a pressure graph a point dragged within [`SNAP`] of the
//! top lands on it (decision 0036, section 6). Smooth points are drawn round, sharp ones square.

use egui::{Pos2, Rect, Sense, Shape as Line, Stroke, StrokeKind, pos2, vec2};
use gs_core::curve::{Curve, Shape};
use gs_protocol::Step;
use serde_json::{Value, json};

use crate::record::{Recorder, Role, Widget};
use crate::theme::theme;

/// How far from a point, in points, the pen may press to take it: half a pen target.
const GRAB: f32 = 16.0;
/// The least distance between two points along the graph.
const GAP: f32 = 0.02;
/// Straight pieces drawn per line.
const STEPS: usize = 64;
/// The graph's height for its width.
const ASPECT: f32 = 0.6;
/// The widest the graph grows, in points.
const WIDEST: f32 = 360.0;
/// A handle's radius, in points.
const HANDLE: f32 = 5.0;
/// How near the top of a pressure graph a point lands on it: 3 percent.
const SNAP: f32 = 0.03;

/// A curve's points, which of them are smooth, and its Steps, Step strength and Tile.
#[derive(Clone, Debug, PartialEq)]
pub struct Points {
    pub(super) at: Vec<[f32; 2]>,
    pub(super) smooth: Vec<bool>,
    pub(super) shape: Shape,
}

/// The points being dragged, between frames: which one the pen holds, and whether the pen is
/// off the graph with it, where letting go takes it away.
#[derive(Clone, Debug)]
struct Held {
    points: Points,
    index: usize,
    off: bool,
}

impl Held {
    /// The curve if the pen let go now.
    fn result(&self) -> Points {
        let mut points = self.points.clone();
        if self.off {
            points.remove(self.index);
        }
        points
    }
}

impl Points {
    /// `c`'s points and settings.
    pub fn of(c: &Curve) -> Self {
        let at = c.points().to_vec();
        let smooth = (0..at.len()).map(|i| c.is_smooth(i)).collect();
        Self {
            at,
            smooth,
            shape: c.shape(),
        }
    }

    /// Adds a smooth point at `v` between the ends, unless another is nearer along the graph
    /// than [`GAP`]. Returns where it went.
    fn add(&mut self, [x, y]: [f32; 2]) -> Option<usize> {
        let i = self.at.iter().position(|p| p[0] > x)?;
        let (left, right) = (self.at[i - 1][0], self.at[i][0]);
        if x - left < GAP || right - x < GAP {
            return None;
        }
        self.at.insert(i, [x, y]);
        self.smooth.insert(i, true);
        Some(i)
    }

    /// Moves point `i` to `v`: an end only up and down, any other between its neighbours.
    fn put(&mut self, i: usize, [x, y]: [f32; 2]) {
        let last = self.at.len() - 1;
        let x = if i == 0 || i == last {
            self.at[i][0]
        } else {
            x.max(self.at[i - 1][0] + GAP).min(self.at[i + 1][0] - GAP)
        };
        self.at[i] = [x, y];
    }

    /// Whether point `i` lies between the ends, so it may be taken away or turned.
    fn between(&self, i: usize) -> bool {
        i > 0 && i + 1 < self.at.len()
    }

    /// Takes point `i` away, unless it is an end.
    fn remove(&mut self, i: usize) {
        if self.between(i) {
            self.at.remove(i);
            self.smooth.remove(i);
        }
    }

    /// Turns point `i` from smooth to sharp or back, unless it is an end (an end's kind changes
    /// nothing). Returns whether it turned.
    fn turn(&mut self, i: usize) -> bool {
        let between = self.between(i);
        if between {
            self.smooth[i] = !self.smooth[i];
        }
        between
    }

    /// These points upside down, `y` to `1 - y`, their kinds and settings kept (decision 0048,
    /// section 2).
    #[must_use]
    pub fn inverted(&self) -> Self {
        let at = self.at.iter().map(|&[x, y]| [x, 1.0 - y]).collect();
        Self { at, ..self.clone() }
    }

    /// These points as a curve.
    pub fn curve(&self) -> Option<Curve> {
        let c = Curve::with_kinds(self.at.clone(), self.smooth.clone());
        c.and_then(|c| c.shaped(self.shape)).ok()
    }

    /// These points as `brush.set` takes a curve: in thousandths, smooth, with the sharp ones
    /// listed even when there are none, so a preset's list never carries over, and the
    /// settings when they are not the defaults.
    pub fn value(&self) -> Value {
        let round = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
        let at: Vec<[f64; 2]> = (self.at.iter())
            .map(|&[x, y]| [round(x), round(y)])
            .collect();
        let sharp: Vec<usize> = (0..self.at.len()).filter(|&i| !self.smooth[i]).collect();
        let mut v = json!({ "points": at, "smooth": true, "sharp": sharp });
        let s = self.shape;
        if s != Shape::default() {
            v["steps"] = s.steps.into();
            v["step_strength"] = round(s.step_strength).into();
            v["tile"] = s.tile.into();
        }
        v
    }
}

/// A curve the pen edits on a graph.
pub struct Edit<'a> {
    /// What `ui.describe` calls the graph, which also keeps the pen's drag between frames.
    pub name: &'a str,
    /// What `ui.describe` calls each point, numbered from 1 at the left.
    pub point: &'a str,
    /// The points as they are.
    pub points: Points,
    /// The call that sets the curve to some points.
    pub step: &'a dyn Fn(&Points) -> Step,
    /// The lines drawn under the points, each sampled across the graph ([`across`]): the last
    /// one bright, the curve itself, any before it dim.
    pub lines: &'a dyn Fn(&Points) -> Vec<Vec<[f32; 2]>>,
    /// A pressure graph: a point dragged within [`SNAP`] of the top lands on it.
    pub snap: bool,
}

/// `f` sampled across a graph, from 0 at the left to 1 at the right.
pub fn across(f: impl Fn(f32) -> f32) -> Vec<[f32; 2]> {
    (0..=STEPS)
        .map(|k| {
            let x = k as f32 / STEPS as f32;
            [x, f(x)]
        })
        .collect()
}

/// The graph's area: 0 to 1 rightwards and upwards; with `snap`, the top takes what is near it.
struct Graph {
    rect: Rect,
    snap: bool,
}

impl Graph {
    fn screen(&self, [x, y]: [f32; 2]) -> Pos2 {
        let r = self.rect;
        pos2(r.left() + x * r.width(), r.bottom() - y * r.height())
    }

    fn value(&self, p: Pos2) -> [f32; 2] {
        let r = self.rect;
        let x = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0);
        let y = ((r.bottom() - p.y) / r.height()).clamp(0.0, 1.0);
        let y = if self.snap && y >= 1.0 - SNAP { 1.0 } else { y };
        [x, y]
    }

    /// The point nearest `p` within the pen's reach.
    fn near(&self, points: &Points, p: Pos2) -> Option<usize> {
        (points.at.iter().enumerate())
            .map(|(i, &v)| (i, self.screen(v).distance(p)))
            .filter(|&(_, d)| d <= GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

/// The point the pen takes when it presses at `p`: the one in reach, or a new one there.
fn take(graph: &Graph, mut points: Points, p: Pos2) -> Option<Held> {
    let index = graph
        .near(&points, p)
        .or_else(|| points.add(graph.value(p)))?;
    Some(Held {
        points,
        index,
        off: false,
    })
}

/// Draws `e` as a graph the pen edits, ending its row, and records it. Its points lie the pen's
/// reach in from its edge, so the pen may take even a corner's point anywhere in a pen target
/// on the graph (decision 0025, rule 12). Returns what it asked to run.
pub fn graph(ui: &mut egui::Ui, e: &Edit, rec: &mut Recorder) -> Option<Step> {
    let width = ui.available_width().min(WIDEST);
    let size = vec2(width, (width * ASPECT).round());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let graph = Graph {
        rect: rect.shrink(GRAB),
        snap: e.snap,
    };
    let (held, asked) = pen(ui, &response, &graph, e);
    let points = held.as_ref().map_or_else(|| e.points.clone(), Held::result);
    let lit = held.filter(|h| !h.off).map(|h| h.index);
    paint(ui, rect, &graph, &(e.lines)(&points), (&points, lit));
    record(rec, (rect, &graph), e, &points);
    ui.end_row();
    asked
}

/// What the pen does on `e`'s graph this frame: the points it is dragging, kept between frames,
/// and what it asked to run.
fn pen(
    ui: &egui::Ui,
    response: &egui::Response,
    graph: &Graph,
    e: &Edit,
) -> (Option<Held>, Option<Step>) {
    let id = egui::Id::new(("curve graph", e.name));
    let mut held: Option<Held> = ui.data(|d| d.get_temp(id));
    let mut asked = None;
    let at = response.interact_pointer_pos();
    if response.drag_started() {
        let from = ui.input(|i| i.pointer.press_origin()).or(at);
        held = from.and_then(|p| take(graph, e.points.clone(), p));
    }
    if let (Some(h), Some(p)) = (&mut held, at.filter(|_| response.dragged())) {
        // Off the graph, past the pen's reach of its edge: a point between the ends goes, and
        // brought back on it turns.
        let off = !response.rect.expand(GRAB).contains(p) && h.points.between(h.index);
        if h.off && !off {
            h.points.turn(h.index);
        }
        h.off = off;
        if !h.off {
            h.points.put(h.index, graph.value(p));
        }
    }
    if response.drag_stopped() {
        asked = held.take().map(|h| (e.step)(&h.result()));
    } else if let Some(p) = at.filter(|_| response.clicked()) {
        let mut points = e.points.clone();
        let changed = match graph.near(&points, p) {
            Some(i) => points.turn(i),
            None => points.add(graph.value(p)).is_some(),
        };
        asked = changed.then(|| (e.step)(&points));
    }
    ui.data_mut(|d| match &held {
        Some(h) => {
            d.insert_temp(id, h.clone());
        }
        None => d.remove::<Held>(id),
    });
    (held, asked)
}

/// The graph: quarter lines, `lines` (the last bright, any before it dim) and the points, the
/// one the pen holds lit, smooth ones round and sharp ones square.
fn paint(
    ui: &egui::Ui,
    rect: Rect,
    graph: &Graph,
    lines: &[Vec<[f32; 2]>],
    (points, held): (&Points, Option<usize>),
) {
    let t = theme(ui.ctx());
    let c = &t.colors;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.spacing.radius, c.control);
    let g = graph.rect;
    let grid = Stroke::new(1.0, c.edge);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let x = g.left() + f * g.width();
        let y = g.top() + f * g.height();
        painter.line_segment([pos2(x, g.top()), pos2(x, g.bottom())], grid);
        painter.line_segment([pos2(g.left(), y), pos2(g.right(), y)], grid);
    }
    painter.rect_stroke(g, 0.0, grid, StrokeKind::Middle);
    for (i, line) in lines.iter().enumerate() {
        let stroke = if i + 1 == lines.len() {
            Stroke::new(2.0, c.lit)
        } else {
            Stroke::new(1.5, c.text_dim)
        };
        let at = line.iter().map(|&v| graph.screen(v)).collect();
        painter.add(Line::line(at, stroke));
    }
    for (i, &v) in points.at.iter().enumerate() {
        let fill = if held == Some(i) { c.lit } else { c.text };
        let (at, edge) = (graph.screen(v), Stroke::new(1.0, c.edge));
        if points.smooth[i] {
            painter.circle_filled(at, HANDLE, fill);
            painter.circle_stroke(at, HANDLE, edge);
        } else {
            let square = Rect::from_center_size(at, vec2(2.0 * HANDLE, 2.0 * HANDLE));
            painter.rect_filled(square, 0.0, fill);
            painter.rect_stroke(square, 0.0, edge, StrokeKind::Middle);
        }
    }
}

/// Records the graph, and each point with the square the pen may take it in, which lies on the
/// graph (see [`graph`]), as sliders that run `e`'s command with the curve as it is.
fn record(rec: &mut Recorder, (rect, graph): (Rect, &Graph), e: &Edit, points: &Points) {
    let step = (e.step)(points);
    let curve = Widget::runs(e.name, Role::Slider, step.clone(), None);
    rec.widget(curve, rect);
    for (i, &v) in points.at.iter().enumerate() {
        let reach = Rect::from_center_size(graph.screen(v), vec2(2.0 * GRAB, 2.0 * GRAB));
        let label = format!("{} {}", e.point, i + 1);
        rec.widget(Widget::runs(label, Role::Slider, step.clone(), None), reach);
    }
}
