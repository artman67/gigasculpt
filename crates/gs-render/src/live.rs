//! Live Boolean's stand-ins (decision 0057, section 9, as amended for LB7). A tinted mesh (a
//! cut wall or an issue mark) is drawn by the geometry pass like any part, with its paint times
//! its tint and nudged nearer in depth by 1 part in 65,536, so it wins over the copy of the same
//! faces under it: bit 3 of its flags and its tint in the bytes above them (see
//! [`Renderer::drawn`]). A ghost (a Difference or Intersect operand standing in for a cut) is
//! left out of the geometry pass and drawn after the display pass as an outline only, about 2
//! pixels wide just outside its silhouette and seen through everything (`shaders/ghost.wgsl`):
//! each ghost flat in its color into a cover target the frame's size, made on first use, then a
//! ring around the cover into the frame.

use crate::frame::FrameTarget;
use crate::pipeline::{
    FRAME_WGSL, Path, UNIFORM, bind_group, bind_layout, fullscreen, layout_entry, module, texel,
};
use crate::pool::GpuMesh;
use crate::snapshot::{Renderer, draw};
use crate::ui::UI_FORMAT;

/// The active part's outline, #FFAA40 (display.wgsl's `OUTLINE`, in sRGB bytes): a ghost that
/// is the active part takes it in a view whose outline is on.
const ORANGE: [u8; 3] = [255, 170, 64];

/// The cover target: each ghost's color where it covers the pixel, alpha 0 elsewhere.
const COVER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The ghosts' two pipelines and the ring pass's group layout: the frame's uniforms and the
/// cover. Made the first time a frame draws a ghost.
pub(crate) struct Ghosts {
    cover: wgpu::RenderPipeline,
    ring: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl GpuMesh {
    /// This mesh with its paint times `tint` (sRGB bytes) and nudged nearer in depth, so it
    /// shows over a copy of the same faces: a cut wall or an issue mark (decision 0057, section
    /// 9); `None` draws it plainly. Setting it sends nothing to the GPU.
    #[must_use]
    pub fn tinted(mut self, tint: Option<[u8; 3]>) -> GpuMesh {
        self.placement.tint = tint;
        self
    }

    /// This mesh as an outline only, in `color` (sRGB bytes), about 2 pixels wide just outside
    /// its silhouette and seen through everything; in the active part's orange instead while it
    /// is marked in a view whose outline is on. `None` draws its surface. Setting it sends
    /// nothing to the GPU.
    #[must_use]
    pub fn ghosted(mut self, color: Option<[u8; 3]>) -> GpuMesh {
        self.placement.ghost = color;
        self
    }
}

/// A color's flags word for the geometry pass (see [`Renderer::drawn`]): bit 3 set and the
/// color's bytes above the flags' own byte, red lowest.
pub(crate) fn tint_flags([r, g, b]: [u8; 3]) -> u32 {
    8 | u32::from(r) << 8 | u32::from(g) << 16 | u32::from(b) << 24
}

impl Ghosts {
    fn new(r: &Renderer) -> Ghosts {
        use wgpu::ShaderStages as S;
        let gpu = &r.gpu;
        let source = format!(
            "{}\n{}\n{}",
            crate::deform::DEFORM_WGSL,
            include_str!("shaders/geometry.wgsl"),
            include_str!("shaders/ghost.wgsl")
        );
        let entries = [
            layout_entry(0, S::VERTEX_FRAGMENT, UNIFORM),
            layout_entry(1, S::FRAGMENT, texel(false)),
        ];
        let layout = bind_layout(gpu, "ghost ring", &entries);
        let ring = fullscreen(
            gpu,
            "ghost ring",
            (&source, "fs_ring"),
            &layout,
            &[UI_FORMAT],
        );
        let shader = module(gpu, "ghost cover", format!("{FRAME_WGSL}\n{source}"));
        let p = &r.pipelines;
        let groups = [&p.frame, &p.mask, &p.part, &p.deform].map(Some);
        let entries = (&shader, "vs", "fs_ghost");
        let cover = crate::peel::pipeline(gpu, entries, &groups, &[COVER_FORMAT], None);
        Ghosts {
            cover,
            ring,
            layout,
        }
    }
}

impl Renderer {
    /// Draws `ghosts` over the frame just displayed, as outlines (see the module's notes), at
    /// `viewport` (clipped to the frame) with the frame's `uniforms`. `outline` is the view's:
    /// with it on, a marked ghost is outlined in orange. Nothing without ghosts.
    pub(crate) fn ghost_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        (target, [x, y, w, h]): (&FrameTarget, [u32; 4]),
        ghosts: &[&GpuMesh],
        uniforms: &wgpu::Buffer,
        outline: bool,
    ) {
        if ghosts.is_empty() || w == 0 || h == 0 {
            return;
        }
        let ghost = self.ghosts.get_or_init(|| Ghosts::new(self));
        let size = target.size();
        let cover = (target.ghost).get_or_init(|| self.cover_target([size.width, size.height]));
        let colors: Vec<_> = (ghosts.iter())
            .map(|m| {
                let p = m.placement;
                let color = if p.marked && outline {
                    ORANGE
                } else {
                    p.ghost.unwrap_or(ORANGE)
                };
                (Path::Plain, tint_flags(color))
            })
            .collect();
        let (parts, stride) = self.parts(ghosts, &colors);
        let frame = bind_group(
            &self.gpu,
            &self.pipelines.frame,
            &[uniforms.as_entire_binding()],
        );
        let label = ("ghost cover", wgpu::Color::TRANSPARENT);
        let mut pass = crate::passes::begin(encoder, label, &[cover], None);
        pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
        pass.set_scissor_rect(x, y, w, h);
        pass.set_pipeline(&ghost.cover);
        pass.set_bind_group(0, &frame, &[]);
        for (i, mesh) in ghosts.iter().enumerate() {
            pass.set_bind_group(2, &parts, &[(i as u64 * stride) as u32]);
            draw(&mut pass, mesh, Path::Plain, None);
        }
        drop(pass);
        let reads = [
            uniforms.as_entire_binding(),
            wgpu::BindingResource::TextureView(cover),
        ];
        let group = bind_group(&self.gpu, &ghost.layout, &reads);
        let color = target.view(UI_FORMAT);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ghost ring"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_scissor_rect(x, y, w, h);
        pass.set_pipeline(&ghost.ring);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// The cover target for frames of `size`.
    fn cover_target(&self, size: [u32; 2]) -> wgpu::TextureView {
        crate::peel::target_view(&self.gpu, "ghost cover", COVER_FORMAT, size)
    }
}
