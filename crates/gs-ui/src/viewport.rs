//! The 3D view's area: navigation (the gesture map in `navigate`, the corner figure and its Zoom
//! and Pan buttons in `figure`, in the corner on the pen side), the pivot dot while the view turns, and the pen's
//! cursor ring, and the gizmo of the gizmo tool (see `gizmo`), all colored by the theme.
//!
//! A click of the barrel button (or the right mouse button) that does not drag runs the
//! navigation setting `barrel_click`; with Alt held it picks the part under it instead. The tip on the view sculpts (see `Studio::pen`), unless it
//! pressed on empty canvas; the cursor ring shows the brush, and a quieter ring shows each
//! place symmetry repeats the dab. The brush's symmetry planes are drawn on the active part,
//! and a select outline being dragged with each place symmetry repeats it.
//!
//! The drawings over the view (the pivot, the symmetry planes, a select outline) are in
//! `overlays`; the cursor (its rings, its lines of text, the smoothing string) is in `cursor`.

mod cursor;
mod overlays;

use egui::{Sense, Stroke};
use gs_engine::{BrushState, Hand, MainView, Navigation};
use gs_protocol::Step;

use crate::figure::{area, figure, pads};
use crate::gizmo::{GizmoPrefs, Tool, draw as draw_gizmo};
use crate::navigate::{Context, Navigator};
use crate::record::Recorder;
use crate::theme::theme;

pub use cursor::{Cursor, Detail, Ring, symmetry_text};
use cursor::{draw_mirrored, draw_ring, draw_string};
pub use overlays::ActivePart;
use overlays::{PLANE_LINE, draw_outline, draw_pivot, draw_planes};

/// What the 3D view navigates: `main` as the engine last reported it, the navigation
/// settings, whether the pen's press landed on empty canvas, and the hand that holds the pen
/// (the corner figure sits on its side).
pub struct Sight<'a> {
    pub main: &'a MainView,
    pub settings: &'a Navigation,
    pub canvas: bool,
    pub hand: Hand,
    /// The gizmo tool and its settings.
    pub gizmo: (&'a Tool, GizmoPrefs),
    /// The brush whose symmetry planes show, and the active part they cross.
    pub planes: (&'a BrushState, Option<ActivePart>),
}

/// The 3D view in one frame: its rectangle in points, whether the barrel button (or the right
/// mouse button) clicked on it without dragging, and the view commands it asks for.
pub struct View {
    pub rect: egui::Rect,
    pub barrel_click: bool,
    pub steps: Vec<Step>,
}

/// The central area where the model shows through.
pub fn central(
    ui: &mut egui::Ui,
    nav: &mut Navigator,
    sight: &Sight,
    ring: &Ring,
    rec: &mut Recorder,
) -> View {
    let mut view = View {
        rect: egui::Rect::NOTHING,
        barrel_click: false,
        steps: Vec::new(),
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| {
            view.rect = ui.max_rect();
            rec.panel("viewport", "3D view", view.rect);
            let response = ui.allocate_rect(view.rect, Sense::click_and_drag());
            let cx = Context {
                rect: view.rect,
                ppp: ui.ctx().pixels_per_point(),
                main: sight.main,
                settings: sight.settings,
                canvas: sight.canvas,
                figure: area(view.rect, sight.hand),
                pads: pads(view.rect, sight.hand),
            };
            view.steps = nav.frame(ui, &response, &cx);
            // The brush hides while the view is dragged: no stroke lands then (0062, row 805).
            let hidden = Ring {
                brush: None,
                ..*ring
            };
            let ring = if nav.navigating() { &hidden } else { ring };
            // With Alt, a barrel click picks the part under it instead (`Studio::pen`).
            view.barrel_click = response.secondary_clicked() && !ui.input(|i| i.modifiers.alt);
            if let (brush, Some(part)) = sight.planes {
                draw_planes(ui, &cx, (brush, part), rec);
            }
            draw_outline(ui, &cx, ring, sight.planes, rec);
            let line = Stroke::new(PLANE_LINE, ring.tint(&theme(ui.ctx()).colors));
            crate::studio::draw_path(ui, cx.rect, cx.ppp, ring.cursor, line, rec);
            if sight.main.moving || nav.dragging().is_some() {
                draw_pivot(ui, &cx);
            }
            let (tool, prefs) = sight.gizmo;
            // A handle lights under a hovering pen while the brush shows (nothing covers it).
            let corner = view.rect.min * cx.ppp;
            let hover = (ring
                .cursor
                .pen
                .filter(|c| !c.contact && ring.brush.is_some()))
            .map(|c| [c.x as f32 - corner.x, c.y as f32 - corner.y]);
            let lit = draw_gizmo(ui, tool, (view.rect, &sight.main.camera), prefs, hover);
            if let Some(pen) = ring.cursor.pen {
                let gizmo = tool.on.then_some(prefs);
                draw_ring(ui, (&cx, pen), (ring, lit.as_ref(), gizmo), rec);
                if lit.is_none() {
                    draw_mirrored(ui, &cx, ring, rec);
                    if let Some(dab) = ring.cursor.string {
                        draw_string(ui, pen, dab, rec);
                    }
                }
            }
            let corner = (view.rect, sight.hand);
            let seen = (sight.main, nav.dragging_pad());
            view.steps.extend(figure(ui, corner, seen, rec));
        });
    view
}
