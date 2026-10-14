//! Adapter choice and feature probing.

use crate::common::{covered, fitted, hardware, lavapipe, snap, upload};
use crate::scene;
use gs_protocol::NO_VERTEX;
use gs_render::{
    AdapterKind, Gpu, Preset, RenderError, RenderMesh, RenderSize, Shading, UploadOptions, Wire,
};

#[test]
fn adapter_names_round_trip() {
    for kind in AdapterKind::ALL {
        assert_eq!(
            AdapterKind::from_name(kind.name()),
            Some(kind),
            "{kind:?} parses from its own name"
        );
    }
    assert_eq!(
        AdapterKind::from_name("llvmpipe"),
        Some(AdapterKind::Lavapipe),
        "llvmpipe is lavapipe's driver name"
    );
    assert_eq!(
        AdapterKind::from_name("intel"),
        None,
        "unknown names are refused"
    );
    assert_eq!(
        AdapterKind::default(),
        AdapterKind::Lavapipe,
        "tests default to lavapipe"
    );
}

/// `available` lists exactly the kinds that open, whatever hardware the machine has; with no
/// kind named, the first adapter wgpu offers opens (the default off Linux).
#[test]
fn available_lists_what_opens() {
    let available = AdapterKind::available();
    assert_eq!(
        Gpu::open(None).is_ok(),
        !available.is_empty(),
        "with no kind named, the first adapter opens"
    );
    for kind in AdapterKind::ALL {
        let opens = Gpu::open(kind);
        assert_eq!(
            opens.is_ok(),
            available.contains(&kind),
            "{kind:?} is listed exactly when it opens"
        );
        if let Err(e) = opens {
            assert_eq!(
                e,
                RenderError::NoAdapter(kind.name().to_owned()),
                "a missing adapter says so"
            );
        }
    }
}

#[test]
fn lavapipe_is_installed_and_opens() {
    assert!(
        AdapterKind::available().contains(&AdapterKind::Lavapipe),
        "lavapipe is listed as available"
    );
    assert!(
        lavapipe().gpu().adapter_name().contains("llvmpipe"),
        "the lavapipe renderer runs on llvmpipe"
    );
}

/// The spike's feature table (docs/spikes/golden.md): llvmpipe lacks barycentrics, every
/// adapter has multi-draw-indirect-count, and llvmpipe binds at most 128 MiB as storage.
#[test]
fn probed_features_match_the_spike() {
    let caps = lavapipe().gpu().capabilities();
    assert!(
        !caps.barycentric_wireframe,
        "llvmpipe has no shader barycentrics"
    );
    assert!(
        caps.multi_draw_indirect_count,
        "llvmpipe has multi-draw-indirect-count"
    );
    assert_eq!(
        caps.max_storage_binding,
        128 << 20,
        "llvmpipe binds at most 128 MiB as storage"
    );
    for (kind, r) in hardware() {
        let name = r.gpu().adapter_name();
        let expected = if *kind == AdapterKind::Nvidia {
            "NVIDIA"
        } else {
            "RADV"
        };
        assert!(name.contains(expected), "{kind:?} opened {name}");
        let caps = r.gpu().capabilities();
        assert!(
            caps.barycentric_wireframe && caps.multi_draw_indirect_count,
            "{kind:?} has barycentrics and MDIC"
        );
        assert!(
            caps.max_storage_binding > 128 << 20,
            "{kind:?} binds more than llvmpipe"
        );
    }
}

#[test]
fn renderer_key_names_adapter_driver_and_wgpu() {
    let key = lavapipe().gpu().renderer_key();
    assert!(
        key.starts_with("llvmpipe (LLVM"),
        "key starts with the adapter name: {key}"
    );
    assert!(
        key.contains("| llvmpipe Mesa"),
        "key has the driver and its version: {key}"
    );
    assert!(
        key.ends_with("| wgpu 30.0.1"),
        "key ends with the pinned wgpu version: {key}"
    );
    let gpu = lavapipe().gpu();
    assert_eq!(
        key,
        format!(
            "{} | {} | wgpu {}",
            gpu.adapter_name(),
            gpu.driver(),
            gs_render::WGPU_VERSION
        ),
        "the key is made of the three parts golden manifests store"
    );
    assert!(
        gpu.driver().starts_with("llvmpipe Mesa "),
        "the driver part names the driver and its version: {}",
        gpu.driver()
    );
    let manifest = include_str!("../../Cargo.toml");
    assert!(
        manifest.contains(&format!("wgpu = \"={}\"", gs_render::WGPU_VERSION)),
        "WGPU_VERSION is the exact pin in Cargo.toml"
    );
}

#[test]
fn barycentric_wireframe_is_refused_without_the_feature() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(2));
    let view = |wire| fitted(&mesh, Preset::Front, Shading::Clay, wire);
    let refused = r.snapshot(&mesh, &view(Wire::Barycentric), RenderSize::square(64));
    assert!(
        matches!(refused, Err(RenderError::Unsupported(_))),
        "barycentric wireframe needs the feature: {refused:?}"
    );
    let auto = snap(r, &mesh, &view(Wire::Auto), 64);
    assert_eq!(
        auto,
        snap(r, &mesh, &view(Wire::VertexPulling), 64),
        "Auto falls back to vertex pulling on llvmpipe"
    );
    assert_ne!(
        auto,
        snap(r, &mesh, &view(Wire::Off), 64),
        "the fallback draws lines"
    );
}

/// llvmpipe binds at most 128 MiB as storage, so its only wireframe (vertex pulling) works up
/// to exactly a 128 MiB vertex pool and is refused above it, while plain shading still draws.
#[test]
fn vertex_pulling_stops_at_the_storage_limit() {
    let r = lavapipe();
    let limit = (128 << 20) / 16; // 16-byte vertices in 128 MiB
    for (vertices, pulls) in [(limit, true), (limit + 1, false)] {
        let mut positions = vec![[0.0f32; 3]; vertices];
        (positions[1], positions[2]) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let normals = vec![[0.0f32, 0.0, 1.0]; vertices];
        let plain = scene::plain(positions, vec![[0, 1, 2, NO_VERTEX]]);
        let mesh = r
            .upload(
                RenderMesh::from(&plain).with_normals(&normals),
                UploadOptions::default(),
            )
            .expect("uploads");
        let view = |wire| fitted(&mesh, Preset::Front, Shading::Clay, wire);
        for wire in [Wire::Auto, Wire::VertexPulling] {
            let result = r.snapshot(&mesh, &view(wire), RenderSize::square(32));
            assert_eq!(
                result.is_ok(),
                pulls,
                "{vertices} vertices, {wire:?}: {result:?}"
            );
            assert!(
                pulls || matches!(result, Err(RenderError::Unsupported(_))),
                "refused as unsupported"
            );
        }
        assert!(
            covered(&snap(r, &mesh, &view(Wire::Off), 32)) > 0,
            "plain shading draws any size"
        );
    }
}
