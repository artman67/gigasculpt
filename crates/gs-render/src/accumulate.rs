//! Still-camera accumulation (research 11 section 8, decision 0019): while nothing changes,
//! each frame is drawn with the camera shifted by a fraction of a pixel and averaged into the
//! last, so edges and the occlusion's noise smooth out over a few frames and then the picture
//! stops changing, as Blender's Workbench samples do. Any change starts again from one
//! unjittered sample.

use crate::Camera;
use crate::display::Display;
use crate::occlusion::Occlusion;
use crate::pool::Sides;
use crate::snapshot::{Shading, View, Wire};
use gs_core::transform::Similarity;

/// Element `k` of the van der Corput sequence in `base`: `k` written in `base`, mirrored about
/// the point. Exact for the indices used here.
fn radical_inverse(mut k: u32, base: u32) -> f32 {
    let (mut inv, mut f) = (0.0f32, 1.0f32 / base as f32);
    while k > 0 {
        inv += (k % base) as f32 * f;
        k /= base;
        f /= base as f32;
    }
    inv
}

/// The sub-pixel offset of sample `k`, in pixels: none for the first sample, then the Halton
/// (2, 3) points centered on the pixel.
pub(crate) fn jitter(k: u32) -> [f32; 2] {
    if k == 0 {
        return [0.0; 2];
    }
    [radical_inverse(k, 2) - 0.5, radical_inverse(k, 3) - 0.5]
}

/// Everything a frame's picture depends on. Two frames with equal keys show the same scene, so
/// their samples may be averaged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Key {
    camera: Camera,
    /// Shading mode and, for MatCaps, which one.
    shading: (u32, u64),
    /// For lit shading, the environment and the uniforms of the material and lights.
    lit: Option<(u64, Vec<u32>)>,
    wire: Wire,
    wire_style: Option<crate::WireStyle>,
    display: Display,
    occlusion: Occlusion,
    grid: Option<crate::FloorGrid>,
    background: crate::Background,
    mask_opacity: f32,
    outline: bool,
    viewport: [u32; 4],
    /// Each drawn mesh's stamp, sides, place (decision 0029) and part flags: its mark (decision
    /// 0033) and what its paint is drawn from (decision 0039, section 9).
    meshes: Vec<((u64, u64), Sides, Similarity, u32)>,
}

impl Key {
    pub fn of(
        view: &View,
        (viewport, lit): ([u32; 4], Option<(u64, Vec<u32>)>),
        meshes: &[((u64, u64), Sides, Similarity, u32)],
    ) -> Key {
        let shading = match view.shading {
            Shading::Clay => (0, 0),
            Shading::MatCap(m) => (1, m.id),
            Shading::Zebra => (2, 0),
            Shading::Normals => (3, 0),
            Shading::Lit(_) => (4, 0),
            Shading::Flat(base) => {
                let [r, g, b] = base.map(|c| u64::from(c.to_bits()));
                (5, r ^ g.rotate_left(21) ^ b.rotate_left(42))
            }
        };
        Key {
            camera: view.camera,
            shading,
            lit,
            wire: view.wire,
            wire_style: (view.wire != Wire::Off).then_some(view.wire_style),
            display: view.display,
            occlusion: view.occlusion,
            grid: view.grid.then_some(view.floor),
            background: view.background,
            mask_opacity: view.mask_opacity,
            outline: view.outline,
            viewport,
            meshes: meshes.to_vec(),
        }
    }
}

/// What a frame target's next frame does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// Draw sample `k` and average it in (`k` = 0 starts over).
    Sample(u32),
    /// Every sample is in: show the last sample's picture again.
    Hold,
}

/// A frame target's accumulation: what its last frame showed, how many samples are averaged,
/// and which history texture holds the mean.
#[derive(Debug, Default)]
pub(crate) struct Accumulation {
    key: Option<Key>,
    done: u32,
    /// Index of the history texture holding the current mean.
    pub current: usize,
}

impl Accumulation {
    /// The next step for a frame with `key`, accumulating up to `samples`.
    pub fn next(&mut self, key: Key, samples: u32) -> Step {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.done = 0;
        }
        if self.done >= samples {
            return Step::Hold;
        }
        self.done += 1;
        Step::Sample(self.done - 1)
    }

    /// Whether every sample of the last frame's scene is in.
    pub fn settled(&self, samples: u32) -> bool {
        self.key.is_some() && self.done >= samples
    }

    /// The history to read and the one to write for a frame doing `step`: a sample writes the
    /// other one, which becomes current; a settled frame writes none.
    pub fn histories(&mut self, step: Step) -> (usize, Option<usize>) {
        let read = self.current;
        if step == Step::Hold {
            return (read, None);
        }
        self.current = 1 - read;
        (read, Some(self.current))
    }
}
