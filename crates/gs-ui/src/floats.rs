//! What floats over the 3D view (the header's dropdown panels, the quick menu, an open brush
//! family, the brush picker and the settings window) in a window that may be too short for it
//! (decision 0025, section 2): pieces taller than their room scroll in all of it, dragged with
//! the pen as a tray's are, and pieces that fit are drawn as they are.

use egui::ScrollArea;
use egui::scroll_area::{DragScroll, ScrollSource};

use crate::record::Recorder;

/// Pieces `add` draws, scrolling ([`scrolling`]) when taller than `room` points. Pieces that fit
/// are drawn as they are: egui's scroll area rounds its width to whole pixels, which narrows a
/// panel as wide as its pieces a wrapped row at a time. Whether they fit is known from the frame
/// before, as egui lays out what floats in the size it had then; the room comes from the window,
/// not from what floats.
pub fn scrolled<R>(
    ui: &mut egui::Ui,
    room: f32,
    rec: &mut Recorder,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder) -> R,
) -> R {
    let room = room.max(0.0);
    let scrolls = ui.id().with("scrolls");
    if !ui.data(|d| d.get_temp::<bool>(scrolls).unwrap_or_default()) {
        let top = ui.cursor().top();
        let inner = add(ui, rec);
        if ui.min_rect().bottom() - top > room {
            ui.data_mut(|d| d.insert_temp(scrolls, true));
            ui.ctx().request_repaint();
        }
        return inner;
    }
    let (inner, fits) = scrolling(ui, room, rec, add);
    if fits {
        ui.data_mut(|d| d.remove::<bool>(scrolls));
    }
    inner
}

/// Pieces `add` draws in a scroll area at most `room` points high, dragged with the pen as a
/// tray's are; what scrolled out of sight is not reported, where a tap would miss it. Returns
/// what `add` did and whether the pieces fit.
pub fn scrolling<R>(
    ui: &mut egui::Ui,
    room: f32,
    rec: &mut Recorder,
    add: impl FnOnce(&mut egui::Ui, &mut Recorder) -> R,
) -> (R, bool) {
    let source = ScrollSource {
        drag: DragScroll::Always,
        ..ScrollSource::default()
    };
    let scroll = ScrollArea::vertical().scroll_source(source);
    let shown = scroll
        .max_height(room)
        .min_scrolled_height(room)
        .show(ui, |ui| {
            rec.list(Some(ui.clip_rect()));
            add(ui, rec)
        });
    rec.list(None);
    (shown.inner, shown.content_size.y <= room)
}
