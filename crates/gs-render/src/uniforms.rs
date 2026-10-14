//! The frame's uniforms (`Frame` in `shaders/frame.wgsl`): one buffer per sample, read by every
//! pass of the frame (decision 0019).

use bytemuck::{Pod, Zeroable};
use gs_core::material::Cavity;

use crate::accumulate::jitter;
use crate::display::{ViewTransform, decode};
use crate::grid;
use crate::math::M4;
use crate::snapshot::{Background, Shading, View, Wire, WireStyle};
use crate::{Camera, Projection};

/// Matches `Frame` in `shaders/frame.wgsl`; see it for what each field holds.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct FrameUniforms {
    pub view: M4,
    pub proj: M4,
    viewport: [f32; 4],
    albedo: [f32; 4],
    background: [f32; 4],
    shading: [u32; 4],
    occlusion: [f32; 4],
    counts: [u32; 4],
    display: [f32; 4],
    flags: [u32; 4],
    grid: [f32; 4],
    grid_plane: [f32; 4],
    grid_past: [f32; 4],
    grid_eye: [f32; 4],
    grid_center: [f32; 4],
    matcap: [f32; 4],
    wire: [f32; 4],
    gradient: [f32; 4],
    cavity: [f32; 4],
}

/// The kind of pixel the lighting pass writes for each shading (the light target's alpha):
/// 1 physical light, 2 a display-referred image, 3 a display aid.
fn kind(shading: Shading) -> u32 {
    match shading {
        Shading::MatCap(m) if m.display_referred => 2,
        Shading::MatCap(_) | Shading::Clay | Shading::Zebra | Shading::Lit(_) => 1,
        Shading::Normals | Shading::Flat(_) => 3,
    }
}

/// The camera matrices of sample `sample` for a viewport `[x, y, w, h]`: world to view, and
/// view to clip with the sample's sub-pixel jitter.
pub(crate) fn matrices(
    camera: &Camera,
    viewport: [u32; 4],
    points: &[[f32; 3]],
    sample: u32,
) -> (M4, M4) {
    let [w, h] = [viewport[2].max(1), viewport[3].max(1)].map(|v| v as f32);
    let (view, mut proj) = camera.matrices(w / h, points);
    // Clip x and y move by the offset times w, so every depth shifts by the same pixels.
    let [jx, jy] = jitter(sample);
    let (dx, dy) = (2.0 * jx / w, -2.0 * jy / h);
    for column in &mut proj {
        column[0] += dx * column[3];
        column[1] += dy * column[3];
    }
    (view, proj)
}

/// What the passes need to know about one sample, besides the camera.
pub(crate) struct Sample<'a> {
    pub view: &'a View<'a>,
    pub viewport: [u32; 4],
    /// Every drawn chunk's bounds corners.
    pub points: &'a [[f32; 3]],
    pub index: u32,
    /// The accumulation weight: 1 / (sample + 1), or 0 to hold the history.
    pub weight: f32,
    /// Occlusion slices and steps per side, and frame pixels per occlusion texel.
    pub quality: (u32, u32, u32),
    /// Draw the active part's outline (decision 0033, section 5): the view's `outline`, with
    /// two parts or more drawn and one of them marked.
    pub outline: bool,
}

impl FrameUniforms {
    pub fn new(s: &Sample) -> FrameUniforms {
        let view = s.view;
        let (view_m, proj) = matrices(&view.camera, s.viewport, s.points, s.index);
        let (albedo, roughness, crease, metalness, transition) = match view.shading {
            Shading::MatCap(m) => {
                let crease = m.crease.is_some();
                (m.albedo, m.roughness, crease, m.metalness, m.transition)
            }
            Shading::Flat(base) => (base, 1.0, false, 0.0, 0.0),
            _ => ([1.0; 3], 1.0, false, 0.0, 0.0),
        };
        let ortho = view.camera.projection == Projection::Orthographic;
        let display = view.display;
        let transform = match display.transform {
            ViewTransform::PbrNeutral => 0,
            ViewTransform::Agx => 1,
            ViewTransform::Standard => 2,
        };
        let [r, g, b] = decode(view.background.color);
        let size = [s.viewport[2], s.viewport[3]];
        let floor = (view.grid).then(|| grid::Floor::of(&view.camera, s.points, size, &view.floor));
        let floor = floor.flatten().unwrap_or(grid::Floor::NONE);
        let (style, [px, py, pz], [cx, cy, cz]) = (view.floor, floor.planes, floor.center);
        let (occlusion, cavity) = occlusion(view, s.points, roughness);
        FrameUniforms {
            view: view_m,
            proj,
            viewport: s.viewport.map(|v| v as f32),
            albedo: [albedo[0], albedo[1], albedo[2], metalness],
            background: [r, g, b, 1.0],
            shading: [
                shading_mode(view.shading),
                kind(view.shading),
                u32::from(crease),
                u32::from(ortho),
            ],
            occlusion,
            counts: [s.quality.0, s.quality.1, s.index, s.quality.2],
            display: [
                display.gain(),
                s.weight,
                view.mask_opacity,
                f32::from(u8::from(view.wire != Wire::Off && view.wire_style.line)),
            ],
            flags: [
                transform,
                u32::from(display.dither),
                if view.grid { style.plane_bits() } else { 0 },
                u32::from(s.outline),
            ],
            grid: [floor.spacing, floor.fine, 1.0 / floor.ratio, floor.half],
            grid_plane: [px, py, pz, style.fill_opacity()],
            grid_past: [floor.past[0], floor.past[1], floor.past[2], style.tint],
            grid_eye: [
                floor.eye[0],
                floor.eye[1],
                floor.eye[2],
                f32::from(style.fill == 3),
            ],
            grid_center: [cx, cy, cz, 0.0],
            matcap: [
                if view.matcap_flip { -1.0 } else { 1.0 },
                transition,
                0.0,
                0.0,
            ],
            wire: wire(view.wire_style),
            gradient: gradient(view.background),
            cavity,
        }
    }

    /// Whether the occlusion passes have anything to do.
    pub fn occludes(&self) -> bool {
        self.occlusion[0] > 0.0 && self.occlusion[1] > 0.0
    }
}

/// `Frame`'s occlusion and cavity lanes for `view` of a scene with these bounds corners, whose
/// look has this perceptual `roughness`. Occlusion darkens environment light, which every
/// physical look has; the material's cavity settings scale it in MatCap and Lit shading only
/// (decision 0043, step (c)).
fn occlusion(view: &View, points: &[[f32; 3]], roughness: f32) -> ([f32; 4], [f32; 4]) {
    let cavity = match view.shading {
        Shading::MatCap(_) | Shading::Lit(_) => view.occlusion.cavity,
        _ => Cavity::NEUTRAL,
    };
    let radius = match view.shading {
        Shading::Normals | Shading::Flat(_) => 0.0,
        _ => view.occlusion.radius_for(points) * cavity.radius.max(0.0),
    };
    let strength = if radius > 0.0 {
        view.occlusion.strength.clamp(0.0, 1.0) * cavity.intensity.clamp(0.0, 2.0)
    } else {
        0.0
    };
    let [r, g, b] = cavity.color.0;
    (
        [radius, strength, roughness * roughness, 0.0],
        [r, g, b, cavity.colorize.clamp(0.0, 1.0)],
    )
}

/// The wire's edge color, display-linear, and how much an edge covers the surface.
fn wire(style: WireStyle) -> [f32; 4] {
    let [r, g, b] = decode(style.color);
    [r, g, b, style.opacity.clamp(0.0, 1.0)]
}

/// The background's gradient: how much of it shows, its center and its softness, each 0 to 1.
fn gradient(b: Background) -> [f32; 4] {
    [b.gradient, b.center, b.softness, 0.0].map(|v| v.clamp(0.0, 1.0))
}

fn shading_mode(shading: Shading) -> u32 {
    match shading {
        Shading::Clay => 0,
        Shading::MatCap(_) => 1,
        Shading::Zebra => 2,
        Shading::Normals => 3,
        Shading::Lit(_) => 4,
        Shading::Flat(_) => 5,
    }
}
