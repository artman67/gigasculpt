//! One frame of the interface as a draw list: each region of the window is a function, and
//! [`REGIONS`] draws them in order. The order is egui's: the status line first so it spans the
//! window, then the panels docked on their edges (trays, docked reference, Library, the header
//! along the view's top), then the 3D view in what is left, then everything that floats over it
//! (the header's open menu or view button's options among them). Before them all, the recovery
//! dialog takes its keys, before anything that draws can see them. What a region asked to run
//! joins [`Frame::asked`] in the same order. A new region is one function and one line here.

use gs_engine::MainView;
use gs_protocol::{Catalog, Step};

use super::{Shell, listed, now, wheel_slots};
use crate::arrange::{Asked, panels, reference};
use crate::descriptions;
use crate::direction::ReferenceSpec;
use crate::gizmo::Tool;
use crate::held::Held;
use crate::layout::Layout;
use crate::library::Library;
use crate::navigate::Navigator;
use crate::palette::Palette;
use crate::pieces::{Dragging, Now};
use crate::quick;
use crate::record::Recorder;
use crate::recovery::Recovery;
use crate::reference::{RefPanel, Sight as RefSight};
use crate::settings;
use crate::status::{Status, status_bar};
use crate::viewport::{ActivePart, Cursor, Detail, Ring, Sight, central};
use crate::wheel::Wheel;

/// A region of the window: draws itself and adds what it asked to run to the frame.
type Region = fn(&mut Frame, &mut egui::Ui);

/// The regions, in the order they are drawn.
pub(super) const REGIONS: [Region; 18] = [
    recovery_keys,
    status,
    trays,
    docked_reference,
    library,
    header,
    view,
    floating_reference,
    adjust,
    header_menu,
    settings_window,
    quick_menu,
    brush_family,
    barrel_click,
    wheel,
    palette,
    recovery,
    description,
];

/// What the regions of one frame share: what the pieces see, the parts of the shell they
/// change, and what one region leaves for a later one.
pub(super) struct Frame<'a> {
    now: Now<'a>,
    rec: Recorder,
    /// What the regions asked to run, in order.
    asked: Vec<Step>,
    dragging: &'a mut Dragging,
    shell: Parts<'a>,
    /// The pen is over the 3D view: the brush shows at it.
    brush_shown: bool,
    /// The trays' fold and the width a tray was dragged to.
    trays: Asked,
    /// Where the Reference View sits, from the room the trays left.
    reference: ReferenceSpec,
    /// The 3D view's area.
    view: egui::Rect,
    barrel_click: bool,
}

/// The parts of the shell the regions read or change.
struct Parts<'a> {
    status: &'a Status,
    catalog: &'a Catalog,
    main: &'a MainView,
    canvas: bool,
    cursor: &'a Cursor,
    active_part: Option<ActivePart>,
    gizmo: &'a Tool,
    settings: &'a settings::SettingsWindow,
    nav: &'a mut Navigator,
    reference: &'a mut RefPanel,
    library: &'a mut Library,
    quick: &'a mut Option<quick::Open>,
    wheel: &'a mut Wheel,
    palette: &'a mut Palette,
    recovery: &'a mut Recovery,
    menus: &'a mut crate::header::Menus,
    adjust: &'a mut crate::adjust::Adjust,
}

/// What a frame leaves the shell when its regions are drawn.
pub(super) struct Drawn {
    pub asked: Vec<Step>,
    pub rec: Recorder,
    pub fold: Vec<String>,
    pub width: Option<(String, f32)>,
    pub view: egui::Rect,
}

impl Shell {
    /// A frame showing the session's state as `shown`, with `held` keys, recording in `rec`.
    pub(super) fn frame<'a>(
        &'a mut self,
        (shown, held): (&'a gs_engine::SessionState, Option<Held>),
        brush_shown: bool,
        rec: Recorder,
    ) -> Frame<'a> {
        let Shell {
            directions,
            layout,
            keymap,
            looks,
            materials,
            themes,
            parts,
            applies,
            preset,
            copied,
            gizmo,
            status,
            catalog,
            main,
            canvas,
            cursor,
            active_part,
            settings,
            nav,
            reference,
            library,
            quick,
            wheel,
            palette,
            recovery,
            dragging,
            menus,
            adjust,
            ..
        } = self;
        let mut now = now(
            directions,
            &layout.layout,
            keymap,
            (looks, themes),
            parts,
            shown,
            held,
        );
        now.applies = Some(applies);
        now.materials = materials;
        now.preset = preset.as_ref();
        now.copied = copied.as_ref();
        now.gizmo = gizmo.drawn();
        let shell = Parts {
            status,
            catalog,
            main,
            canvas: *canvas,
            cursor,
            active_part: *active_part,
            gizmo,
            settings,
            nav,
            reference,
            library,
            quick,
            wheel,
            palette,
            recovery,
            menus,
            adjust,
        };
        Frame {
            now,
            rec,
            asked: Vec::new(),
            dragging,
            shell,
            brush_shown,
            trays: Asked::default(),
            reference: ReferenceSpec::default(),
            view: egui::Rect::NOTHING,
            barrel_click: false,
        }
    }
}

impl Frame<'_> {
    /// Ends the frame, leaving what the shell keeps.
    pub(super) fn end(self) -> Drawn {
        Drawn {
            asked: self.asked,
            rec: self.rec,
            fold: self.trays.fold,
            width: self.trays.width,
            view: self.view,
        }
    }
}

/// Where the Reference View sits, by `spec`, and how it navigates, by `layout`.
fn ref_sight<'a>(spec: &'a ReferenceSpec, layout: &'a Layout) -> RefSight<'a> {
    RefSight {
        spec,
        settings: &layout.navigation,
        hand: layout.hand,
    }
}

/// The status line along the window's foot: the brush, symmetry and the held keys, then, as
/// no state may be invisible, a locked camera and the navigation in progress.
fn status(f: &mut Frame, ui: &mut egui::Ui) {
    let mut line = super::sculpt_line(&f.now);
    line.splice(0..0, f.shell.status.prompt.clone());
    if f.shell.main.locked {
        line.push("Camera locked".into());
    }
    if f.now.layout.minimal {
        line.push("Panels hidden".into());
    }
    line.extend(f.shell.cursor.detail.and_then(Detail::status));
    if let Some(row) = f.shell.nav.dragging() {
        line.push(format!("{row:?}"));
    }
    let (state, keymap) = (&f.now.state, f.now.keymap);
    let asked = status_bar(ui, f.shell.status, state, &line, keymap, &mut f.rec);
    f.asked.extend(asked);
}

/// The layout's trays, docked on their edges.
fn trays(f: &mut Frame, ui: &mut egui::Ui) {
    let Some(d) = f.now.direction else {
        return;
    };
    let mut trays = panels(ui, d, &f.now, f.dragging, &mut f.rec);
    // A control marked `adjust` passes the adjust panel's door (decision 0060 as amended for S1).
    let marked = |s: &Step| f.now.directions.controls.adjusts(s);
    let (marked, rest): (Vec<Step>, Vec<Step>) = trays.steps.drain(..).partition(marked);
    f.asked.extend(f.shell.adjust.offer(marked));
    f.asked.extend(rest);
    f.trays = trays;
}

/// The Reference View, when it is docked beside the 3D view.
fn docked_reference(f: &mut Frame, ui: &mut egui::Ui) {
    f.reference = reference(&f.now, ui.available_width());
    let sight = ref_sight(&f.reference, f.now.layout);
    f.asked
        .extend(f.shell.reference.docked(ui, &sight, &f.now, &mut f.rec));
}

/// The Library drawer.
fn library(f: &mut Frame, ui: &mut egui::Ui) {
    f.asked
        .extend(f.shell.library.drawer(ui, &f.now, &mut f.rec));
}

/// The header bar along the top of the room left for the 3D view, so the view's picture starts
/// under it.
fn header(f: &mut Frame, ui: &mut egui::Ui) {
    let asked = crate::header::bar(ui, &f.now, f.shell.menus, f.dragging, &mut f.rec);
    f.asked.extend(asked);
}

/// The 3D view in the room left, with the brush ring at the pen.
fn view(f: &mut Frame, ui: &mut egui::Ui) {
    let ring = Ring {
        cursor: f.shell.cursor,
        brush: f.brush_shown.then_some(f.now.brush),
        held: f.now.held.as_ref(),
    };
    let layout = f.now.layout;
    let sight = Sight {
        main: f.shell.main,
        settings: &layout.navigation,
        canvas: f.shell.canvas,
        hand: layout.hand,
        gizmo: (f.shell.gizmo, layout.gizmo),
        planes: (f.now.brush, f.shell.active_part),
    };
    let view = central(ui, f.shell.nav, &sight, &ring, &mut f.rec);
    f.view = view.rect;
    f.barrel_click = view.barrel_click;
    f.asked.extend(view.steps);
}

/// The Reference View, when it floats over the 3D view.
fn floating_reference(f: &mut Frame, ui: &mut egui::Ui) {
    let sight = ref_sight(&f.reference, f.now.layout);
    let asked = (f.shell.reference).floating(ui, (&sight, f.view), &f.now, &mut f.rec);
    f.asked.extend(asked);
}

/// The adjust panel, in the 3D view's lower corner away from the pen (decision 0060, section
/// 5).
fn adjust(f: &mut Frame, ui: &mut egui::Ui) {
    let asked = (f.shell.adjust).show(ui, f.view, &f.now, &mut f.rec);
    f.asked.extend(asked.steps);
    f.trays.fold.extend(asked.fold);
}

/// The header's open menu under its title, and its submenu, or a view button's options under
/// it, over the 3D view.
fn header_menu(f: &mut Frame, ui: &mut egui::Ui) {
    let room = f.view;
    let asked = crate::header::menu(ui, &f.now, f.shell.menus, f.dragging, &mut f.rec, room);
    f.asked.extend(f.shell.adjust.offer(asked));
}

/// The settings window, at the 3D view's corner, and Reset's question.
fn settings_window(f: &mut Frame, ui: &mut egui::Ui) {
    let opened = (f.shell.settings, f.view);
    let asked = settings::window(ui, opened, &f.now, f.dragging, &mut f.rec);
    f.asked.extend(asked);
}

/// The quick menu, where it was opened, inside the 3D view.
fn quick_menu(f: &mut Frame, ui: &mut egui::Ui) {
    let asked = quick::menu(ui, (f.shell.quick, f.view), &f.now, f.dragging, &mut f.rec);
    f.asked.extend(asked);
}

/// The brush family opened from the shelf, at the pointer.
fn brush_family(f: &mut Frame, ui: &mut egui::Ui) {
    f.asked
        .extend(crate::family::window(ui, &f.now, &mut f.rec));
}

/// What a barrel click on the 3D view without a drag asks for, by the navigation setting.
fn barrel_click(f: &mut Frame, _: &mut egui::Ui) {
    use crate::record::with_args;
    use gs_engine::BarrelClick;
    if !f.barrel_click {
        return;
    }
    let asked = match f.now.layout.navigation.barrel_click {
        BarrelClick::Wheel => {
            let open = serde_json::json!({ "open": f.shell.wheel.at.is_none() });
            Some(with_args("ui.wheel", open))
        }
        BarrelClick::Palette => Some(with_args("ui.palette", serde_json::json!({}))),
        BarrelClick::Quick => {
            let open = serde_json::json!({ "open": f.shell.quick.is_none() });
            Some(with_args("ui.quick", open))
        }
        BarrelClick::None => None,
    };
    f.asked.extend(asked);
}

/// The brush wheel, while it is open.
fn wheel(f: &mut Frame, ui: &mut egui::Ui) {
    let (slots, radius) = wheel_slots(&f.now);
    let asked = (f.shell.wheel).show(ui, &slots, radius, &f.now, &mut f.rec);
    f.asked.extend(asked);
}

/// The command palette, while it is open.
fn palette(f: &mut Frame, ui: &mut egui::Ui) {
    let spec = (f.now.direction.map(|d| d.palette.clone())).unwrap_or_default();
    let listed = listed(&f.now);
    let asked = (f.shell.palette).show(ui, f.shell.catalog, &listed, &spec, &f.now, &mut f.rec);
    f.asked.extend(f.shell.adjust.offer(asked));
}

/// Enter for the offer to recover a crashed session, when the window gave it the press
/// (`escape`), taken before any region draws.
fn recovery_keys(f: &mut Frame, ui: &mut egui::Ui) {
    f.asked.extend(f.shell.recovery.keys(ui));
}

/// The offer to recover a crashed session, asked once at start, or Discard's question.
fn recovery(f: &mut Frame, ui: &mut egui::Ui) {
    f.asked.extend(f.shell.recovery.show(ui, &mut f.rec));
}

/// What the button under the pointer does, while Ctrl is held over it, over everything.
fn description(f: &mut Frame, ui: &mut egui::Ui) {
    descriptions::show(ui, f.shell.catalog, &mut f.rec);
}
