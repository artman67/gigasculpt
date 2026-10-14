//! The interface's look as data (decision 0025, sections 6 and 13): every color, type size and
//! spacing, by role, from a theme file. The shipped themes are `assets/themes/` (dark, light and
//! high contrast); the user's own are read from their `themes` folder when the window starts
//! ([`Themes::load`]), and the window keeps the list. The theme shown is the layout state's
//! (`ui.layout` `theme`): whenever it changes the shell builds egui's style from it and keeps it
//! in egui's context ([`show`]), and owned code asks [`theme`] for a role, given the context it
//! draws in, instead of writing a color. Nothing here is shared between windows: each has its
//! own context and its own list, so two can show two themes.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
};
use serde::{Deserialize, Deserializer};

/// The interface's font: Go Regular (BSD-3-Clause, `assets/fonts/Go-LICENSE.txt`). egui's own
/// fonts are left out because their licenses are not on the project's list.
const FONT: &[u8] = include_bytes!("../../../assets/fonts/Go-Regular.ttf");

/// The shipped themes by name, in the order the picker steps through them. The first is the
/// default, and the others' type and spacing, when they leave them out, are its.
const SHIPPED: [(&str, &str); 3] = [
    ("dark", include_str!("../../../assets/themes/dark.toml")),
    ("light", include_str!("../../../assets/themes/light.toml")),
    (
        "high-contrast",
        include_str!("../../../assets/themes/high-contrast.toml"),
    ),
];

/// Colors, type sizes and spacing, with the theme's name and title.
#[derive(Clone, Debug)]
pub struct Theme {
    /// Its file's name without `.toml`.
    pub name: String,
    pub title: String,
    pub colors: Colors,
    pub sizes: Sizes,
    pub spacing: Spacing,
}

/// A theme file as written: every color, and type and spacing if they differ from the first
/// shipped theme's.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    title: String,
    colors: Colors,
    #[serde(rename = "type", default)]
    sizes: Option<Sizes>,
    #[serde(default)]
    spacing: Option<Spacing>,
}

/// The colors, by role. `assets/themes/dark.toml` says where each is used.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    #[serde(deserialize_with = "hex")]
    pub panel: Color32,
    #[serde(deserialize_with = "hex")]
    pub header: Color32,
    #[serde(deserialize_with = "hex")]
    pub control: Color32,
    #[serde(deserialize_with = "hex")]
    pub hover: Color32,
    #[serde(deserialize_with = "hex")]
    pub pressed: Color32,
    #[serde(deserialize_with = "hex")]
    pub lit: Color32,
    #[serde(deserialize_with = "hex")]
    pub on_lit: Color32,
    #[serde(deserialize_with = "hex")]
    pub text: Color32,
    #[serde(deserialize_with = "hex")]
    pub text_dim: Color32,
    #[serde(deserialize_with = "hex")]
    pub text_off: Color32,
    #[serde(deserialize_with = "hex")]
    pub edge: Color32,
    #[serde(deserialize_with = "hex")]
    pub backdrop: Color32,
    #[serde(deserialize_with = "hex")]
    pub smooth: Color32,
    #[serde(deserialize_with = "hex")]
    pub mask: Color32,
    #[serde(deserialize_with = "hex")]
    pub subtract: Color32,
    #[serde(deserialize_with = "hex")]
    pub axis_x: Color32,
    #[serde(deserialize_with = "hex")]
    pub axis_y: Color32,
    #[serde(deserialize_with = "hex")]
    pub axis_z: Color32,
    /// The axis figure's letters, on the axis colors. A theme may leave it out; see
    /// [`Colors::on_axis`].
    #[serde(default, deserialize_with = "some_hex")]
    on_axis: Option<Color32>,
    #[serde(deserialize_with = "hex")]
    pub problem: Color32,
    /// The status line's warnings (low disk space). A theme may leave it out; see
    /// [`Colors::warning`].
    #[serde(default, deserialize_with = "some_hex")]
    warning: Option<Color32>,
    #[serde(deserialize_with = "hex")]
    pub pen: Color32,
    #[serde(deserialize_with = "hex")]
    pub pen_down: Color32,
    #[serde(deserialize_with = "hex")]
    pub brush: Color32,
    #[serde(deserialize_with = "hex")]
    pub pivot: Color32,
    /// The cursor while Live Detail refines under it, and with a smooth brush. A theme may
    /// leave them out; see [`Colors::detail`].
    #[serde(default, deserialize_with = "some_hex")]
    detail: Option<Color32>,
    #[serde(default, deserialize_with = "some_hex")]
    detail_smooth: Option<Color32>,
    /// Live Boolean's tints (decision 0057, section 9). A theme may leave them out; see
    /// [`Colors::live`].
    #[serde(default, deserialize_with = "some_hex")]
    live_union: Option<Color32>,
    #[serde(default, deserialize_with = "some_hex")]
    live_difference: Option<Color32>,
    #[serde(default, deserialize_with = "some_hex")]
    live_intersect: Option<Color32>,
    #[serde(default, deserialize_with = "some_hex")]
    live_issue: Option<Color32>,
    #[serde(default, deserialize_with = "some_hex")]
    live_coplanar: Option<Color32>,
}

/// Type sizes, in points.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sizes {
    pub small: f32,
    pub body: f32,
    pub heading: f32,
    pub mono: f32,
}

/// Spacing, in points.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spacing {
    pub control: f32,
    pub button: [f32; 2],
    pub item: [f32; 2],
    pub section: f32,
    pub margin: f32,
    pub radius: f32,
    pub slider: f32,
}

/// The themes a window can show: the shipped ones, then the user's.
#[derive(Clone, Debug, Default)]
pub struct Themes {
    /// The user's, by name, as [`Themes::load`] read them. Each theme is read once and kept for
    /// the life of the program, so [`theme`] can hand out a plain reference.
    user: Vec<&'static Theme>,
}

/// The shipped themes, read once. They are part of the program, so a file that does not read is
/// a bug the tests catch.
fn shipped() -> &'static [&'static Theme] {
    static READ: OnceLock<Vec<&'static Theme>> = OnceLock::new();
    READ.get_or_init(|| {
        let mut read: Vec<&'static Theme> = Vec::new();
        for (name, text) in SHIPPED {
            let theme = Theme::read(name, text, read.first().copied())
                .unwrap_or_else(|e| panic!("assets/themes/{name}.toml: {e}"));
            read.push(Box::leak(Box::new(theme)));
        }
        read
    })
}

/// The first shipped theme: what a window shows until its layout state names another.
pub fn first() -> &'static Theme {
    shipped()[0]
}

/// Where egui's context keeps the theme shown in it.
fn shown_id() -> egui::Id {
    egui::Id::new("gigasculpt theme")
}

/// The theme the interface drawn in `ctx` draws with: the one last shown in it, or the first
/// before one is.
pub fn theme(ctx: &egui::Context) -> &'static Theme {
    shown(ctx).unwrap_or_else(first)
}

/// The theme last shown in `ctx`; `None` before one is.
pub fn shown(ctx: &egui::Context) -> Option<&'static Theme> {
    ctx.data(|d| d.get_temp(shown_id()))
}

/// Draws with `t` in `ctx` from now on; the shell sets egui's style to it as well.
pub fn show(ctx: &egui::Context, t: &'static Theme) {
    ctx.data_mut(|d| d.insert_temp(shown_id(), t));
}

impl Themes {
    /// Reads the user's themes, the `.toml` files in `dir`. Returns them with what was wrong: a
    /// file that does not read, or that has a shipped theme's name (so `dark` always means the
    /// shipped one), is left out; one whose text falls under the contrast floors is kept, and
    /// named.
    pub fn load(dir: &Path) -> (Themes, Vec<String>) {
        let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let mut files: Vec<_> = (entries.map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        files.sort();
        let (mut user, mut problems) = (Vec::new(), Vec::new());
        for path in files {
            let name =
                (path.file_stem()).map_or(String::new(), |s| s.to_string_lossy().into_owned());
            let shown = path.display();
            if shipped().iter().any(|t| t.name == name) {
                problems.push(format!(
                    "{shown}: {name} is a shipped theme; copy it under another name to change it"
                ));
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string());
            match text.and_then(|text| Theme::from_toml(&name, &text)) {
                Ok(theme) => {
                    let faint = theme.too_faint();
                    if !faint.is_empty() {
                        problems.push(format!("{shown}: {}", faint.join("; ")));
                    }
                    user.push(&*Box::leak(Box::new(theme)));
                }
                Err(e) => problems.push(format!("{shown}: {e}")),
            }
        }
        (Themes { user }, problems)
    }

    /// Every theme, in the order the picker steps through them: the shipped ones, then the
    /// user's by name.
    #[must_use]
    pub fn list(&self) -> Vec<&'static Theme> {
        shipped().iter().chain(&self.user).copied().collect()
    }

    /// The theme called `name`.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&'static Theme> {
        self.list().into_iter().find(|t| t.name == name)
    }
}

impl Theme {
    /// Reads the theme file `name`, its type and spacing the first shipped theme's where it
    /// leaves them out.
    ///
    /// # Errors
    /// The file's TOML or a value in it is not valid, or a role is missing or unknown.
    pub fn from_toml(name: &str, text: &str) -> Result<Theme, String> {
        Theme::read(name, text, Some(shipped()[0]))
    }

    /// [`Theme::from_toml`] with `base`'s type and spacing, or none.
    fn read(name: &str, text: &str, base: Option<&Theme>) -> Result<Theme, String> {
        let file: File = toml::from_str(text).map_err(|e| e.to_string())?;
        let sizes = (file.sizes.or_else(|| base.map(|b| b.sizes.clone()))).ok_or("no `type`")?;
        let spacing =
            (file.spacing.or_else(|| base.map(|b| b.spacing.clone()))).ok_or("no `spacing`")?;
        Ok(Theme {
            name: name.into(),
            title: file.title,
            colors: file.colors,
            sizes,
            spacing,
        })
    }

    /// Where the theme's text falls under WCAG 2's contrast floors (decision 0025, section 6):
    /// 4.5 to 1 for text on what it sits on, 3 for `text_off`, which WCAG exempts. Each named
    /// with its ratio; empty when every pair clears.
    #[must_use]
    pub fn too_faint(&self) -> Vec<String> {
        let c = &self.colors;
        let pairs = [
            ("text on panel", c.text, c.panel, 4.5),
            ("text on header", c.text, c.header, 4.5),
            ("text on control", c.text, c.control, 4.5),
            ("text on hover", c.text, c.hover, 4.5),
            ("text on pressed", c.text, c.pressed, 4.5),
            ("text_dim on panel", c.text_dim, c.panel, 4.5),
            ("text_dim on header", c.text_dim, c.header, 4.5),
            ("text_dim on control", c.text_dim, c.control, 4.5),
            ("on_lit on lit", c.on_lit, c.lit, 4.5),
            ("on_axis on axis_x", c.on_axis(), c.axis_x, 4.5),
            ("on_axis on axis_y", c.on_axis(), c.axis_y, 4.5),
            ("on_axis on axis_z", c.on_axis(), c.axis_z, 4.5),
            ("problem on panel", c.problem, c.panel, 4.5),
            ("warning on panel", c.warning(), c.panel, 4.5),
            ("text_off on panel", c.text_off, c.panel, 3.0),
        ];
        (pairs.into_iter())
            .filter_map(|(name, fg, bg, least)| {
                let ratio = contrast(fg, bg);
                (ratio < least).then(|| format!("{name} is {ratio:.2} to 1, under {least}"))
            })
            .collect()
    }

    /// Sets `style` to this theme: sizes for a pen on a 4K monitor at 1.25 scale (16-point text,
    /// controls at least 32 points tall), and the colors by role.
    pub fn apply(&self, style: &mut egui::Style) {
        let (s, p) = (&self.sizes, &self.spacing);
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(s.small)),
            (TextStyle::Body, FontId::proportional(s.body)),
            (TextStyle::Button, FontId::proportional(s.body)),
            (TextStyle::Heading, FontId::proportional(s.heading)),
            (TextStyle::Monospace, FontId::monospace(s.mono)),
        ]
        .into();
        style.spacing.interact_size.y = p.control;
        style.spacing.button_padding = p.button.into();
        style.spacing.item_spacing = p.item.into();
        style.spacing.slider_width = p.slider;
        // A pen drag on text scrolls its panel or reaches the control under it; it never selects.
        style.interaction.selectable_labels = false;
        // No fades, slides or blinking: the interface answers at once, and the same input always
        // draws the same frame.
        style.animation_time = 0.0;
        // A panel that scrolls shows its bar beside its controls, not over them, in theme colors
        // and whether or not the pointer hovers (decision 0025, rule 11). Its handle takes the
        // text color, so it reads at least 3 to 1 on its `edge` track (decision 0062).
        style.spacing.scroll = egui::style::ScrollStyle {
            foreground_color: true,
            ..egui::style::ScrollStyle::solid()
        };
        self.colors.apply(&mut style.visuals, corners(p.radius));
    }

    /// The axis figure's color for scene axis `axis` (0, 1, 2 for X, Y, Z).
    #[must_use]
    pub fn axis(&self, axis: usize) -> Color32 {
        let c = &self.colors;
        [c.axis_x, c.axis_y, c.axis_z][axis.min(2)]
    }
}

impl Colors {
    /// The axis figure's letters: the theme's `on_axis`, or for a theme without one, whichever
    /// of `on_lit` and `text` reads better on the axis color it reads worst on.
    #[must_use]
    pub fn on_axis(&self) -> Color32 {
        let axes = [self.axis_x, self.axis_y, self.axis_z];
        let worst = |c: Color32| (axes.iter()).fold(f32::MAX, |w, a| w.min(contrast(c, *a)));
        let better = if worst(self.on_lit) >= worst(self.text) {
            self.on_lit
        } else {
            self.text
        };
        self.on_axis.unwrap_or(better)
    }

    /// The status line's warnings (decision 0016, section 17, item 9): the theme's `warning`, or
    /// `problem` for a theme without one.
    #[must_use]
    pub fn warning(&self) -> Color32 {
        self.warning.unwrap_or(self.problem)
    }

    /// Live Boolean's color for `tint`: an operation's walls and outline, or an issue mark. The
    /// theme's `live_*`, or for a theme without one, its Y axis, `subtract`, its Z axis,
    /// `problem` and the warning.
    #[must_use]
    pub fn live(&self, tint: gs_engine::Tint) -> Color32 {
        use gs_engine::Tint as T;
        match tint {
            T::Union => self.live_union.unwrap_or(self.axis_y),
            T::Difference => self.live_difference.unwrap_or(self.subtract),
            T::Intersect => self.live_intersect.unwrap_or(self.axis_z),
            T::Issue => self.live_issue.unwrap_or(self.problem),
            T::Coplanar => self.live_coplanar.unwrap_or_else(|| self.warning()),
        }
    }

    /// The cursor while Live Detail refines under it (`geometry.live-detail-mode`): the theme's
    /// `detail`, or purple; with a `smooth` brush, which thins as it smooths, `detail_smooth`,
    /// or orange.
    #[must_use]
    pub fn detail(&self, smooth: bool) -> Color32 {
        if smooth {
            (self.detail_smooth).unwrap_or(Color32::from_rgb(0xff, 0xa0, 0x3c))
        } else {
            (self.detail).unwrap_or(Color32::from_rgb(0xb4, 0x8c, 0xff))
        }
    }

    /// egui's visuals from the roles. A lit toggle and a waiting control set their own fill and
    /// text (`pieces::control`), since egui paints a selected button and selected text alike.
    fn apply(&self, v: &mut egui::Visuals, radius: CornerRadius) {
        // egui's own dark or light visuals underneath, by the panels' lightness (L* under 50).
        let dark = luminance(self.panel) < 0.184;
        *v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.text_cursor.blink = false;
        v.override_text_color = None;
        v.weak_text_color = Some(self.text_dim);
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.edge);
        v.window_corner_radius = radius;
        v.menu_corner_radius = radius;
        v.faint_bg_color = self.header;
        v.extreme_bg_color = self.edge;
        v.code_bg_color = self.control;
        v.warn_fg_color = self.mask;
        v.error_fg_color = self.problem;
        v.hyperlink_color = self.smooth;
        v.selection.bg_fill = self.pressed;
        v.selection.stroke = Stroke::new(1.0, self.text);
        // Waiting controls draw their own text_off; nothing is faded on top of it.
        v.disabled_alpha = 1.0;
        let w = &mut v.widgets;
        let states = [
            (&mut w.noninteractive, self.panel),
            (&mut w.inactive, self.control),
            (&mut w.hovered, self.hover),
            (&mut w.active, self.pressed),
            (&mut w.open, self.pressed),
        ];
        for (state, fill) in states {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::NONE;
            state.fg_stroke = Stroke::new(1.0, self.text);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
        // Lines between regions, and separators.
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.edge);
    }
}

/// Corners `radius` points round.
fn corners(radius: f32) -> CornerRadius {
    CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8)
}

/// egui fonts using [`FONT`] for everything.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::empty();
    fonts
        .font_data
        .insert("go".into(), Arc::new(FontData::from_static(FONT)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts.families.insert(family, vec!["go".into()]);
    }
    fonts
}

/// WCAG 2's contrast ratio of two opaque colors.
#[must_use]
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (l1, l2) = (luminance(a), luminance(b));
    (l1.max(l2) + 0.05) / (l1.min(l2) + 0.05)
}

/// WCAG 2's relative luminance of an opaque sRGB color.
fn luminance(c: Color32) -> f32 {
    let linear = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            libm::powf((v + 0.055) / 1.055, 2.4)
        }
    };
    0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
}

/// A color written `#rrggbb` or `#rrggbbaa`, alpha not premultiplied.
fn hex<'de, D: Deserializer<'de>>(d: D) -> Result<Color32, D::Error> {
    let text = String::deserialize(d)?;
    parse(&text)
        .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not #rrggbb or #rrggbbaa")))
}

/// [`hex`] for a role a theme may leave out.
fn some_hex<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Color32>, D::Error> {
    hex(d).map(Some)
}

/// [`hex`]'s parser.
fn parse(text: &str) -> Option<Color32> {
    let digits = text.strip_prefix('#')?;
    if !matches!(digits.len(), 6 | 8) || !digits.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    let alpha = if digits.len() == 8 { byte(6)? } else { 255 };
    Some(Color32::from_rgba_unmultiplied(
        byte(0)?,
        byte(2)?,
        byte(4)?,
        alpha,
    ))
}
