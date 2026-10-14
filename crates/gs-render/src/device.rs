//! A headless wgpu device on a chosen adapter. No window and no surface.

use std::sync::OnceLock;

use crate::error::RenderError;

/// Which adapter to render on. Tests use [`AdapterKind::Lavapipe`] (the default), Mesa's
/// software Vulkan, because it renders byte-identically run to run and across thread counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AdapterKind {
    /// Mesa's software rasterizer (llvmpipe). Tier C in ARCHITECTURE section 11.
    #[default]
    Lavapipe,
    /// An NVIDIA GPU with NVIDIA's own driver. Tier A.
    Nvidia,
    /// An AMD GPU with Mesa's RADV driver. Tier B (the Radeon iGPU).
    Radv,
}

const NVIDIA_VENDOR: u32 = 0x10de;

impl AdapterKind {
    /// Every kind, in the order they are documented.
    pub const ALL: [AdapterKind; 3] = [
        AdapterKind::Lavapipe,
        AdapterKind::Nvidia,
        AdapterKind::Radv,
    ];

    /// `lavapipe` (or `llvmpipe`), `nvidia` or `radv`.
    pub fn from_name(name: &str) -> Option<AdapterKind> {
        match name {
            "lavapipe" | "llvmpipe" => Some(AdapterKind::Lavapipe),
            "nvidia" => Some(AdapterKind::Nvidia),
            "radv" => Some(AdapterKind::Radv),
            _ => None,
        }
    }

    /// The name [`AdapterKind::from_name`] accepts.
    pub fn name(self) -> &'static str {
        match self {
            AdapterKind::Lavapipe => "lavapipe",
            AdapterKind::Nvidia => "nvidia",
            AdapterKind::Radv => "radv",
        }
    }

    pub(crate) fn matches(self, info: &wgpu::AdapterInfo) -> bool {
        match self {
            AdapterKind::Lavapipe => info.name.contains("llvmpipe"),
            AdapterKind::Nvidia => info.vendor == NVIDIA_VENDOR,
            AdapterKind::Radv => info.driver.eq_ignore_ascii_case("radv"),
        }
    }

    /// Whether the adapter [`Gpu::open`] opens for this kind is an integrated GPU, read once a
    /// process without opening a device; `None` when none is installed.
    pub fn integrated(self) -> Option<bool> {
        static SEEN: [OnceLock<Option<bool>>; AdapterKind::ALL.len()] =
            [const { OnceLock::new() }; AdapterKind::ALL.len()];
        *SEEN[self as usize].get_or_init(|| {
            let mut found = adapters().1.into_iter().map(|a| a.get_info());
            found
                .find(|info| self.matches(info))
                .map(|info| integrated(&info))
        })
    }

    /// The kinds installed on this machine.
    pub fn available() -> Vec<AdapterKind> {
        let (_, adapters) = adapters();
        AdapterKind::ALL
            .into_iter()
            .filter(|k| adapters.iter().any(|a| k.matches(&a.get_info())))
            .collect()
    }
}

/// What the opened device can do, probed from the features it was granted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Native wireframe from `@builtin(barycentric)` and `@builtin(primitive_index)`.
    /// Missing on llvmpipe, which uses the vertex-pulling wireframe instead.
    pub barycentric_wireframe: bool,
    /// `multi_draw_indexed_indirect_count`, for GPU culling. Without it every chunk is drawn.
    pub multi_draw_indirect_count: bool,
    /// Pipeline statistics queries, for counting the triangles a frame draws.
    pub pipeline_statistics: bool,
    /// Largest buffer a shader may bind as storage. The vertex-pulling wireframe binds both
    /// pools as storage, so it only works for pools up to this size (128 MiB on llvmpipe).
    pub max_storage_binding: u64,
}

/// gs-render draws every chunk with its first triangle passed as `first_instance`.
const REQUIRED: wgpu::Features = wgpu::Features::INDIRECT_FIRST_INSTANCE;
const BARYCENTRIC: wgpu::Features =
    wgpu::Features::SHADER_BARYCENTRICS.union(wgpu::Features::PRIMITIVE_INDEX);
const OPTIONAL: wgpu::Features = BARYCENTRIC
    .union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
    .union(wgpu::Features::PIPELINE_STATISTICS_QUERY)
    .union(wgpu::Features::TIMESTAMP_QUERY);
/// Everything gs-render asks for; `open` has already checked the required part is there.
const WANTED: wgpu::Features = REQUIRED.union(OPTIONAL);

/// An open device and its queue.
pub struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    caps: Capabilities,
}

/// An integrated GPU is tier B (ARCHITECTURE 11); lavapipe is a CPU device, not one.
fn integrated(info: &wgpu::AdapterInfo) -> bool {
    info.device_type == wgpu::DeviceType::IntegratedGpu
}

/// A wgpu instance on the primary backends (Vulkan on Linux), and its adapters.
pub(crate) fn adapters() -> (wgpu::Instance, Vec<wgpu::Adapter>) {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::PRIMARY;
    // Not from the build type: debug builds would ask for Vulkan validation layers. Indirect-call
    // validation stays on because chunk draws are indirect. WGPU_VALIDATION=1 still works.
    desc.flags = wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL.with_env();
    let instance = wgpu::Instance::new(desc);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::PRIMARY));
    (instance, adapters)
}

impl Gpu {
    /// Opens a device on the first adapter of `kind`, with every optional feature it has.
    pub fn open(kind: AdapterKind) -> Result<Gpu, RenderError> {
        let adapter = adapters()
            .1
            .into_iter()
            .find(|a| kind.matches(&a.get_info()))
            .ok_or_else(|| RenderError::NoAdapter(kind.name().to_owned()))?;
        Gpu::on(&adapter)
    }

    /// Opens a device on `adapter`, with every optional feature it has.
    pub(crate) fn on(adapter: &wgpu::Adapter) -> Result<Gpu, RenderError> {
        let missing = REQUIRED - adapter.features();
        if !missing.is_empty() {
            return Err(RenderError::Device(format!(
                "{} lacks {missing:?}",
                adapter.get_info().name
            )));
        }
        let desc = wgpu::DeviceDescriptor {
            required_features: adapter.features() & WANTED,
            required_limits: adapter.limits(),
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc))
            .map_err(|e| RenderError::Device(e.to_string()))?;
        let granted = device.features();
        let caps = Capabilities {
            barycentric_wireframe: granted.contains(BARYCENTRIC),
            multi_draw_indirect_count: granted.contains(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT),
            pipeline_statistics: granted.contains(wgpu::Features::PIPELINE_STATISTICS_QUERY),
            max_storage_binding: device.limits().max_storage_buffer_binding_size,
        };
        Ok(Gpu {
            device,
            queue,
            info: adapter.get_info(),
            caps,
        })
    }

    /// What this device can do.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Whether this is an integrated GPU (tier B in ARCHITECTURE 11), which gets the cheaper
    /// ambient occlusion (decision 0019). Lavapipe is not.
    pub fn integrated(&self) -> bool {
        integrated(&self.info)
    }

    /// The adapter's name, for example `llvmpipe (LLVM 22.1.8, 256 bits)`.
    pub fn adapter_name(&self) -> &str {
        &self.info.name
    }

    /// The driver and its version, for example `llvmpipe Mesa 26.2.2 (LLVM 22.1.8)`.
    pub fn driver(&self) -> String {
        format!("{} {}", self.info.driver, self.info.driver_info)
    }

    /// Identifies the renderer for golden images (ARCHITECTURE section 10.3): adapter, driver
    /// and wgpu version. A golden made under the same key must match byte for byte.
    pub fn renderer_key(&self) -> String {
        format!(
            "{} | {} | wgpu {}",
            self.info.name,
            self.driver(),
            WGPU_VERSION
        )
    }
}

/// The wgpu version gs-render is built on. Must match the exact pin in Cargo.toml.
pub const WGPU_VERSION: &str = "30.0.1";
