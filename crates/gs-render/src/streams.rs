//! The streams group 1 binds beside the pools: per vertex slot, the mask (decision 0015) and
//! the point colors, and per triangle, the face group colors (decision 0039, section 9). A mesh
//! without one binds stand-in words (zeros for the mask, the renderer's white words for
//! colors), so it uploads and draws as it did before colors were drawn.

use wgpu::util::DeviceExt;

use crate::device::Gpu;
use crate::pipeline::Pipelines;
use crate::pool::{GpuMesh, grown};

/// The `Rgba8` vertex column point colors come from unless an upload names another.
pub(crate) use gs_protocol::COLOR;

/// Opaque white as an `Rgba8` word: a slot or triangle without paint.
pub(crate) const WHITE: u32 = u32::MAX;

/// What group 1 binds beside the pools.
pub(crate) struct Streams {
    /// The mask: one byte per vertex slot, four to a word; a few zero bytes while the mesh has
    /// no mask.
    pub mask: wgpu::Buffer,
    /// What `mask` holds, so updates can write whole words around a node's range; `None`
    /// while the mesh has no mask.
    pub mask_bytes: Option<Vec<u8>>,
    /// Point colors: one `Rgba8` word per vertex slot (sRGB bytes, red in the low byte) from
    /// the column named `column`; `None` while no node holds it, or when it is larger than the
    /// adapter lets a shader bind. No copy on the CPU: each slot is a whole word.
    pub colors: Option<wgpu::Buffer>,
    /// The `Rgba8` vertex column that feeds `colors`.
    pub column: String,
    /// Face group colors: one word per triangle of the index pool; `None` without groups, or
    /// when larger than the adapter lets a shader bind.
    pub groups: Option<wgpu::Buffer>,
    /// Whether the mesh was uploaded with groups, even ones too many to bind: an update that
    /// finds groups come or go uploads everything again.
    pub grouped: bool,
}

/// A storage buffer holding `bytes`, read by the geometry pass and written by updates.
fn stream(gpu: &Gpu, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::BufferUsages as B;
    gpu.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: B::STORAGE.union(B::COPY_DST).union(B::COPY_SRC),
        })
}

/// The mask stream's buffer: `bytes`, or a few zero bytes for a mesh without a mask.
fn mask_buffer(gpu: &Gpu, bytes: Option<&[u8]>) -> wgpu::Buffer {
    stream(
        gpu,
        "mask",
        bytes.filter(|b| !b.is_empty()).unwrap_or(&[0; 16]),
    )
}

/// A color stream holding `words`; `None` when there are none, or more than the adapter lets a
/// shader bind.
fn color_buffer(gpu: &Gpu, label: &str, words: Option<&[u32]>) -> Option<wgpu::Buffer> {
    let words = words.filter(|w| !w.is_empty())?;
    let fits = (size_of_val(words) as u64) <= gpu.capabilities().max_storage_binding;
    fits.then(|| stream(gpu, label, bytemuck::cast_slice(words)))
}

impl Streams {
    /// The streams of a fresh upload: the mask's bytes (dropped when larger than the adapter
    /// lets a shader bind), the point colors from `column`, and the group colors when the mesh
    /// has groups.
    pub fn new(
        gpu: &Gpu,
        mask: Option<&[u8]>,
        (colors, column): (Option<&[u32]>, &str),
        groups: Option<&[u32]>,
    ) -> Streams {
        let fits = |b: &&[u8]| b.len() as u64 <= gpu.capabilities().max_storage_binding;
        let mask_bytes = mask.filter(fits).map(<[u8]>::to_vec);
        Streams {
            mask: mask_buffer(gpu, mask_bytes.as_deref()),
            mask_bytes,
            colors: color_buffer(gpu, "point colors", colors),
            column: column.to_owned(),
            groups: color_buffer(gpu, "group colors", groups),
            grouped: groups.is_some(),
        }
    }

    /// The bytes its buffers take (the renderer's white stand-ins are not the mesh's).
    pub fn bytes(&self) -> u64 {
        let colors = [&self.colors, &self.groups].map(|b| b.as_ref().map_or(0, wgpu::Buffer::size));
        self.mask.size() + colors.iter().sum::<u64>()
    }

    /// The buffers group 1 binds at 3, 4 and 5: the mask, the point colors and the group
    /// colors, white stand-ins for the colors the mesh lacks.
    pub fn bound<'a>(&'a self, pipelines: &'a Pipelines) -> [&'a wgpu::Buffer; 3] {
        let white = &pipelines.white;
        [
            &self.mask,
            self.colors.as_ref().unwrap_or(white),
            self.groups.as_ref().unwrap_or(white),
        ]
    }

    /// Room for `slots` vertex slots and `triangles` triangles in the color streams: one too
    /// small grows by a quarter more than it needs, copied on the GPU, as the pools do; one that
    /// would pass what a shader may bind is dropped, and the part shows its material. Whether
    /// any changed.
    pub fn grow(&mut self, gpu: &Gpu, slots: u64, triangles: u64) -> bool {
        let max = gpu.capabilities().max_storage_binding;
        let mut changed = false;
        for (stream, need) in [
            (&mut self.colors, slots * 4),
            (&mut self.groups, triangles * 4),
        ] {
            let Some(buffer) = stream.as_ref().filter(|b| b.size() < need) else {
                continue;
            };
            let size = (need + need / 4).next_multiple_of(4).min(max);
            *stream = (need <= max).then(|| grown(gpu, buffer, size));
            changed = true;
        }
        changed
    }
}

impl GpuMesh {
    /// The mask stream's bytes, to fill before [`GpuMesh::send_mask`]. A mesh drawn without a
    /// mask gets one first, `fill` (one byte per vertex slot); `None` when it is larger than
    /// the adapter lets a shader bind.
    pub(crate) fn mask_bytes(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        fill: impl FnOnce() -> Vec<u8>,
    ) -> Option<&mut Vec<u8>> {
        if self.streams.mask_bytes.is_none() {
            let mut bytes = fill();
            bytes.resize(bytes.len().next_multiple_of(4), 0);
            if bytes.len() as u64 > gpu.capabilities().max_storage_binding {
                return None;
            }
            self.set_mask(gpu, pipelines, Some(bytes));
        }
        self.streams.mask_bytes.as_mut()
    }

    /// Draws the mesh without a mask again, as a fresh upload of a mesh without one does.
    pub(crate) fn drop_mask(&mut self, gpu: &Gpu, pipelines: &Pipelines) {
        if self.streams.mask_bytes.is_some() {
            self.set_mask(gpu, pipelines, None);
        }
    }

    /// A new mask stream holding `bytes` (or none), in every path's group 1.
    pub(crate) fn set_mask(&mut self, gpu: &Gpu, pipelines: &Pipelines, bytes: Option<Vec<u8>>) {
        self.streams.mask = mask_buffer(gpu, bytes.as_deref());
        self.streams.mask_bytes = bytes;
        self.rebind(gpu, pipelines);
    }

    /// Writes the mask stream's vertex slots `spans` (start and end, each a node's range) from
    /// its bytes, word-aligned. Returns the bytes written.
    pub(crate) fn send_mask(&self, gpu: &Gpu, spans: &[(u32, u32)]) -> u64 {
        let Some(bytes) = &self.streams.mask_bytes else {
            return 0;
        };
        let mut sent = 0;
        for &(start, end) in spans {
            let start = (start & !3) as usize;
            let end = (end as usize).next_multiple_of(4).min(bytes.len());
            (gpu.queue).write_buffer(&self.streams.mask, start as u64, &bytes[start..end]);
            sent += (end - start) as u64;
        }
        sent
    }

    /// A new point color stream holding `words` (or none: the part shows its material), in
    /// every path's group 1. Returns the bytes sent.
    pub(crate) fn set_colors(
        &mut self,
        gpu: &Gpu,
        pipelines: &Pipelines,
        words: Option<&[u32]>,
    ) -> u64 {
        self.streams.colors = color_buffer(gpu, "point colors", words);
        self.rebind(gpu, pipelines);
        self.streams.colors.as_ref().map_or(0, wgpu::Buffer::size)
    }
}

/// Writes `words` into a color stream (if the mesh has it) from word `first`: a vertex slot in
/// [`Streams::colors`], a triangle in [`Streams::groups`]. Returns the bytes written.
pub(crate) fn send_words(
    gpu: &Gpu,
    stream: Option<&wgpu::Buffer>,
    first: u32,
    words: &[u32],
) -> u64 {
    let bytes: &[u8] = bytemuck::cast_slice(words);
    match stream {
        Some(stream) if !bytes.is_empty() => {
            gpu.queue.write_buffer(stream, u64::from(first) * 4, bytes);
            bytes.len() as u64
        }
        _ => 0,
    }
}
