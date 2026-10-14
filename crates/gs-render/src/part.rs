//! Each part's drawing state (decision 0033, section 5): where its mesh is drawn and whether
//! (decision 0029), whether it is the active part, and what its surface's paint is drawn from
//! (decision 0039, section 9). The geometry pass takes it per mesh as the `Part` uniform: the
//! model matrix, part to scene (to the culling pass, the camera seen from the part), and the
//! flags, which it writes into a one-byte target per pixel, from which the display pass draws
//! the active part's outline. A mesh that is not shown stays on the GPU and draws nothing.

use crate::error::RenderError;
use crate::math::{M4, mul4};
use crate::pipeline::Path;
use crate::pool::GpuMesh;
use crate::snapshot::{Renderer, View, Wire};
use gs_core::transform::Similarity;
use wgpu::util::DeviceExt;

/// Where a mesh is drawn, and whether (decision 0029), and how: the part's drawing state
/// (decision 0033, section 5), which part transparency and ghosting extend later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    /// Part to scene.
    pub transform: Similarity,
    /// False keeps the mesh on the GPU but draws nothing, so showing it again sends nothing.
    pub shown: bool,
    /// The active part: outlined in a view whose `outline` is on, when it draws two parts or
    /// more.
    pub marked: bool,
    /// Whether the part shows its point colors (decision 0039, sections 9 and 10); false shows
    /// its material only.
    pub painted: bool,
    /// Live Boolean's tint and ghost color (sRGB bytes, see `live.rs`).
    pub tint: Option<[u8; 3]>,
    pub ghost: Option<[u8; 3]>,
    /// How it joins Live Boolean's peel (`peel.rs`), if it does.
    pub peel: Option<crate::peel::Operand>,
}

impl Default for Placement {
    fn default() -> Self {
        Placement {
            transform: Similarity::IDENTITY,
            shown: true,
            marked: false,
            painted: true,
            tint: None,
            ghost: None,
            peel: None,
        }
    }
}

/// What a drawn part's surface color comes from (decision 0039, section 9), in bits 1 and 2 of
/// its flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The material alone: white paint.
    Material = 0,
    /// The point colors, interpolated across each triangle.
    Points = 1,
    /// The face group colors, one per triangle.
    Groups = 2,
    /// The size of each triangle on screen (the density view).
    Density = 3,
}

impl GpuMesh {
    /// This mesh drawn at `transform` (part to scene), or kept but not drawn when `shown` is
    /// false. Setting it sends nothing to the GPU.
    #[must_use]
    pub fn placed(mut self, transform: Similarity, shown: bool) -> GpuMesh {
        self.placement = Placement {
            transform,
            shown,
            ..self.placement
        };
        self
    }

    /// This mesh marked as the active part (decision 0033, section 5), or not: a view whose
    /// `outline` is on outlines it while it draws two parts or more. Setting it sends nothing
    /// to the GPU.
    #[must_use]
    pub fn marked(mut self, marked: bool) -> GpuMesh {
        self.placement.marked = marked;
        self
    }

    /// This mesh showing its point colors (the default), or its material only (decision 0039,
    /// section 9): Colorize's switch. A mesh without colors shows its material either way, and
    /// a view showing face groups shows them either way. Setting it sends nothing to the GPU.
    #[must_use]
    pub fn painted(mut self, painted: bool) -> GpuMesh {
        self.placement.painted = painted;
        self
    }

    /// Whether the mesh is drawn (see [`GpuMesh::placed`]).
    pub fn shown(&self) -> bool {
        self.placement.shown
    }

    /// The camera and world-to-clip matrix the culling pass tests this mesh's part-space
    /// chunks against: `camera` and `view_proj` seen from the part.
    pub(crate) fn part_view(&self, camera: &crate::Camera, view_proj: &M4) -> (crate::Camera, M4) {
        let t = &self.placement.transform;
        if t.is_identity() {
            return (*camera, *view_proj);
        }
        (t.part_camera(camera), mul4(view_proj, &t.matrix()))
    }
}

/// How a mesh is drawn in a view: its path and its part flags (see [`Renderer::drawn`]).
pub(crate) type Drawn = (Path, u32);

/// Bytes of one mesh's `Part` in the geometry pass's group 2: its model matrix, then its four
/// words of flags.
pub(crate) const PART_BYTES: u64 = 80;

/// Bytes between two meshes' `Part`s in the frame's buffer: `PART_BYTES` rounded up to the
/// device's alignment for dynamic uniform offsets.
fn part_stride(gpu: &crate::Gpu) -> u64 {
    let align = u64::from(gpu.device.limits().min_uniform_buffer_offset_alignment);
    PART_BYTES.next_multiple_of(align.max(1))
}

impl Renderer {
    /// How `mesh` is drawn in `view`: its path and its part flags (bit 0 the mark, bits 1 and 2
    /// the [`Source`], bit 3 and the bytes above it a tint, `live.rs`). The density view colors
    /// every part by its triangles' size. A view showing face groups (or a wire with its fill)
    /// shows them on every part that has them, and the material on the rest; otherwise a painted part with point colors shows them. A
    /// triangle's number and corners reach the shader only on the barycentric and
    /// vertex-pulling paths, so a part showing its groups or density takes one of them, its
    /// wire off unless the view's is on; on a device with neither for a part that large, it
    /// shows its material.
    pub(crate) fn drawn(&self, view: &View, mesh: &GpuMesh) -> Result<Drawn, RenderError> {
        let path = self.path(view.wire, mesh)?;
        let (streams, placement) = (&mesh.streams, &mesh.placement);
        let groups = view.face_groups || (view.wire != Wire::Off && view.wire_style.fill);
        let source = if view.density {
            Source::Density
        } else if groups {
            if streams.groups.is_some() {
                Source::Groups
            } else {
                Source::Material
            }
        } else if placement.painted && streams.colors.is_some() {
            Source::Points
        } else {
            Source::Material
        };
        let per_triangle = matches!(source, Source::Groups | Source::Density);
        let (path, source) = match path {
            Path::Plain if per_triangle && self.gpu.capabilities().barycentric_wireframe => {
                (Path::Barycentric, source)
            }
            Path::Plain if per_triangle && mesh.pull_group.is_some() => (Path::Pulled, source),
            Path::Plain if per_triangle => (path, Source::Material),
            _ => (path, source),
        };
        let tint = placement.tint.map_or(0, crate::live::tint_flags);
        let flags = u32::from(placement.marked) | (source as u32) << 1 | tint;
        Ok((path, flags))
    }

    /// The geometry pass's group 2: every mesh's `Part`, its model matrix (part to scene) and
    /// its flags (from [`Renderer::drawn`]), one per `stride` bytes, bound at mesh `i` with
    /// offset `i * stride`.
    pub(crate) fn parts(&self, meshes: &[&GpuMesh], drawn: &[Drawn]) -> (wgpu::BindGroup, u64) {
        let words: Vec<[u32; 4]> = (drawn.iter())
            .map(|&(_, flags)| [flags & 0xFF, flags >> 8, 0, 0])
            .collect();
        self.parts_of(meshes, &words)
    }

    /// [`Renderer::parts`] with each mesh's four words of flags as given (the peel's own).
    pub(crate) fn parts_of(
        &self,
        meshes: &[&GpuMesh],
        words: &[[u32; 4]],
    ) -> (wgpu::BindGroup, u64) {
        let stride = part_stride(&self.gpu);
        let mut bytes = vec![0u8; (stride as usize) * meshes.len().max(1)];
        let slots = bytes.chunks_mut(stride as usize);
        for ((mesh, w), slot) in meshes.iter().zip(words).zip(slots) {
            let m = mesh.placement.transform.matrix();
            slot[..64].copy_from_slice(bytemuck::bytes_of(&m));
            slot[64..80].copy_from_slice(bytemuck::bytes_of(w));
        }
        let device = &self.gpu.device;
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("parts"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("parts"),
            layout: &self.pipelines.part,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(PART_BYTES),
                }),
            }],
        });
        (group, stride)
    }
}
