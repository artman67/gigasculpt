//! Brush presets (ARCHITECTURE 4.2): every brush is a TOML file over the five kernels, so
//! symmetry, masking, falloff, pressure and undo are built once and no brush can drift from the
//! others. [`BrushPreset::from_toml`] reads and checks one; `assets/brushes/` holds the shipped
//! ones. The stroke pipeline (`crate::stroke`) runs them.
//!
//! A brush in use is its preset with a patch: the settings changed for it, in the preset's own
//! names, merged in by [`BrushPreset::resolve`] (decision 0018). The session keeps one patch per
//! brush and every stroke carries its patch, so this module is the one home of what a brush's
//! settings are and what they mean, including what the Brush Modifier and the alpha's sign do
//! on each kernel ([`KernelSpec::modifier_meaning`], [`KernelSpec::signed_alpha`]).
//!
//! # Where the settings live
//!
//! The preset itself, its family and mode are here; its settings are in one file each by what
//! they shape: `kernel` (the kernel and its settings, the only brush-specific step of a dab),
//! `stroke` (the stroke class: how pointer movement becomes dabs), `dab` (what every dab reads
//! beside its kernel: tip, depth, sampling, pressure curves, post effects and automasks) and
//! `alpha` (the picture a tip stamps). `load` reads and checks a preset, `patch` merges a patch,
//! `save` writes a preset back into its file's text.
//!
//! # What a preset means
//!
//! This is the specification the slow reference in `gs-testkit` implements independently, so
//! the two share only preset data.
//!
//! **Dabs.** The press gets a dab standing for `tap * r` of stroke. Then the path is cut into
//! cells one spacing step long (`spacing * pressure.spacing(p) * 2r`, read at the cell's start),
//! and each cell, including the unfinished one when the stroke ends, gets a dab at its middle
//! (the midpoint of its ends) standing for its length `ds`. A dab's radius is
//! `r = sample radius * pressure.size(p)` and its amount is
//! `strength * pressure.strength(p) * ds / r`, so the total amount of a stroke depends on its
//! length, never on spacing or sample rate (ARCHITECTURE 4.3).
//!
//! **Dab frame.** Over the vertices within `R = sampling.normal_radius * r` of the dab center
//! `c` (and facing the camera, `n . view < 0`, with `sampling.front_only`), the area normal is
//! `N = normalize(sum w n)` with `w = 1 - d / R`; with no such vertex it is `-view`. The area
//! center `O` is the same weighted mean of positions over `sampling.position_radius * r`, or `c`.
//! With `sampling.accumulate` the frame reads the current surface, without it the surface as
//! the stroke started. The frame's axes across the dab are `t`, the step from the previous dab
//! within the plane across `N` (or, when that part is under a hundred-thousandth of the step,
//! the coordinate axis most across `N`, the first on a tie), and `b = N x t`; a drag's step is
//! its offset `T`, a Drag Rect dab's the drag from the press, and a Drag Dot dab has none.
//!
//! **Tilt and Align To Path** (decision 0058). With `tip.constant_tilt`, or `tip.align` below
//! 100, `t` follows the stroke's aim instead of the step: the first step sets it, Constant
//! Tilt holds it, and each later step turns it toward itself about `N` by
//! `1 - exp(-s / (2r (1 - align / 100)))` of the angle between them (`s` the step's length).
//! Each pass's copy then leans by `a = tip.tilt` degrees: `N' = cos(a) N + sin(a) t` and
//! `t' = cos(a) t - sin(a) N`, which everything after reads.
//!
//! **Influence.** For each symmetry pass, vertex weight `w = tip falloff(|p - c| / r) * alpha *
//! automask`, times `1 - mask` (except for mask and hide brushes), times 0 for a hidden vertex
//! (one with faces, all of them hidden); then, where symmetry passes overlap, scaled by one
//! factor per dab (`Symmetry::feather_scale`, decision 0040). With `depth.infinite` the
//! distance `|p - c|` (and the feather's center) leaves out the parts along the free
//! directions: the pass's view, or the model axes switched on in `depth.axes` turned by the
//! pass. With `depth.mask` the tip's weight is also times `depth.curve(|h| / e)`, with
//! `h = (p - (O + N * imbed * r)) . N` and `e = outer * r` for `h > 0`, else `-inner * r`;
//! 0 where `|h| > e` (decision 0053).
//!
//! **Curve By Pen and Accurate Curve** (decision 0059). With `tip.curve_by_pen`, a dab at
//! pressure `q` reads the falloff as `z + k (e - z)` at the remapped distance, where `e` is the
//! profile (the Edit Curve), `z` is `tip.zero_curve` and `k` is `tip.pen_curve(q)`; a drag
//! update takes its latest sample's pressure, and a sample without one presses fully. With
//! `tip.accurate_curve`, a custom profile and the Zero Curve with smooth points are read from
//! their cubics at each vertex instead of their 257-entry tables. The profiles `soft`, `hard`,
//! `flat_top` and `sharp_center` are exact formulas (`crate::falloff`). The slow reference has
//! none of these; every shipped preset leaves them out.
//!
//! **Wrap Mode** (`tip.wrap`, decision 0054). Above 1, the part's box as the stroke begins is
//! cut into `tip.wrap` equal tiles along each of its two longer sides (the first of equal sides
//! is the thinnest), and every pass is laid again moved by whole tiles: each copy whose sphere
//! of the pass's reach meets the box, and the pass itself, in tile order. The slow reference
//! has no Wrap Mode; every shipped preset has it at 1.
//!
//! **Hidden faces** (decision 0015). Every neighbour list (Smooth, mask blur and sharpen, the
//! walk along edges) is built from visible faces only, so the edge of the visible region is a
//! boundary. The dab frame and the nearest vertex a stroke starts from read visible vertices
//! only. Normals still count every face.
//!
//! **Alpha.** The picture `tip.alpha.image` goes through its tone settings into a pyramid
//! (`crate::alpha`). Each dab reads one level: with `n` the visible vertices within
//! `R = max(sampling.normal_radius, sampling.position_radius) * r` of `c` (facing the camera
//! or not), the vertex spacing is `R * sqrt(pi / max(n, 1))`, and the level is the finest whose
//! texel spacing `2r / (tiles * size)` (the smaller over the two axes) is at least
//! `tip.alpha.adaptive` times half of that (level 0 at 0), or the coarsest. A vertex reads it at `(u, v) = ((p - c) . t / R, (p - c) . b / R)` with `R`
//! the dab's reach (`r`, or `support * r` for the elastic move): the picture's square spans
//! `-1..1` both ways around the circle the weights reach, its columns along `u` and
//! its top row at `v = 1` (seen from outside with the stroke running right, the picture is
//! upright), and is 0 outside; `tiles` repeats it inside, and the four texels around the point
//! (texel `i` centered at `(i + 0.5) / size`, held at the edges, wrapped across tiles) are
//! blended bilinearly to a value `v`. Levels: level 0 is the processed picture, each next one
//! the rounded mean of 2 by 2 texels of the one before (reading the last row or column again
//! past the edge), down to 1 by 1. A kernel with [`KernelSpec::signed_alpha`] (Offset) takes
//! the factor `v - mid`, which carves below the mid value; the others take
//! `max(0, v - mid) / (1 - mid)` (0 at `mid = 1`). With no alpha the factor is 1.
//!
//! **Kernels.** `s` is -1 when the stroke is inverted (Alt), else 1; `mode = "subtract"` flips
//! it once more. Alt is read per sample: a sample's sign holds for the dabs it lays, and a drag
//! (or a class laid at the end) takes the latest sample's sign as a whole (decision 0036).
//!
//! - Offset: each term adds `D * gain * s * w^exponent * amount * r` along its direction `D`
//!   (area normal `N`, vertex normal, `-view`, or an axis), except `to_center`, which adds
//!   `(c - p) * (1 - exp(-gain * s * w^exponent * amount))`, with the part along `N` removed
//!   when `tangent` is set. A negative `w` (an alpha below its mid value) gives
//!   `w^exponent = -|w|^exponent`. The Brush Modifier adds one more term per dab: with
//!   `m = modifier / 100 * pressure.modifier(p)`, a positive `m` adds `to_center` with
//!   `tangent` and gain `2m` (a pinch), a negative one `vertex_normal` with gain `0.5 |m|` (an
//!   inflate).
//! - Plane: with `M = s * N`, the plane passes through `O + M * imbed * r` with normal `M`,
//!   where `imbed = depth.imbed * pressure.imbed(p)`; `d` is the signed distance of `p` above
//!   it. Vertices on the excluded side (`above` keeps `d > 0`, `below` keeps `d < 0`) or past
//!   the `height` / `depth` caps (in radii) stay put; the rest move by
//!   `-M * d * (1 - exp(-gain * w * amount))`. With `source = "stroke"` the first dab's `N` and
//!   `O` serve the whole stroke; with `source = "stroke_normal"` the first dab's `N` with each
//!   dab's own `O`.
//! - Smooth: `(t - p) * s * (1 - exp(-gain * w * amount))` toward a target `t` made from the
//!   edge neighbours (boundary vertices use only their boundary neighbours): `uniform` their
//!   mean; `stronger` the mean of their uniform targets; `tangent` the uniform target with the
//!   part along the vertex normal removed; `relax` the mean of `(u - p) (1 - L / |u - p|)`, with
//!   `L` the mean edge length, along the surface; `along_stroke` / `across_stroke` the point on
//!   the line joining the nearest cuts, ahead and behind, of the ring by the plane through `p`
//!   holding `N` and the dab's step laid into the tangent plane (or `N` crossed with it);
//!   `group_borders` the uniform target moved by `weight` toward the mean of the neighbours across
//!   a face group border. A vertex with fewer than `min_connected` neighbours stays; `projection`
//!   removes the part along the vertex's normal at the stroke's start (decision 0049). A dab
//!   with `x = gain * amount` above 0.5 runs as `n = ceil(x / 0.5)` sub-steps (at most 32; `x`
//!   found in `f32`), each a whole pass of the kernel and gravity with `amount / n` over the
//!   positions and normals the one before left; `auto_smooth` runs once after them (decision
//!   0036).
//! - Move: a drag. With `T` the pointer's offset from the first sample `g` (along `N` at `g`
//!   when inverted), every vertex goes to its stroke-start position plus `T * w * strength`.
//!   `ball` weighs by distance to `g`; `connected` by distance along edges from the vertex
//!   nearest `g`, out to `r`; `elastic` replaces the tip falloff with a regularized Kelvinlet
//!   (scale `r`, Poisson ratio `poisson`) faded to zero at `support * r`. A drag update stands
//!   for the stroke length `|T|`, so its amount (which only post effects read) is
//!   `strength * pressure.strength(p) * |T| / r`.
//! - Hook: each dab moves the vertices around the previous dab point by the step to this one,
//!   times `w * strength * pressure.strength(p)`, plus a pull of `pinch` toward the line of
//!   motion. On a screen stroke its `direction` and the Brush Modifier choose the plane the
//!   pointer lands on ([`BrushPreset::pull_plane`], decision 0049, section 7).
//! - Attribute: the mask moves toward `value` (zero when inverted) by
//!   `1 - exp(-gain * w * amount)`, or toward the mean of its neighbours (blur) or `2m - mean`
//!   clamped (sharpen); mask values are stored clamped to 0..1 on a 2^-24 grid (`crate::mask`).
//!   Face groups and hide take the value on faces whose center has `w >= 0.5`; face groups skip
//!   hidden faces. Color is not this kernel's: a brush paints color with a `[paint]` table.
//!
//! With `mode = "off"` the kernel and the post effects never run: the brush only paints.
//!
//! **Post effects** run after the kernel on every dab and read the size `|w|` of the weight:
//! `auto_smooth` is the gain of a Smooth pass in `auto_smooth_mode` (never inverted), skipped
//! on a part with more than `auto_smooth_max_faces` faces (0 never skips); gravity adds
//! `gravity_dir * gravity * |w| * amount * r`, `gravity_dir` in the model's frame turned by
//! each pass's direction map.
//!
//! **Paint** (decision 0039, sections 6 to 8). A brush with a `[paint]` table paints color in
//! each dab after its kernel and post effects, with the same frames and weights, never
//! inverted and reading the size of the alpha's factor. Colors mix in linear light. A stroke
//! keeps per point its start color `s` and a mix `x`, both linear; each dab moves `x` toward
//! its target by `1 - exp(-8 * w * amount)`, the target the Main color, with Alt the Secondary,
//! and with Gradient the other color mixed toward that one by the tip falloff (before `1 - mask`).
//! The point shows `s + (x - s) * color_strength`, sRGB-encoded and rounded to the nearest
//! level; at the stroke's end every point whose `x` moved is rounded down after adding a
//! number from 0 to 1 drawn at its id from the stroke's seed. Smooth paints instead by moving
//! each touched point's color toward its edge neighbours' mean, in linear light, by
//! `color_strength * (1 - exp(-gain * w * amount))` from the colors as the dab found them,
//! rounding each dab down after adding a number drawn at the point's id and the dab's count.
//! A point with no color is white.

mod alpha;
mod dab;
mod kernel;
mod load;
mod patch;
mod save;
mod stroke;

use serde::{Deserialize, Serialize};

pub use alpha::{AlphaSpec, Flip};
pub use dab::{Automask, Depth, Post, Pressure, ProfileName, Sampling, TipSpec};
pub use kernel::{
    AttrOp, AttrSpec, Column, Direction, HookSpec, KernelSpec, MoveSpec, OffsetSpec, OffsetTerm,
    PlaneSource, PlaneSpec, PullDirection, Select, Side, SmoothMode, SmoothSpec,
};
pub use load::{FORMAT, PresetError};
pub use patch::{Patch, merge};
pub use save::Changes;
pub use stroke::{Shape, Smoothing, StrokeSpec, StrokeType, carried};

/// One brush: kernel, tip, depth, sampling, stroke, pressure, post effects and automask.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrushPreset {
    /// The format the file is written in, [`FORMAT`] when left out (decision 0018, "Saving a
    /// brush"). Not a setting, and not in the canonical form, so it changes no hash.
    #[serde(default = "load::format", skip_serializing)]
    pub version: u32,
    /// Display name, from `docs/NAMES.md`.
    pub name: String,
    /// The catalog item this preset builds (`brush.draw`).
    pub id: String,
    /// The group the interface lists it under (decision 0013).
    pub family: Family,
    /// Behaviors the tests hold this preset to, by name (the vocabulary lives in the tests).
    #[serde(default)]
    pub claims: Vec<String>,
    /// Default Strength, 0 to 1.
    pub strength: f32,
    /// The Brush Modifier, -100 to 100; what it does is set by the kernel
    /// ([`KernelSpec::modifier_meaning`]).
    #[serde(default)]
    pub modifier: f32,
    /// Add or Subtract: Subtract inverts every stroke, and Alt inverts it back.
    #[serde(default)]
    pub mode: Mode,
    pub kernel: KernelSpec,
    #[serde(default)]
    pub tip: TipSpec,
    /// How high the brush sits.
    #[serde(default)]
    pub depth: Depth,
    #[serde(default)]
    pub sampling: Sampling,
    #[serde(default)]
    pub stroke: StrokeSpec,
    #[serde(default)]
    pub pressure: Pressure,
    #[serde(default)]
    pub post: Post,
    #[serde(default)]
    pub automask: Automask,
    /// What the brush paints in each dab after it sculpts (decision 0039, section 8). Left out
    /// of the preset's text when it has none, so a preset without one keeps its hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<PaintSpec>,
    /// How the brush refines with Live Detail (decision 0021, section 4). Left out of the
    /// preset's text while it is the default, so a preset without one keeps its hash.
    #[serde(default, skip_serializing_if = "LiveDetailSpec::is_default")]
    pub live_detail: LiveDetailSpec,
}

/// A preset's `[paint]` table: the channel a brush paints and how far a stroke goes toward it.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PaintSpec {
    pub channel: Channel,
    /// Color Strength, 0 to 1: caps the whole stroke, so 0.5 over a point the stroke covers
    /// fully gives the half-way mix in linear light however many dabs overlap.
    #[serde(default = "full")]
    pub color_strength: f32,
}

fn full() -> f32 {
    1.0
}

/// A channel a brush paints. Only color today; a material index is later (decision 0039).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// The point colors (the `color` column).
    Color,
}

/// A preset's `[live_detail]` table: whether the brush refines at all, and the settings it
/// gives in place of the session's. A field left out is the session's.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct LiveDetailSpec {
    /// Off: the brush never refines, whatever the session says.
    #[serde(skip_serializing_if = "is_true")]
    pub enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<LiveMode>,
    /// The size in the mode's terms: a fraction of the brush radius, pixels, or scene units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combined: Option<bool>,
}

/// How Live Detail's target edge length follows the brush (decision 0021, section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveMode {
    /// A fraction of the brush radius: small brushes add fine detail.
    Brush,
    /// A length in screen pixels, so detail follows the zoom.
    Screen,
    /// A length in scene units, the same everywhere.
    Constant,
}

impl Default for LiveDetailSpec {
    fn default() -> Self {
        Self {
            enable: true,
            mode: None,
            size: None,
            merge: None,
            combined: None,
        }
    }
}

impl LiveDetailSpec {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference"
)]
fn is_true(b: &bool) -> bool {
    *b
}

/// The family a brush belongs to: what it is for, as the interface groups brushes. A closed
/// list, so a misspelled family is refused with the rest of the preset; a new family is one
/// line here. Brushes are listed by family in this order (decision 0013).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Family {
    /// Adds form: Draw, Clay and their kin.
    Build,
    /// Cuts in or takes away: creases, pinches, trims and clips.
    Carve,
    /// Drags what is under the pen.
    Move,
    /// Evens the surface out: smooths and polishes.
    Smooth,
    /// Masks, hides and selects instead of changing shape.
    Mask,
    /// Paints the surface instead of changing shape.
    Paint,
}

impl Family {
    /// Its name, as preset files and `brush.list` write it (the variants' order).
    #[must_use]
    pub fn name(self) -> &'static str {
        ["Build", "Carve", "Move", "Smooth", "Mask", "Paint"][self as usize]
    }
}

/// Add, Subtract or off, per brush: whether a stroke builds the surface up, carves it in or
/// leaves its shape alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Strokes work the brush's own way; Alt inverts them.
    #[default]
    Add,
    /// Strokes are inverted; Alt inverts them back.
    Subtract,
    /// The kernel never runs: the brush only paints, by its `[paint]` table.
    Off,
}

impl BrushPreset {
    /// Read and check a preset.
    pub fn from_toml(text: &str) -> Result<BrushPreset, PresetError> {
        load::parse(text)
    }

    /// The preset in its canonical form: every default filled in, written out as TOML. Two
    /// presets that mean the same have the same canonical form, whatever their files' comments
    /// and layout.
    pub fn canonical(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The BLAKE3 hash of the canonical form, as 64 hex digits: what a stroke's log line pins
    /// (decision 0018).
    pub fn hash(&self) -> String {
        blake3::hash(self.canonical().as_bytes())
            .to_hex()
            .to_string()
    }

    /// Whether strokes are inverted before Alt: the Subtract mode.
    pub fn subtracts(&self) -> bool {
        self.mode == Mode::Subtract
    }

    /// Whether strokes run the kernel: every mode but off.
    pub fn sculpts(&self) -> bool {
        self.mode != Mode::Off
    }

    /// Whether strokes paint color: they mix it in floats and dither once at their end, from
    /// their seed (decision 0039, sections 6 and 8).
    pub fn paints_color(&self) -> bool {
        self.paint.is_some_and(|p| p.channel == Channel::Color)
    }
}
