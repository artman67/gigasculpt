//! `brush.list` and `brush.describe`.

use gs_core::BrushPreset;
use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::presets::{self, Shipped};
use crate::{Command, Ctx, Example, register_command};

/// List every brush `brush.stroke` can use, by family (Build, Carve, Move, Smooth, Mask) and
/// then by name, with its kernel, stroke class and default strength. The window's brush
/// shelves, wheel and palette are built from this list. The brush the window's pen strokes
/// with and its settings are in `session.state`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushList {}

/// One brush, in brief.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct BrushInfo {
    /// Its name, as `brush.stroke` takes it.
    pub name: String,
    /// Its catalog id, which `brush.stroke` takes too.
    pub id: String,
    /// What it is for, as the window groups brushes: Build, Carve, Move, Smooth or Mask.
    pub family: String,
    /// The kernel that moves the surface: offset, plane, smooth, move, hook or attribute.
    pub kernel: String,
    /// How the pen's path becomes work: dab, drag, anchored, curve or gesture.
    pub stroke: String,
    /// Strength when a stroke leaves it out.
    pub strength: f32,
}

/// Every brush, by family and then by name.
#[derive(Serialize, JsonSchema)]
pub struct Brushes {
    pub brushes: Vec<BrushInfo>,
}

impl From<&BrushPreset> for BrushInfo {
    fn from(p: &BrushPreset) -> Self {
        BrushInfo {
            name: p.name.clone(),
            id: p.id.clone(),
            family: p.family.name().into(),
            kernel: p.kernel.name().into(),
            stroke: p.stroke.class().into(),
            strength: p.strength,
        }
    }
}

/// Every shipped brush in brief, by family and then by name: what `brush.list` reports.
pub fn brushes() -> Vec<BrushInfo> {
    presets::all().iter().map(|s| (&s.preset).into()).collect()
}

impl Command for BrushList {
    const NAME: &'static str = "brush.list";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = Brushes;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("every brush", Self {})]
    }

    fn run(self, _cx: &mut Ctx) -> Result<Brushes, CommandError> {
        Ok(Brushes { brushes: brushes() })
    }
}
register_command!(BrushList, "List brushes");

/// Say what a brush does: its description, every setting as the session's strokes use it (its
/// preset with what this session changed), what this session changed, the behaviors the tests
/// hold it to, and what the Brush Modifier does on it.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrushDescribe {
    /// The brush: its name or id, as `brush.list` gives them.
    pub brush: String,
}

/// A brush in full.
#[derive(Serialize, JsonSchema)]
pub struct BrushDescription {
    #[serde(flatten)]
    pub brief: BrushInfo,
    /// What it does, in words.
    pub about: String,
    /// Behaviors the tests hold it to, for example `local` (nothing outside the brush moves)
    /// or `masked_unchanged`.
    pub claims: Vec<String>,
    /// What the Brush Modifier (the setting `modifier`) does on it; nothing yet on most.
    pub modifier: Option<String>,
    /// Every setting, with every default filled in, as the session's strokes use it: the
    /// preset with what this session changed (ARCHITECTURE 4.2). `brush.set`'s and
    /// `brush.stroke`'s `settings` take the same names.
    pub settings: Value,
    /// What this session changed for the brush (`brush.set`), in the same names.
    pub changed: Value,
    /// The BLAKE3 hash of its preset, which every stroke with it pins.
    pub preset_hash: String,
    /// The BLAKE3 hash of the alpha picture it stamps with, which every stroke with it pins;
    /// none without an alpha.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_hash: Option<String>,
}

/// The comment lines at the top of a preset or alpha recipe file, as one paragraph.
pub(crate) fn about(text: &str) -> String {
    let lines = text.lines().map_while(|l| l.strip_prefix('#'));
    lines.map(str::trim).collect::<Vec<_>>().join(" ")
}

fn describe(s: &Shipped, cx: &Ctx) -> Result<BrushDescription, CommandError> {
    let p = &s.preset;
    let session = &cx.sculpt.settings;
    let resolved = session.resolved(&p.name)?;
    let alpha = (resolved.tip.alpha.as_ref())
        .map(|a| crate::alpha::picture(&a.image))
        .transpose()?;
    Ok(BrushDescription {
        brief: p.into(),
        about: about(&s.text),
        claims: p.claims.clone(),
        modifier: (p.kernel.modifier_meaning().filter(|_| p.sculpts())).map(str::to_string),
        settings: serde_json::to_value(&resolved).unwrap_or_default(),
        changed: json!(session.patch_of(&p.name)),
        preset_hash: p.hash(),
        alpha_hash: alpha.map(|a| a.hash().to_string()),
    })
}

impl Command for BrushDescribe {
    const NAME: &'static str = "brush.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = BrushDescription;

    fn examples() -> Vec<Example<Self>> {
        let draw = Self {
            brush: "Draw".into(),
        };
        let unknown = Self {
            brush: "Standard".into(),
        };
        vec![
            Example::new("what Draw does", draw),
            Example::new("a brush that does not exist", unknown).fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<BrushDescription, CommandError> {
        describe(presets::find(&self.brush)?, cx)
    }
}
register_command!(BrushDescribe, "Describe brush");
