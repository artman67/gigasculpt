//! GPU culling per chunk (ARCHITECTURE 6, research 02 section 3.4): a compute pass tests each
//! chunk's bounding sphere against the view frustum and, when only front faces are drawn, its
//! normal cone against the eye, then writes the draw records of the chunks that pass for one
//! `multi_draw_indexed_indirect_count`.
//!
//! The pass runs as one workgroup that counts, prefix-sums and writes, so visible chunks keep
//! their order. A culled frame therefore draws exactly the triangles of an unculled one that can
//! reach the screen, in the same order, and renders the same bytes: tests compare the two.
//!
//! Every test errs toward drawing. A chunk is skipped only when its whole sphere is outside one
//! of the frustum's four side planes by a margin, or when every triangle faces away from the eye
//! by more than about one degree, so rounding in the GPU's own facing test cannot disagree with
//! ours. (There are no near and far planes: the depth range is fitted around every chunk.)

use crate::error::RenderError;
use crate::frame::Scene;
use crate::math::{M4, V3, above, add, dot, normalize_or, scale, sub};
use crate::mesh::Bounds;
use crate::pipeline::Path;
use crate::pool::{CullBuffers, GpuMesh, Sides};
use crate::snapshot::{RenderSize, Renderer, View};
use crate::timing::{Probe, Timing};
use crate::{Camera, Projection};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

impl GpuMesh {
    /// The culling buffers when this mesh drawn by `path` is culled on the GPU: the device can
    /// draw with a GPU-written count, culling is on, and the path is indexed. A vertex-pulled
    /// draw always draws every chunk, and a frame with one culls nothing (see `draw_scene`).
    /// A previewed mesh is not culled: its records hold the open's boxes and cones.
    pub(crate) fn culls(&self, path: Path) -> Option<&CullBuffers> {
        let on = self.culling() && path != Path::Pulled && !self.deform.on;
        (self.cull.as_ref()).filter(|_| on)
    }
}

/// What one frame of a mesh draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    /// Chunks drawn: those the culling pass kept, or every chunk when the mesh is not culled.
    pub chunks: u32,
    /// Triangles that reached the clipper, counted by a pipeline statistics query; `None` on
    /// an adapter without one.
    pub triangles: Option<u64>,
}

impl Renderer {
    /// Records the culling pass for `mesh` seen by `camera` through `view_proj`.
    pub(crate) fn encode_cull(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &GpuMesh,
        buffers: &CullBuffers,
        (camera, view_proj): (&Camera, &M4),
        timing: Option<&Timing>,
    ) {
        let front_only = mesh.sides() == Sides::Front;
        let uniform = CullCamera::new(camera, view_proj, mesh.chunks().len() as u32, front_only);
        let device = &self.gpu.device;
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cull camera"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries: Vec<wgpu::BindGroupEntry> =
            [&uniform, &buffers.chunks, &buffers.draws, &buffers.drawn]
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull"),
            layout: &self.pipelines.cull_layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cull"),
            timestamp_writes: timing.and_then(|t| t.compute("cull")),
        });
        pass.set_pipeline(&self.pipelines.cull);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// What a frame of `mesh` seen by `view` at `size` draws, read back from the GPU after
    /// drawing it: for tests and performance reports.
    pub fn count_drawn(
        &self,
        mesh: &GpuMesh,
        view: &View,
        size: RenderSize,
    ) -> Result<Drawn, RenderError> {
        use wgpu::BufferUsages as B;
        let culled = mesh.culls(self.drawn(view, mesh)?.0);
        let target = self.frame_target(size)?;
        let scene = Scene {
            meshes: &[mesh],
            view: *view,
            viewport: [0, 0, size.width, size.height],
        };
        let device = &self.gpu.device;
        let stats = self.gpu.capabilities().pipeline_statistics.then(|| {
            device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("drawn triangles"),
                ty: wgpu::QueryType::PipelineStatistics(
                    wgpu::PipelineStatisticsTypes::CLIPPER_INVOCATIONS,
                ),
                count: 1,
            })
        });
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage,
                mapped_at_creation: false,
            })
        };
        let readback = buffer("drawn readback", B::COPY_DST.union(B::MAP_READ));
        let resolved = buffer("drawn query", B::QUERY_RESOLVE.union(B::COPY_SRC));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let probe = Probe {
            stats: stats.as_ref(),
            timing: None,
        };
        self.draw_scene(&mut encoder, &target, &scene, probe)?;
        if let Some(buffers) = culled {
            encoder.copy_buffer_to_buffer(&buffers.drawn, 0, &readback, 0, 4);
        }
        if let Some(set) = &stats {
            encoder.resolve_query_set(set, 0..1, &resolved, 0);
            encoder.copy_buffer_to_buffer(&resolved, 0, &readback, 8, 8);
        }
        self.gpu.queue.submit([encoder.finish()]);
        let bytes = self.read_mapped(&readback)?;
        let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or_default());
        let triangles = u64::from_le_bytes(bytes[8..16].try_into().unwrap_or_default());
        Ok(Drawn {
            chunks: culled.map_or(mesh.chunks().len() as u32, |_| count),
            triangles: stats.map(|_| triangles),
        })
    }
}

/// Every triangle normal of a chunk lies within the cone around `axis` whose half angle has
/// cosine `cos` and sine `sin`. A zero axis means no useful cone: the chunk is never skipped for
/// facing away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cone {
    pub axis: V3,
    pub cos: f32,
    pub sin: f32,
}

impl Cone {
    pub(crate) const NONE: Cone = Cone {
        axis: [0.0; 3],
        cos: 0.0,
        sin: 1.0,
    };
}

/// A triangle whose doubled area is below this fraction of its longer edge from the first
/// corner, squared, is too thin to trust its facing: the GPU's rounding could see it either
/// way, so its chunk gets no cone.
const THIN: f64 = 1e-6;
/// Cones wider than this (cosine of the half angle) can hardly ever cull, and past 90 degrees
/// the test stops being safe; they are dropped.
const WIDEST: f64 = 0.05;

/// A chunk's normal cone, built one triangle at a time around a fixed axis, in f64 from the f32
/// positions. Any axis gives a safe cone, because the half angle is the widest triangle normal
/// from it; an axis along the chunk's vertex normals keeps the cone narrow. One pass with no
/// square root or division per triangle: it keeps the smallest squared cosine as a fraction. A
/// thin triangle, a triangle facing past the widest cone, a NaN or no triangle at all gives no
/// cone. f32 rounding of the result is far inside the culling pass's facing margin.
pub(crate) struct ConeBuilder {
    axis: [f64; 3],
    /// The smallest squared cosine so far between a triangle normal and the axis, as
    /// `num / den`; `den` is 0 before the first triangle.
    num: f64,
    den: f64,
    /// A triangle ruled the cone out.
    none: bool,
}

impl ConeBuilder {
    /// Around `axis`, which need not be unit length; a zero or NaN axis gives no cone.
    pub fn new(axis: V3) -> Self {
        let a = axis.map(f64::from);
        let len = dot(a, a).sqrt();
        let none = !above(len, 0.0);
        ConeBuilder {
            axis: if none { [0.0; 3] } else { a.map(|x| x / len) },
            num: 1.0,
            den: 0.0,
            none,
        }
    }

    /// Widens the cone to hold triangle `a b c` (counter-clockwise from its front).
    #[inline]
    pub fn add(&mut self, a: V3, b: V3, c: V3) {
        if self.none {
            return;
        }
        let [ax, ay, az] = a.map(f64::from);
        let (ux, uy, uz) = (
            f64::from(b[0]) - ax,
            f64::from(b[1]) - ay,
            f64::from(b[2]) - az,
        );
        let (vx, vy, vz) = (
            f64::from(c[0]) - ax,
            f64::from(c[1]) - ay,
            f64::from(c[2]) - az,
        );
        let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
        let len2 = nx * nx + ny * ny + nz * nz;
        let (eu, ev) = (ux * ux + uy * uy + uz * uz, vx * vx + vy * vy + vz * vz);
        let edge2 = if eu > ev { eu } else { ev };
        let [kx, ky, kz] = self.axis;
        let d = nx * kx + ny * ky + nz * kz;
        // |n| > THIN * edge2 and d / |n| > WIDEST, squared. Every comparison is false for a
        // NaN, so a NaN rules the cone out.
        let trusted = len2 > THIN * THIN * edge2 * edge2;
        if !(trusted && d > 0.0 && d * d > WIDEST * WIDEST * len2) {
            self.none = true;
            return;
        }
        // d² / len2 < num / den, without dividing (both denominators are positive).
        if d * d * self.den < self.num * len2 {
            (self.num, self.den) = (d * d, len2);
        }
    }

    pub fn finish(self) -> Cone {
        if self.none || self.den == 0.0 {
            return Cone::NONE;
        }
        let cos = (self.num / self.den).min(1.0).sqrt();
        Cone {
            axis: self.axis.map(|x| x as f32),
            cos: cos as f32,
            sin: (1.0 - cos * cos).sqrt() as f32,
        }
    }
}

/// What the culling pass reads for one chunk. Matches `Chunk` in `shaders/cull.wgsl` (48 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub(crate) struct ChunkCull {
    center: [f32; 3],
    radius: f32,
    axis: [f32; 3],
    cos: f32,
    sin: f32,
    first_index: u32,
    index_count: u32,
    first_triangle: u32,
}

impl ChunkCull {
    /// The record for a chunk: the sphere around its bounds, its cone and its draw. An empty
    /// chunk has no triangles and is never drawn.
    pub fn new(first_triangle: u32, triangle_count: u32, bounds: &Bounds, cone: Cone) -> Self {
        let center = scale(add(bounds.min, bounds.max), 0.5);
        let half = sub(bounds.max, center);
        ChunkCull {
            center,
            radius: dot(half, half).sqrt(),
            axis: cone.axis,
            cos: cone.cos,
            sin: cone.sin,
            first_index: first_triangle * 3,
            index_count: triangle_count * 3,
            first_triangle,
        }
    }

    /// This record drawing `triangle_count` triangles: 0 for a chunk whose faces are all
    /// hidden, which the culling pass then skips.
    #[must_use]
    pub fn draws(self, triangle_count: u32) -> Self {
        ChunkCull {
            index_count: triangle_count * 3,
            ..self
        }
    }

    /// This record for its chunk placed at `first_triangle` of the index pool.
    #[must_use]
    pub fn at(self, first_triangle: u32) -> Self {
        ChunkCull {
            first_index: first_triangle * 3,
            first_triangle,
            ..self
        }
    }
}

/// The camera as the culling pass reads it. Matches `CullCamera` in `shaders/cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct CullCamera {
    /// Left, right, bottom, top; `xyz` a unit normal pointing inside.
    planes: [[f32; 4]; 4],
    eye: [f32; 4],
    forward: [f32; 4],
    count: u32,
    /// 1 when back faces are not drawn, so chunks facing away may be skipped.
    cone: u32,
    ortho: u32,
    pad: u32,
}

/// A clip-space plane as a unit normal and offset.
fn unit_plane(p: [f64; 4]) -> [f32; 4] {
    let len = dot([p[0], p[1], p[2]], [p[0], p[1], p[2]]).sqrt();
    p.map(|c| (c / len) as f32)
}

impl CullCamera {
    /// The side planes of `view_proj` (world to clip, as the render pass uses it): inside is
    /// `-w <= x <= w` and `-w <= y <= w`. Seen by `camera`, for a mesh of `count` chunks.
    pub fn new(camera: &Camera, view_proj: &M4, count: u32, front_only: bool) -> Self {
        let row = |r: usize| view_proj.map(|col| f64::from(col[r]));
        let (x, y, w) = (row(0), row(1), row(3));
        let plus = |a: [f64; 4]| -> [f64; 4] { std::array::from_fn(|k| w[k] + a[k]) };
        let minus = |a: [f64; 4]| -> [f64; 4] { std::array::from_fn(|k| w[k] - a[k]) };
        let eye = camera.eye();
        let forward = normalize_or(sub(camera.target, eye), [0.0, 0.0, -1.0]);
        CullCamera {
            planes: [plus(x), minus(x), plus(y), minus(y)].map(unit_plane),
            eye: [eye[0], eye[1], eye[2], 1.0],
            forward: [forward[0], forward[1], forward[2], 0.0],
            count,
            cone: u32::from(front_only),
            ortho: u32::from(camera.projection == Projection::Orthographic),
            pad: 0,
        }
    }
}
