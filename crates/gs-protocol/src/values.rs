//! Value types shared by every command: points, lengths and references to parts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A point or direction in scene units, written as `[x, y, z]`. Y is up, right-handed.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Vec3(pub [f32; 3]);

/// The name of a part in the scene, for example `"sphere"`. See `scene.describe`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PartRef(pub String);

/// A length: a plain number in scene units (`0.2`), or a percentage of the target part's
/// bounding-box diagonal (`"5%"`), so agents need not know the absolute scale.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "LengthRepr", into = "LengthRepr")]
pub enum Length {
    /// Scene units.
    Units(f32),
    /// Percent of the target part's bounding-box diagonal (`5.0` means 5%).
    Percent(f32),
}

/// A number in scene units (0.2), or a percentage of the target part's bounding-box diagonal
/// ("5%").
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
enum LengthRepr {
    Number(f32),
    Text(#[schemars(regex(pattern = r"^[+-]?([0-9]+([.][0-9]*)?|[.][0-9]+)%$"))] String),
}

impl TryFrom<LengthRepr> for Length {
    type Error = String;

    fn try_from(repr: LengthRepr) -> Result<Self, String> {
        let not_length = || "expected a number like 0.2 or a percentage like \"5%\"".to_string();
        match repr {
            LengthRepr::Number(units) if units.is_finite() => Ok(Self::Units(units)),
            LengthRepr::Number(_) => Err(not_length()),
            LengthRepr::Text(text) => {
                let number = text
                    .strip_suffix('%')
                    .filter(|n| is_decimal(n))
                    .ok_or_else(not_length)?;
                match number.parse::<f32>() {
                    Ok(percent) if percent.is_finite() => Ok(Self::Percent(percent)),
                    _ => Err(not_length()),
                }
            }
        }
    }
}

/// True for an optional sign followed by digits with at most one decimal point. This rules out
/// what `f32` parsing accepts but the schema's pattern does not (exponents, `inf`, `nan`); the
/// parse itself rules out strings with no digit at all.
fn is_decimal(text: &str) -> bool {
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    all_digits(whole) && all_digits(fraction)
}

impl From<Length> for LengthRepr {
    fn from(length: Length) -> Self {
        match length {
            Length::Units(units) => Self::Number(units),
            Length::Percent(percent) => Self::Text(format!("{percent}%")),
        }
    }
}
