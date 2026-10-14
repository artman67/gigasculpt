//! Reading and checking a preset. TOML syntax, unknown keys and wrong types are caught while
//! parsing; ranges and combinations that cannot work are caught here, each with the key's path.

use std::fmt;

use super::alpha::HASH_PREFIX;
use super::{
    AlphaSpec, AttrOp, BrushPreset, Column, Direction, KernelSpec, Pressure, ProfileName, Select,
    Shape, Smoothing, StrokeSpec,
};
use crate::curve::Curve;

/// Why a preset was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetError {
    pub message: String,
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PresetError {}

fn fail<T>(message: String) -> Result<T, PresetError> {
    Err(PresetError { message })
}

/// `v` must lie in `lo..=hi` (NaN never does).
fn range(path: &str, v: f32, lo: f32, hi: f32) -> Result<(), PresetError> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        fail(format!("{path} must be from {lo} to {hi}, got {v}"))
    }
}

/// Curve `c`, read as a fraction (`what`), must keep every point's value from 0 to 1.
fn fraction(path: &str, c: &Curve, what: &str) -> Result<(), PresetError> {
    match c.outside_fraction() {
        Some([x, y]) => fail(format!(
            "{path} values must be from 0 to 1 ({what}), got {y} at x = {x}; write 0.5 for half, \
             not 50"
        )),
        None => Ok(()),
    }
}

/// The pressure curves, each a multiplier of its slider: at full pressure at most the slider.
fn pressure(p: &Pressure) -> Result<(), PresetError> {
    let curves = [
        ("size", &p.size),
        ("strength", &p.strength),
        ("spacing", &p.spacing),
        ("modifier", &p.modifier),
        ("imbed", &p.imbed),
    ];
    for (name, c) in curves {
        fraction(&format!("pressure.{name}"), c, "a multiplier of its slider")?;
    }
    Ok(())
}

/// The preset format this build reads and writes: a file's `version` (decision 0018, "Saving a
/// brush"). A later format may add keys, so its files are refused by their version first.
pub const FORMAT: u32 = 1;

/// The version of a file that leaves it out.
pub(super) fn format() -> u32 {
    FORMAT
}

pub(super) fn parse(text: &str) -> Result<BrushPreset, PresetError> {
    match toml::from_str(text) {
        Ok(p) => check(p),
        Err(e) => {
            // A later format fails on the keys it adds; say which format it is instead.
            let table = toml::from_str::<toml::Table>(text).ok();
            let v = table.as_ref().and_then(|t| t.get("version")?.as_integer());
            version(v.unwrap_or(1))?;
            fail(format!("brush preset does not parse: {e}"))
        }
    }
}

/// `version` must be a format this build reads.
fn version(v: i64) -> Result<(), PresetError> {
    if v > i64::from(FORMAT) {
        return fail(format!(
            "version: this file is brush preset format {v}, and this build reads format {FORMAT}"
        ));
    }
    if v < 1 {
        return fail(format!("version must be from 1 to {FORMAT}, got {v}"));
    }
    Ok(())
}

/// A preset that read without error, checked for ranges and combinations that cannot work.
pub(super) fn check(p: BrushPreset) -> Result<BrushPreset, PresetError> {
    version(i64::from(p.version))?;
    if p.name.trim().is_empty() {
        return fail("name must not be empty".into());
    }
    range("strength", p.strength, 0.0, 1.0)?;
    kernel(&p.kernel)?;
    smooth_keys(&p.kernel)?;
    tip(&p)?;
    stroke(&p)?;
    pressure(&p.pressure)?;
    rest(&p)?;
    depth_and_modifier(&p)?;
    depth_mask_and_reach(&p)?;
    paint(&p)?;
    Ok(p)
}

/// The Depth Mask and Infinite Depth (decision 0053, sections 1, 4 and 7): their ranges, and
/// the strokes and kernels they cannot run with.
fn depth_mask_and_reach(p: &BrushPreset) -> Result<(), PresetError> {
    let d = &p.depth;
    range("depth.inner", d.inner, -1.0, 0.0)?;
    range("depth.outer", d.outer, 0.0, 1.0)?;
    fraction("depth.curve", &d.curve, "a weight")?;
    let gesture = matches!(p.stroke, StrokeSpec::Gesture { .. });
    if gesture && d.mask {
        return fail("depth.mask needs a dab to measure from; a gesture stroke has none".into());
    }
    if !d.infinite {
        return Ok(());
    }
    let why = match &p.kernel {
        _ if gesture => "a gesture stroke has no dab to reach from",
        KernelSpec::Move(m) if m.select == Select::Connected => {
            "select = \"connected\" reaches along edges, not through space"
        }
        KernelSpec::Move(m) if m.select == Select::Elastic => {
            "select = \"elastic\" is a solid's response, with no meaning along an endless line"
        }
        KernelSpec::Attribute(_) => "the attribute kernel finds its nodes by a sphere",
        _ if p.paint.is_some() => "a [paint] table finds its nodes by a sphere",
        _ => return Ok(()),
    };
    fail(format!(
        "depth.infinite cannot run here: {why}; leave it off"
    ))
}

/// The Brush Modifier and imbed, each only where the kernel gives it a meaning.
fn depth_and_modifier(p: &BrushPreset) -> Result<(), PresetError> {
    range("modifier", p.modifier, -100.0, 100.0)?;
    let kernel = p.kernel.name();
    if p.modifier != 0.0 && p.kernel.modifier_meaning().is_none() {
        return fail(format!(
            "modifier does nothing on the {kernel} kernel yet; leave it at 0"
        ));
    }
    range("depth.imbed", p.depth.imbed, -1.0, 1.0)?;
    let places = !matches!(p.stroke, StrokeSpec::Gesture { .. });
    if p.depth.imbed != 0.0 && !(matches!(p.kernel, KernelSpec::Plane(_)) && places) {
        return fail(format!(
            "depth.imbed only moves the plane of a plane brush that places dabs, not the \
             {kernel} kernel's; leave it at 0"
        ));
    }
    Ok(())
}

/// The `[tip.alpha]` table.
fn alpha(a: &AlphaSpec) -> Result<(), PresetError> {
    let hex = |h: &str| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    match a.hash_named() {
        _ if a.image.trim().is_empty() => {
            return fail("tip.alpha.image must name a picture".into());
        }
        Some(h) if !hex(h) => {
            return fail(format!(
                "tip.alpha.image: a picture named by hash is {HASH_PREFIX} and 64 lowercase hex \
                 digits, got {}",
                a.image
            ));
        }
        _ => {}
    }
    range("tip.alpha.mid", a.mid, 0.0, 1.0)?;
    if a.turn > 3 {
        return fail(format!(
            "tip.alpha.turn must be 0 to 3 quarter turns, got {}",
            a.turn
        ));
    }
    range("tip.alpha.blur", a.blur, 0.0, 64.0)?;
    range("tip.alpha.noise", a.noise, 0.0, 1.0)?;
    if !(1..=1024).contains(&a.noise_scale) {
        let n = a.noise_scale;
        return fail(format!("tip.alpha.noise_scale must be 1 to 1024, got {n}"));
    }
    range("tip.alpha.contrast", a.contrast, 0.0, 16.0)?;
    range("tip.alpha.radial_fade", a.radial_fade, 0.0, 8.0)?;
    fraction("tip.alpha.curve", &a.curve, "a tone curve")?;
    if a.tiles.iter().any(|t| !(1..=64).contains(t)) {
        let [h, v] = a.tiles;
        return fail(format!(
            "tip.alpha.tiles must be 1 to 64 each, got {h} and {v}"
        ));
    }
    range("tip.alpha.adaptive", a.adaptive, 0.0, 1.0)
}

fn kernel(k: &KernelSpec) -> Result<(), PresetError> {
    match k {
        KernelSpec::Offset(o) => {
            if !(1..=2).contains(&o.terms.len()) {
                return fail(format!(
                    "kernel.terms needs 1 or 2 terms, got {}",
                    o.terms.len()
                ));
            }
            for (i, t) in o.terms.iter().enumerate() {
                // Decision 0018, "Offset gains": twice the largest shipped gain.
                range(&format!("kernel.terms[{i}].gain"), t.gain, -4.0, 4.0)?;
                range(
                    &format!("kernel.terms[{i}].exponent"),
                    t.exponent,
                    0.25,
                    16.0,
                )?;
                if t.tangent && t.dir != Direction::ToCenter {
                    return fail(format!(
                        "kernel.terms[{i}].tangent only applies to to_center"
                    ));
                }
            }
            Ok(())
        }
        KernelSpec::Plane(p) => {
            range("kernel.gain", p.gain, 0.0, 100.0)?;
            range("kernel.height", p.height.unwrap_or(0.0), 0.0, 16.0)?;
            range("kernel.depth", p.depth.unwrap_or(0.0), 0.0, 16.0)
        }
        KernelSpec::Smooth(s) => range("kernel.gain", s.gain, 0.0, 100.0),
        KernelSpec::Move(m) => {
            range("kernel.support", m.support, 1.0, 16.0)?;
            range("kernel.poisson", m.poisson, 0.0, 0.5)
        }
        KernelSpec::Hook(h) => range("kernel.pinch", h.pinch, 0.0, 1.0),
        KernelSpec::Attribute(a) => {
            range("kernel.gain", a.gain, 0.0, 100.0)?;
            attribute(a.column, a.op, a.value)
        }
    }
}

fn attribute(column: Column, op: AttrOp, value: f64) -> Result<(), PresetError> {
    if column == Column::Color {
        return fail(
            "kernel.column: a brush paints color with a [paint] table, not the attribute kernel"
                .into(),
        );
    }
    if op != AttrOp::Set && column != Column::Mask {
        return fail("kernel.op blur and sharpen only apply to the mask".into());
    }
    // A ranged column (the mask, a UV hint) takes any number in its range; the others a whole one.
    let (lo, hi, whole) = match column.def().range() {
        Some(r) => (f64::from(r.lo), f64::from(r.hi), false),
        None if column == Column::Hide => (0.0, 1.0, true),
        None => (0.0, f64::from(u32::MAX), true),
    };
    if !(lo..=hi).contains(&value) || (whole && value.fract() != 0.0) {
        let kind = if whole { "a whole number" } else { "a number" };
        return fail(format!(
            "kernel.value must be {kind} from {lo} to {hi}, got {value}"
        ));
    }
    Ok(())
}

/// The `[paint]` table, and mode off, in which a brush only paints (decision 0039, section 8).
fn paint(p: &BrushPreset) -> Result<(), PresetError> {
    let Some(paint) = &p.paint else {
        if p.sculpts() {
            return Ok(());
        }
        return fail("mode off runs no kernel, so the brush needs a [paint] table".into());
    };
    range("paint.color_strength", paint.color_strength, 0.0, 1.0)?;
    if matches!(p.kernel, KernelSpec::Attribute(_)) {
        return fail(
            "[paint] goes on a brush that sculpts or smooths, not on the attribute kernel".into(),
        );
    }
    if matches!(
        p.stroke,
        StrokeSpec::Drag { .. } | StrokeSpec::Gesture { .. }
    ) {
        return fail("[paint] needs a stroke that lays dabs: dab, anchored or curve".into());
    }
    Ok(())
}

fn tip(p: &BrushPreset) -> Result<(), PresetError> {
    let t = &p.tip;
    range("tip.hardness", t.hardness, 0.0, 0.99)?;
    range("tip.focal", t.focal, -100.0, 100.0)?;
    if (t.profile == ProfileName::Custom) != t.points.is_some() {
        return fail("tip.points is required by a custom profile and only allowed there".into());
    }
    if let Some(c) = &t.points {
        fraction("tip.points", c, "a falloff")?;
    }
    fraction("tip.zero_curve", &t.zero_curve, "a falloff")?;
    fraction("tip.pen_curve", &t.pen_curve, "a share of the Edit Curve")?;
    if let Some(a) = &t.alpha {
        alpha(a)?;
    }
    wrap(p)?;
    turns(p)?;
    range(
        "sampling.normal_radius",
        p.sampling.normal_radius,
        0.01,
        1.0,
    )?;
    range(
        "sampling.position_radius",
        p.sampling.position_radius,
        0.01,
        1.0,
    )
}

/// Wrap Mode: 1 to 16, and above 1 only for a brush whose copies can repeat across the tiles
/// (decision 0054, section 4).
fn wrap(p: &BrushPreset) -> Result<(), PresetError> {
    let n = p.tip.wrap;
    if !(1..=16).contains(&n) {
        return fail(format!("tip.wrap must be from 1 to 16, got {n}"));
    }
    let am = &p.automask;
    let why = match (&p.stroke, &p.kernel) {
        _ if n == 1 => return Ok(()),
        (StrokeSpec::Gesture { .. }, _) => "a gesture shapes the region drawn, once, not dabs",
        (_, KernelSpec::Move(m)) if m.select == Select::Connected => {
            "Move with select = \"connected\" moves what is joined to the stroke start, which a \
             copy in another tile does not share"
        }
        _ if am.topology != 0.0 => {
            "the topology automask spares what is not joined to the stroke start, which a copy \
             in another tile does not share"
        }
        _ if am.face_group != 0.0 => {
            "the face group automask spares the stroke start's other face groups, which a copy \
             in another tile does not share"
        }
        _ => return Ok(()),
    };
    fail(format!("tip.wrap must be 1 for this brush, got {n}: {why}"))
}

/// Tilt Brush, Constant Tilt and Align To Path: in range, and at their defaults on a gesture,
/// which lays no dab frame to lean or turn (decision 0058, section 6).
fn turns(p: &BrushPreset) -> Result<(), PresetError> {
    let t = &p.tip;
    range("tip.tilt", t.tilt, -90.0, 90.0)?;
    range("tip.align", t.align, 0.0, 100.0)?;
    let set = [
        ("tip.tilt", t.tilt != 0.0),
        ("tip.constant_tilt", t.constant_tilt),
        ("tip.align", t.align < 100.0),
    ];
    match (&p.stroke, set.iter().find(|(_, on)| *on)) {
        (StrokeSpec::Gesture { .. }, Some((key, _))) => fail(format!(
            "{key} turns or leans a dab's frame, and a gesture lays none; leave it out"
        )),
        _ => Ok(()),
    }
}

fn smoothing(s: Smoothing) -> Result<(), PresetError> {
    range("stroke.smoothing.radius", s.radius, 0.0, 4.0)?;
    range("stroke.smoothing.factor", s.factor, 0.0, 0.99)
}

fn stroke(p: &BrushPreset) -> Result<(), PresetError> {
    match p.stroke {
        StrokeSpec::Dab {
            spacing,
            tap,
            smoothing: s,
            ..
        } => {
            range("stroke.spacing", spacing, 0.01, 4.0)?;
            range("stroke.tap", tap, 0.0, 4.0)?;
            smoothing(s)?;
        }
        StrokeSpec::Drag { smoothing: s } | StrokeSpec::Gesture { smoothing: s, .. } => {
            smoothing(s)?;
        }
        StrokeSpec::Anchored {
            tap, smoothing: s, ..
        } => {
            range("stroke.tap", tap, 0.0, 4.0)?;
            smoothing(s)?;
        }
        StrokeSpec::Curve {
            spacing,
            smoothing: s,
            ..
        } => {
            range("stroke.spacing", spacing, 0.01, 4.0)?;
            smoothing(s)?;
        }
    }
    let fits = match (&p.kernel, p.stroke) {
        (KernelSpec::Move(_), StrokeSpec::Drag { .. }) => true,
        (KernelSpec::Move(_), _) | (_, StrokeSpec::Drag { .. }) => false,
        (KernelSpec::Hook(_), s) => matches!(s, StrokeSpec::Dab { .. }),
        (KernelSpec::Plane(_), StrokeSpec::Gesture { shape, .. }) => shape == Shape::Curve,
        (KernelSpec::Attribute(a), StrokeSpec::Gesture { shape, .. }) => {
            a.column == Column::Hide && matches!(shape, Shape::Polygon | Shape::Rect)
        }
        (_, StrokeSpec::Gesture { .. }) => false,
        _ => true,
    };
    if !fits {
        return fail(
            "stroke.class does not fit the kernel: move needs drag, drag needs move, hook needs \
             dab, and gestures are a curve over plane (clip) or an outline or rectangle over \
             hide (select)"
                .into(),
        );
    }
    // Square and Center shape a rectangle only (decision 0051, section 3).
    if let StrokeSpec::Gesture {
        shape,
        square,
        center,
        ..
    } = p.stroke
        && shape != Shape::Rect
        && (square || center)
    {
        let key = if square { "square" } else { "center" };
        return fail(format!(
            "stroke.{key} shapes only a rectangle (shape = \"rect\")"
        ));
    }
    Ok(())
}

fn rest(p: &BrushPreset) -> Result<(), PresetError> {
    let (post, auto) = (&p.post, &p.automask);
    range("post.auto_smooth", post.auto_smooth, 0.0, 100.0)?;
    range("post.gravity", post.gravity, -1.0, 1.0)?;
    let g = post.gravity_dir;
    if !(g.iter().all(|x| x.is_finite()) && g.iter().any(|&x| x != 0.0)) {
        return fail("post.gravity_dir must be a finite direction, not zero".into());
    }
    if post.topology_rake != 0.0 {
        return fail("post.topology_rake is not built yet".into());
    }
    let ld = &p.live_detail;
    if let Some(size) = ld.size
        && !(size > 0.0 && size.is_finite())
    {
        return fail(format!(
            "live_detail.size must be a positive number, got {size}"
        ));
    }
    if let Some(merge) = ld.merge {
        range("live_detail.merge", merge, 0.0, 1.0)?;
    }
    range("automask.backface", auto.backface, 0.0, 1.0)?;
    range("automask.cavity", auto.cavity, -1.0, 1.0)?;
    range("automask.topology", auto.topology, 0.0, 16.0)?;
    range("automask.face_group", auto.face_group, 0.0, 1.0)
}

/// Smooth's Weight Strength and Min Connected Points (decision 0049, sections 3 and 5): in
/// range, and Weight Strength only where the mode reads it.
fn smooth_keys(k: &KernelSpec) -> Result<(), PresetError> {
    let KernelSpec::Smooth(s) = k else {
        return Ok(());
    };
    range("kernel.weight", s.weight, 0.0, 1.0)?;
    range("kernel.min_connected", s.min_connected as f32, 1.0, 64.0)?;
    if s.weight < 1.0 && s.mode != crate::brush::SmoothMode::GroupBorders {
        return fail("kernel.weight is read by mode group_borders only; leave it at 1".into());
    }
    Ok(())
}
