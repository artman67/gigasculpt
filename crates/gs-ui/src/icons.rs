//! Icons beside labels (decision 0025, section 13). Which control shows which icon is data,
//! `assets/layouts/icons.toml`; the icons are Lucide SVG files in `assets/icons/`. An SVG is
//! drawn by `resvg` into a white mask at the size the button's text has in pixels, kept per
//! size, and tinted with the color the button's label has, so it follows the theme and the on
//! state.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use egui::load::SizedTexture;
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use resvg::{tiny_skia, usvg};
use serde::Deserialize;

use crate::direction::Control;

/// Which control shows which icon.
const FILE: &str = include_str!("../../../assets/layouts/icons.toml");

/// The icons, by name: every SVG in `assets/icons/`, each named in `icons.toml`.
pub const ICONS: [(&str, &str); 17] = [
    (
        "arrow-left-right",
        include_str!("../../../assets/icons/arrow-left-right.svg"),
    ),
    ("box", include_str!("../../../assets/icons/box.svg")),
    (
        "chart-pie",
        include_str!("../../../assets/icons/chart-pie.svg"),
    ),
    (
        "chevron-left",
        include_str!("../../../assets/icons/chevron-left.svg"),
    ),
    (
        "chevron-right",
        include_str!("../../../assets/icons/chevron-right.svg"),
    ),
    ("focus", include_str!("../../../assets/icons/focus.svg")),
    (
        "grid-3x3",
        include_str!("../../../assets/icons/grid-3x3.svg"),
    ),
    ("import", include_str!("../../../assets/icons/import.svg")),
    (
        "library-big",
        include_str!("../../../assets/icons/library-big.svg"),
    ),
    ("lock", include_str!("../../../assets/icons/lock.svg")),
    ("move-3d", include_str!("../../../assets/icons/move-3d.svg")),
    ("pin", include_str!("../../../assets/icons/pin.svg")),
    ("scan", include_str!("../../../assets/icons/scan.svg")),
    ("search", include_str!("../../../assets/icons/search.svg")),
    ("send", include_str!("../../../assets/icons/send.svg")),
    ("sun", include_str!("../../../assets/icons/sun.svg")),
    ("x", include_str!("../../../assets/icons/x.svg")),
];

/// One line of `icons.toml`: a command and its arguments as a control writes them, or a header
/// options button by its id, and the icon it shows.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub command: Option<String>,
    #[serde(default)]
    pub args: toml::Table,
    pub options: Option<String>,
    pub icon: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    icon: Vec<Entry>,
}

/// The entries of `icons.toml`, or why it cannot be read.
pub fn entries() -> Result<&'static [Entry], &'static str> {
    static READ: OnceLock<Result<Vec<Entry>, String>> = OnceLock::new();
    let read = READ.get_or_init(|| {
        let file = toml::from_str::<File>(FILE).map_err(|e| format!("icons.toml: {e}"))?;
        Ok(file.icon)
    });
    read.as_deref().map_err(String::as_str)
}

/// The icon a control running `command` with `args` (as written) shows, if any.
pub fn of_step(command: &str, args: &toml::Table) -> Option<&'static str> {
    let entries = entries().ok()?;
    let found = entries
        .iter()
        .find(|e| e.command.as_deref() == Some(command) && e.args == *args);
    found.map(|e| e.icon.as_str())
}

/// The icon a button running `command` with the one argument `name = value` shows, if any:
/// for a button not in a control set (the Library's Close and Pin).
pub fn of_arg(command: &str, name: &str, value: impl Into<toml::Value>) -> Option<&'static str> {
    let args = toml::Table::from_iter([(name.to_string(), value.into())]);
    of_step(command, &args)
}

/// The icon control `c` shows, if any.
pub fn of(c: &Control) -> Option<&'static str> {
    of_step(c.command.as_deref()?, &c.args)
}

/// The icon the header's options button `id` shows, if any.
pub fn of_options(id: &str) -> Option<&'static str> {
    let entries = entries().ok()?;
    let found = entries.iter().find(|e| e.options.as_deref() == Some(id));
    found.map(|e| e.icon.as_str())
}

/// Icon `name` as a white mask `side` pixels square, premultiplied: white where the icon is
/// drawn, clear elsewhere. `None` for an icon not shipped or an SVG `resvg` cannot read.
pub fn mask(name: &str, side: u32) -> Option<ColorImage> {
    let (_, svg) = ICONS.iter().find(|(n, _)| *n == name)?;
    // Lucide strokes in the text's color; white, so a tint gives any color.
    let white = svg.replace("currentColor", "#ffffff");
    let tree = usvg::Tree::from_str(&white, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let scale = side as f32 / tree.size().width().max(tree.size().height());
    let fit = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, fit, &mut pixmap.as_mut());
    let size = [side as usize; 2];
    Some(ColorImage::from_rgba_premultiplied(size, pixmap.data()))
}

/// The masks drawn so far, by icon and size in pixels.
#[derive(Clone, Default)]
struct Drawn(BTreeMap<(&'static str, u32), TextureHandle>);

/// Icon `name` at the height of a button's text, tinted `color`, for a button to show beside
/// its label. Its mask is drawn once per size in pixels.
pub fn image(ui: &egui::Ui, name: &'static str, color: Color32) -> Option<egui::Image<'static>> {
    let side = ui.text_style_height(&egui::TextStyle::Button);
    let pixels = (side * ui.ctx().pixels_per_point()).ceil() as u32;
    let id = egui::Id::new("gigasculpt icons");
    let key = (name, pixels);
    let kept = ui.data_mut(|d| d.get_temp_mut_or_default::<Drawn>(id).0.get(&key).cloned());
    let texture = if let Some(texture) = kept {
        texture
    } else {
        let drawn = mask(name, pixels)?;
        let texture =
            (ui.ctx()).load_texture(format!("icon {name}"), drawn, TextureOptions::LINEAR);
        let keep = texture.clone();
        ui.data_mut(|d| d.get_temp_mut_or_default::<Drawn>(id).0.insert(key, keep));
        texture
    };
    let sized = SizedTexture::new(texture.id(), egui::vec2(side, side));
    Some(egui::Image::new(sized).tint(color))
}
