//! `image.diff`.

use gs_protocol::{CommandError, ErrorCode, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::picture::{Picture, encode, read};
use crate::errors::args_invalid;
use crate::view::deliver;
use crate::{Command, Ctx, Example, register_command};

/// Compare two PNG files of the same size pixel by pixel, for example two `view.snapshot`
/// results before and after a change.
///
/// Reports how many pixels differ, the largest difference in one channel, the mean absolute
/// difference per channel and the box around the differing pixels. With `path`, also writes a
/// difference image there: differing pixels white, the rest `b` dimmed. Both files must be
/// 8-bit RGBA, as `view.snapshot` writes them; files of different sizes are refused.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageDiff {
    /// The first PNG file.
    #[schemars(length(min = 1))]
    pub a: String,
    /// The second PNG file.
    #[schemars(length(min = 1))]
    pub b: String,
    /// Write the difference image to this file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
}

/// How the two images differ. Channels are 8-bit levels, 0 to 255.
#[derive(Serialize, JsonSchema)]
pub struct Diff {
    /// How many pixels differ in any channel, alpha included.
    pub differing: u64,
    /// The largest difference in one channel of one pixel.
    pub max: u8,
    /// The mean absolute difference per channel over every pixel, as `[r, g, b, a]`.
    pub mean: [f64; 4],
    /// The differing pixels' bounds, both corners included, row 0 at the top. Null when no
    /// pixel differs.
    #[serde(rename = "box")]
    pub bounds: Option<PixelBox>,
    /// Where the difference image was written, when a path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A box of pixels, as `[x, y]` corners.
#[derive(Clone, Copy, Serialize, JsonSchema)]
pub struct PixelBox {
    pub min: [u32; 2],
    pub max: [u32; 2],
}

impl Command for ImageDiff {
    const NAME: &'static str = "image.diff";
    const KIND: Kind = Kind::Query;
    type Output = Diff;

    fn examples() -> Vec<Example<Self>> {
        vec![
            Example::new(
                "two snapshots compared, with a difference image (refused: no such files)",
                Self {
                    a: "/nonexistent/before.png".into(),
                    b: "/nonexistent/after.png".into(),
                    path: Some("/nonexistent/diff.png".into()),
                },
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, _cx: &mut Ctx) -> Result<Diff, CommandError> {
        let a = read(&self.a)?;
        let b = read(&self.b)?;
        if (a.width, a.height) != (b.width, b.height) {
            return Err(args_invalid(
                "/b",
                format!(
                    "a is {}x{} but b is {}x{}; image.diff compares images of the same size",
                    a.width, a.height, b.width, b.height
                ),
            ));
        }
        let mut sums = [0u64; 4];
        let mut max = 0;
        let mut differing = 0;
        let mut bounds: Option<PixelBox> = None;
        let mut shown = Vec::with_capacity(b.rgba.len());
        let pairs = a
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b.rgba.as_chunks::<4>().0);
        for (i, (pa, pb)) in pairs.enumerate() {
            let d: [u8; 4] = std::array::from_fn(|c| pa[c].abs_diff(pb[c]));
            for (sum, d) in sums.iter_mut().zip(d) {
                *sum += u64::from(d);
            }
            let worst = d.into_iter().max().unwrap_or(0);
            max = max.max(worst);
            if worst > 0 {
                differing += 1;
                let (x, y) = (i as u32 % a.width, i as u32 / a.width);
                bounds = Some(bounds.map_or(
                    PixelBox {
                        min: [x, y],
                        max: [x, y],
                    },
                    |r| PixelBox {
                        min: [r.min[0].min(x), r.min[1].min(y)],
                        max: [r.max[0].max(x), r.max[1].max(y)],
                    },
                ));
                shown.extend([255; 4]);
            } else {
                // A quarter of b's brightness, opaque.
                shown.extend(pb[..3].iter().map(|v| v.div_euclid(4)).chain([255]));
            }
        }
        let path = match self.path {
            Some(path) => {
                let picture = Picture { rgba: shown, ..a };
                deliver(&encode(&picture)?, Some(path))?.0
            }
            None => None,
        };
        let pixels = f64::from(a.width) * f64::from(a.height);
        Ok(Diff {
            differing,
            max,
            mean: sums.map(|s| s as f64 / pixels),
            bounds,
            path,
        })
    }
}
register_command!(ImageDiff, "Compare images");
