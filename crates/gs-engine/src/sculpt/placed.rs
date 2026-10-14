//! A stroke on a part with a transform (decision 0029, section 3): the call's scene-space
//! arguments mapped into the part's own space before gs-core sees them, and what the stroke
//! reports mapped back out, so gs-core's stroke code never knows the part moved. Screen samples,
//! sizes in pixels and fractions of the brush radius mean the same in both spaces, because the
//! part's camera sees the same picture. Mirror planes and lock axes are the part's own (section
//! 7), as Blender's sculpt mode keeps them in object space.

use std::borrow::Cow;

use gs_core::camera::Camera;
use gs_core::transform::Similarity;
use gs_protocol::{Length, Vec3};

use super::DetailMode;
use super::aim::Finished;
use super::stroke::{BrushStroke, DEFAULT_SIZE};
use crate::document::Part;

/// A screen stroke's camera and the size of its image.
type Aimed = Option<(Camera, [f32; 2])>;

impl BrushStroke {
    /// The call as `part` sees it, with `camera` as the part sees it: world samples and
    /// `view_dir` mapped in, the brush size resolved in the scene (a world stroke's default
    /// too) then divided by the scale, a constant Live Detail size divided by it. At the
    /// identity, the call itself and the camera as it is, before any work.
    pub(crate) fn in_part(&self, part: &Part, camera: Aimed) -> (Cow<'_, Self>, Aimed) {
        let t = part.transform();
        if t.is_identity() {
            return (Cow::Borrowed(self), camera);
        }
        let mut args = self.clone();
        if self.camera.is_none() {
            for s in args.samples.iter_mut().filter(|s| s.position.len() == 3) {
                s.position = t.part_point(s.input().at).to_array().to_vec();
            }
            args.view_dir = (self.view_dir)
                .map(|Vec3(d)| Vec3(t.part_direction(gs_core::Vec3::from_array(d)).to_array()));
        }
        let size = match self.camera {
            None => Some(self.size.unwrap_or(DEFAULT_SIZE)),
            Some(_) => self.size,
        };
        args.size = size.map(|l| Length::Units(part.length(l).part));
        if let Some(ld) = &mut args.live_detail
            && ld.mode == Some(DetailMode::Constant)
        {
            ld.size = ld.size.map(|s| t.part_length(s));
        }
        (
            Cow::Owned(args),
            camera.map(|(c, size)| (t.part_camera(&c), size)),
        )
    }
}

impl Finished {
    /// What the stroke did, in the scene: the radius and gap scaled, where it first touched
    /// mapped out.
    pub(crate) fn in_scene(self, t: &Similarity) -> Self {
        if t.is_identity() {
            return self;
        }
        Self {
            radius: self.radius.map(|r| t.length(r)),
            gap: self.gap.map(|g| t.length(g)),
            first: self.first.map(|p| t.point(p)),
            ..self
        }
    }
}
