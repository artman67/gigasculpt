//! `alpha.list` and `alpha.describe`.

use gs_protocol::{CommandError, Cost, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Shipped, find, shipped};
use crate::{Command, Ctx, Example, register_command};

/// List every alpha brushes can stamp with: grey pictures that shape each dab. A brush names
/// one in its setting `tip.alpha.image` (`brush.set`'s `alpha`), by name or by `blake3:` and
/// its hash. Each is 16-bit grey, our own, drawn from a recipe. Says too which were used lately,
/// as the Alpha palette shows them (`alpha.pick`).
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlphaList {}

/// One alpha, in brief.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct AlphaInfo {
    /// What brushes name it by, for example `square`.
    pub name: String,
    /// What the user reads, for example `Square`.
    pub title: String,
    /// The catalog item it builds.
    pub catalog_id: String,
    pub width: u32,
    pub height: u32,
    /// Bits per pixel.
    pub bits: u32,
    /// `height`: one grey value per pixel (vector pictures come later).
    pub kind: String,
    /// The BLAKE3 hash of its pixels, as 64 hex digits: its name by content.
    pub hash: String,
}

/// Every alpha, by name.
#[derive(Serialize, JsonSchema)]
pub struct Alphas {
    pub alphas: Vec<AlphaInfo>,
    /// The alphas used lately, by name, slot by slot as the Alpha palette shows them.
    pub recent: Vec<String>,
}

fn info(s: &Shipped) -> Result<AlphaInfo, CommandError> {
    let p = s.picture()?;
    Ok(AlphaInfo {
        name: s.name.into(),
        title: s.recipe.name.clone(),
        catalog_id: s.recipe.catalog_id.clone(),
        width: p.width(),
        height: p.height(),
        bits: super::BITS,
        kind: "height".into(),
        hash: p.hash().into(),
    })
}

impl Command for AlphaList {
    const NAME: &'static str = "alpha.list";
    const KIND: Kind = Kind::Query;
    type Output = Alphas;

    fn examples() -> Vec<Example<Self>> {
        vec![Example::new("every alpha", Self {})]
    }

    fn run(self, cx: &mut Ctx) -> Result<Alphas, CommandError> {
        let alphas = shipped().iter().map(info).collect::<Result<_, _>>()?;
        let recent = cx.sculpt.settings.recent.list().to_vec();
        Ok(Alphas { alphas, recent })
    }
}
register_command!(AlphaList, "List alphas");

/// Say what an alpha is: its size, hash, what it shows, and the recipe it is drawn from.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlphaDescribe {
    /// The alpha, by name (`alpha.list`).
    pub alpha: String,
}

/// An alpha in full.
#[derive(Serialize, JsonSchema)]
pub struct AlphaDescription {
    #[serde(flatten)]
    pub brief: AlphaInfo,
    /// What it shows, in words.
    pub about: String,
    /// The recipe it is drawn from: layers combined in order.
    pub recipe: Value,
}

impl Command for AlphaDescribe {
    const NAME: &'static str = "alpha.describe";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    type Output = AlphaDescription;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "the square Clay Build stamps",
                Self {
                    alpha: "square".into(),
                },
            ),
            Example::new(
                "an alpha that does not exist",
                Self {
                    alpha: "pores".into(),
                },
            )
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn run(self, _cx: &mut Ctx) -> Result<AlphaDescription, CommandError> {
        let s = find(&self.alpha)?;
        let recipe = toml::from_str::<toml::Value>(s.text).map_or(Value::Null, |t| json!(t));
        Ok(AlphaDescription {
            brief: info(s)?,
            about: crate::sculpt::about(s.text),
            recipe,
        })
    }
}
register_command!(AlphaDescribe, "Describe alpha");
