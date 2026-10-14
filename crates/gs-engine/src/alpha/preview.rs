//! `alpha.preview`.

use gs_core::alpha::Pyramid;
use gs_core::brush::AlphaSpec;
use gs_protocol::{CommandError, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::errors::{self, internal};
use crate::view::deliver;
use crate::{Command, Ctx, Example, register_command};

/// Show an alpha as a 16-bit grey PNG, square, top row first: on its own, or through a brush's
/// tone settings as the brush stamps it (blur, contrast, curve, tiles and the rest, as this
/// session has the brush). Returns the PNG as base64, or writes it to `path`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlphaPreview {
    /// The alpha, by name (`alpha.list`). Left out, the alpha the brush stamps with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<String>,
    /// A brush whose tone settings to show the alpha through (`brush.list`). Left out, the
    /// alpha as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    /// Width and height in pixels, 16 to 1024. Left out, 256.
    #[serde(default = "side")]
    #[schemars(range(min = 16, max = 1024))]
    pub size: u32,
    /// Write the PNG here instead of returning it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn side() -> u32 {
    256
}

/// The preview.
#[derive(Serialize, JsonSchema)]
pub struct Preview {
    /// The alpha shown, by name or hash.
    pub alpha: String,
    /// The size in pixels, each way.
    pub size: u32,
    /// Where the PNG was written, when a path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The PNG as base64, when no path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png_base64: Option<String>,
}

impl Command for AlphaPreview {
    const NAME: &'static str = "alpha.preview";
    const KIND: Kind = Kind::Query;
    type Output = Preview;

    fn examples() -> Vec<Example<Self>> {
        let plain = Self {
            alpha: Some("ring".into()),
            brush: None,
            size: 64,
            path: None,
        };
        let stamped = Self {
            alpha: None,
            brush: Some("Clay".into()),
            size: 64,
            path: None,
        };
        let neither = Self {
            alpha: None,
            brush: None,
            size: 64,
            path: None,
        };
        vec![
            Example::new("the ring alpha", plain),
            Example::new("the alpha Clay stamps, as it stamps it", stamped),
            Example::new("nothing to show", neither).fails_with(ErrorCode::ArgsInvalid),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<Preview, CommandError> {
        if !(16..=1024).contains(&self.size) {
            return Err(errors::args_invalid("/size", "the size must be 16 to 1024"));
        }
        let spec = self.spec(cx)?;
        let pyramid = super::pyramid(&spec)?;
        let png = encode(&pyramid, &spec, self.size)?;
        let (path, png_base64) = deliver(&png, self.path)?;
        Ok(Preview {
            alpha: spec.image,
            size: self.size,
            path,
            png_base64,
        })
    }
}
register_command!(AlphaPreview, "Preview alpha");

impl AlphaPreview {
    /// How the alpha is shown: the brush's settings as this session has it, with the alpha
    /// asked for, or the alpha with neutral settings.
    fn spec(&self, cx: &Ctx) -> Result<AlphaSpec, CommandError> {
        let brush = match &self.brush {
            Some(b) => {
                let preset = cx.sculpt.settings.resolved(b)?;
                let spec = preset.tip.alpha;
                if spec.is_none() && self.alpha.is_none() {
                    let message = format!("{} stamps no alpha: name one", preset.name);
                    return Err(errors::args_invalid("/alpha", message));
                }
                spec
            }
            None => None,
        };
        match (brush, &self.alpha) {
            (Some(spec), Some(a)) => Ok(AlphaSpec {
                image: a.clone(),
                ..spec
            }),
            (Some(spec), None) => Ok(spec),
            (None, Some(a)) => Ok(AlphaSpec::plain(a)),
            (None, None) => Err(errors::args_invalid(
                "/alpha",
                "name an alpha, a brush, or both",
            )),
        }
    }
}

/// The finest level read across a `size`-pixel square, as a 16-bit grey PNG: each pixel the
/// value at its center, as a dab reads it (tiles included).
fn encode(pyramid: &Pyramid, spec: &AlphaSpec, size: u32) -> Result<Vec<u8>, CommandError> {
    let level = pyramid.level_for(1.0, 4.0 / size as f32, spec.tiles);
    let at = |i: u32| 2.0 * (i as f32 + 0.5) / size as f32 - 1.0;
    let mut bytes = Vec::with_capacity((size * size * 2) as usize);
    for y in 0..size {
        for x in 0..size {
            let v = pyramid.sample(level, at(x), -at(y), spec.tiles);
            bytes.extend(((v * 65535.0).round() as u16).to_be_bytes());
        }
    }
    let failed = |e: png::EncodingError| internal(&format!("encoding the preview: {e}"));
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, size, size);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Sixteen);
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(&bytes).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}
