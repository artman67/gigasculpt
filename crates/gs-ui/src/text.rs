//! How wide a piece of text shows on one line, the measure a button, menu or row sizes itself
//! by so its label never wraps.

/// `text`'s width on one line in `style`, in points.
pub(crate) fn width(
    ui: &egui::Ui,
    text: impl Into<egui::WidgetText>,
    style: impl Into<egui::FontSelection>,
) -> f32 {
    let wrap = Some(egui::TextWrapMode::Extend);
    let galley = text.into().into_galley(ui, wrap, f32::INFINITY, style);
    galley.size().x
}
