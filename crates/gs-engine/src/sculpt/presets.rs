//! The shipped brush presets (`assets/brushes/`), built into the engine so every session has
//! the same brushes, and finding one by name.

use std::borrow::Cow;
use std::sync::OnceLock;

use gs_core::BrushPreset;
use gs_protocol::CommandError;

use crate::errors;

/// Every shipped preset file.
const FILES: [&str; 24] = [
    include_str!("../../../../assets/brushes/clay-build.toml"),
    include_str!("../../../../assets/brushes/clay-line.toml"),
    include_str!("../../../../assets/brushes/clay.toml"),
    include_str!("../../../../assets/brushes/clip-curve.toml"),
    include_str!("../../../../assets/brushes/draw.toml"),
    include_str!("../../../../assets/brushes/flat-polish.toml"),
    include_str!("../../../../assets/brushes/inflate.toml"),
    include_str!("../../../../assets/brushes/mask-pen.toml"),
    include_str!("../../../../assets/brushes/move-connected.toml"),
    include_str!("../../../../assets/brushes/move-elastic.toml"),
    include_str!("../../../../assets/brushes/move.toml"),
    include_str!("../../../../assets/brushes/paint-blend.toml"),
    include_str!("../../../../assets/brushes/paint-draw.toml"),
    include_str!("../../../../assets/brushes/pinch.toml"),
    include_str!("../../../../assets/brushes/pull-out-along-normal.toml"),
    include_str!("../../../../assets/brushes/pull-out.toml"),
    include_str!("../../../../assets/brushes/select-rect.toml"),
    include_str!("../../../../assets/brushes/sharp-crease.toml"),
    include_str!("../../../../assets/brushes/smooth-polish.toml"),
    include_str!("../../../../assets/brushes/smooth-strong.toml"),
    include_str!("../../../../assets/brushes/smooth.toml"),
    include_str!("../../../../assets/brushes/trim.toml"),
    include_str!("../../../../assets/brushes/uv-density.toml"),
    include_str!("../../../../assets/brushes/uv-hint.toml"),
];

/// One shipped brush: its preset and the file it came from. (A stroke that carries the preset
/// it was recorded with sculpts with one made from that file instead, decision 0042.)
#[derive(Clone)]
pub(crate) struct Shipped {
    pub preset: BrushPreset,
    pub text: Cow<'static, str>,
}

impl Shipped {
    /// Whether the file itself names `key` in `table`, as Move names a flat `pressure.size` so
    /// that the session's size curve passes it by (decision 0036). Naming a default value does
    /// not change the preset or its hash, so only the file can tell.
    pub fn names(&self, table: &str, key: &str) -> bool {
        let file = toml::from_str::<toml::Table>(&self.text).ok();
        file.as_ref().and_then(|f| f.get(table)?.get(key)).is_some()
    }
}

/// Every shipped brush, by family (in `Family`'s order), then by name: the order `brush.list`
/// reports and every brush set in the window shows. A file that does not read is left out (the
/// gs-core tests read every one, so none is).
pub(crate) fn all() -> &'static [Shipped] {
    static ALL: OnceLock<Vec<Shipped>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<Shipped> = (FILES.iter())
            .filter_map(|&text| {
                let preset = BrushPreset::from_toml(text).ok()?;
                let text = Cow::Borrowed(text);
                Some(Shipped { preset, text })
            })
            .collect();
        all.sort_by(|a, b| {
            let (a, b) = (&a.preset, &b.preset);
            a.family.cmp(&b.family).then_with(|| a.name.cmp(&b.name))
        });
        all
    })
}

/// The brush called `name` (its display name in any case, or its id such as `brush.draw`).
/// `E_NOT_FOUND` with the closest names otherwise.
pub(crate) fn find(name: &str) -> Result<&'static Shipped, CommandError> {
    let all = all();
    let wanted = name.trim();
    all.iter()
        .find(|s| s.preset.name.eq_ignore_ascii_case(wanted) || s.preset.id == wanted)
        .ok_or_else(|| {
            let names = all.iter().map(|s| s.preset.name.as_str());
            CommandError {
                hint: Some("brush.list lists every brush".into()),
                ..errors::not_found("brush", name, names)
            }
        })
}
