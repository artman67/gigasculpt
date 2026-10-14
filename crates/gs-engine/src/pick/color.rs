//! `pick.color` (decision 0039, section 11, as amended 2026-10-12 in step P8b).

use gs_core::columns::linear_to_srgb;
use gs_core::math::{closest_on_triangle, face_triangles};
use gs_core::{AttrValue, FaceId, MeshStore, NO, PosRead, TopoRead, Vec3, linear_rgb};
use gs_protocol::{COLOR, CommandError, Cost, Kind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ray::PickRay;
use super::{first_hit, parts};
use crate::display::color::hex;
use crate::{Command, Ctx, Example, register_command};

/// `pick.ray`'s arguments, asking for the color.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct PickColor(pub PickRay);

/// The color stored where the ray meets a part.
#[derive(Serialize, JsonSchema)]
pub struct PickedColor {
    /// The color there, `"#rrggbb"` in sRGB; left out when the ray meets no part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The part it meets; left out likewise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

impl Command for PickColor {
    const NAME: &'static str = "pick.color";
    const KIND: Kind = Kind::Query;
    const COST: Cost = Cost::Instant;
    const DOCS: &'static str = "The color stored where a ray first meets a part, as the eyedropper \
        picks it: unshaded, read from the part's paint, never from the screen.\n\nIt takes \
        `pick.ray`'s arguments: a part, or every part that shows, then a camera, its image size \
        and a point on its image, or an origin and a direction. Inside a face it answers the \
        colors of the corners of the drawn triangle there, blended in linear light by where the \
        ray meets it, as the 3D view draws them, and rounded to bytes; at a corner, the \
        corner's own. Unpainted points are white. Paint that Show Paint hides still answers. On \
        a part with detail levels it reads the level shown. Off every part it answers neither \
        `color` nor `part`.";
    type Output = PickedColor;

    fn examples() -> Vec<Example<Self>> {
        let painted = json!({ "color": "#3366cc", "seed": 1 });
        vec![
            Example::raw(
                "the color in the middle of a front snapshot of a painted sphere",
                json!({ "camera": "front", "at": [512.0, 512.0] }),
            )
            .after("primitive.sphere", json!({}))
            .after("paint.fill", painted),
            Example::raw(
                "straight down past every part",
                json!({ "origin": [5.0, 5.0, 0.0], "direction": [0.0, -1.0, 0.0] }),
            )
            .after("primitive.sphere", json!({})),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<PickedColor, CommandError> {
        let (o, d) = self.0.ray(cx)?;
        let doc = cx.doc.document();
        let hit = first_hit(&parts(doc, self.0.part.as_ref())?, o, d);
        Ok(match hit {
            Some((_, hit, name, part)) => PickedColor {
                color: Some(hex(paint_at(part.store(), hit.face, hit.pos))),
                part: Some(name.to_string()),
            },
            None => PickedColor {
                color: None,
                part: None,
            },
        })
    }
}
register_command!(PickColor, "Pick the color on a ray");

/// The color stored at point `p` of face `f`, in the store's own space, as the GPU draws it
/// (section 9): in the drawn triangle of `f` nearest `p` (`face_triangles`, the split the 3D view
/// draws), its corners' colors in linear light weighted by where `p` lies, encoded to sRGB and
/// rounded to the nearest byte. At a corner, the corner's own bytes; unpainted, white.
pub(crate) fn paint_at(store: &MeshStore, f: FaceId, p: Vec3) -> [u8; 3] {
    let face = store.topo().face(f);
    let pos = store.pos();
    let at = face.map(|v| if v == NO { Vec3::ZERO } else { pos.get(v) });
    let colors = store.attr_id(COLOR).map(|c| store.attr_reader(c));
    let color = |k: usize| match colors.and_then(|r| r.get(face[k])) {
        Some(AttrValue::U32(c)) => c,
        _ => u32::MAX,
    };
    let near = (face_triangles(face, |k| at[k].to_array()).iter())
        .map(|t| (t, closest_on_triangle(p, &t.map(|k| at[k]))))
        .min_by(|a, b| (a.1.0 - p).len2().total_cmp(&(b.1.0 - p).len2()));
    let Some((t, (_, w))) = near else {
        return [255; 3];
    };
    let mut light = [0.0f32; 3];
    for (&k, w) in t.iter().zip(w) {
        for (sum, c) in light.iter_mut().zip(linear_rgb(color(k))) {
            *sum += w * c;
        }
    }
    light.map(|c| (255.0 * linear_to_srgb(c.clamp(0.0, 1.0)) + 0.5).floor() as u8)
}
