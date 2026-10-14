//! The canvas's colors as commands take and report them: `"#rrggbb"`, 8-bit sRGB, the form
//! `group.list` reports face group colors in.

use gs_protocol::CommandError;
use serde::{Deserialize, Deserializer, Serializer};

use crate::errors::args_invalid;

/// The pattern a color argument's schema checks.
pub(crate) const PATTERN: &str = r"^#[0-9a-fA-F]{6}$";

/// `hex` (`"#rrggbb"`) as bytes; the argument at `path` names it when it is not one.
pub(crate) fn parse(path: &str, hex: &str) -> Result<[u8; 3], CommandError> {
    let bad = || args_invalid(path, format!("{hex} is not a color: give \"#rrggbb\""));
    let digits = hex
        .strip_prefix('#')
        .filter(|d| d.len() == 6)
        .ok_or_else(bad)?;
    let byte = |i: usize| {
        let pair = digits.get(i..i + 2).ok_or_else(bad)?;
        u8::from_str_radix(pair, 16).map_err(|_| bad())
    };
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

/// `c` as `"#rrggbb"`.
pub(crate) fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Serde for a color field held as bytes and written as `"#rrggbb"`.
pub(crate) mod as_hex {
    use super::{Deserialize, Deserializer, Serializer};

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` takes the field by reference"
    )]
    pub(crate) fn serialize<S: Serializer>(c: &[u8; 3], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::hex(*c))
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 3], D::Error> {
        let text = String::deserialize(d)?;
        super::parse("", &text).map_err(|e| serde::de::Error::custom(e.message))
    }
}
