//! Hidden faces and the mask on screen (decision 0015). Hiding or showing faces rewrites only
//! the touched nodes' index ranges and leaves the GPU holding what a fresh upload holds; a
//! store with hidden faces draws like its plain export, which carries the hide column; showing
//! everything again gives back the first upload's bytes; a node with every face hidden draws
//! nothing. The mask darkens what it covers, more for a stronger mask, the same from the store
//! and from its export.

use gs_core::mask::Scope;
use gs_core::select::{FaceRule, Selector};
use gs_core::{AttrDef, AttrVec, ColumnSet, MeshStore, NodeId, VisibilityEdit};
use gs_render::{GpuMesh, Image, RenderMesh, Renderer, Shading, UploadOptions};

use crate::common::{covered, front_ortho, lavapipe, luma, snap};
use crate::store::sphere_store;

/// The store's export drawn from plain mesh data (as looks are), with the store's normals.
fn exported(r: &Renderer, store: &MeshStore) -> GpuMesh {
    let plain = store.to_plain();
    let normals: Vec<[f32; 3]> = (0..store.node_count() as NodeId)
        .flat_map(|n| store.node(n).nrm.iter().map(|m| m.to_array()))
        .collect();
    r.upload(
        RenderMesh::from(&plain).with_normals(&normals),
        UploadOptions::default(),
    )
    .expect("uploads")
}

/// Masks the points right of `x`, then hides the faces with any corner masked, as one edit
/// like `visibility.hide`.
fn hide_right_of(store: &mut MeshStore, x: f32) -> Vec<NodeId> {
    let mask = store.add_attr(AttrDef::mask()).expect("registers");
    let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    let pos: Vec<Vec<f32>> = (all.iter())
        .map(|&n| store.node(n).pos.iter().map(|p| p.x).collect())
        .collect();
    store.write(&all, ColumnSet::attr(mask)).each(|m| {
        let xs = &pos[m.id() as usize];
        if let AttrVec::F32(v) = m.attr(mask) {
            for (value, &px) in v.iter_mut().zip(xs) {
                *value = if px > x { 1.0 } else { 0.0 };
            }
        }
    });
    let set = store.select(&Selector::Masked {
        threshold: 1.0,
        rule: FaceRule::Any,
    });
    store.edit_visibility(VisibilityEdit::Hide(&set))
}

/// What the GPU holds for a mesh and what a fresh upload of `store` holds, byte for byte.
fn holds_fresh(r: &Renderer, mesh: &gs_render::GpuMesh, store: &MeshStore, what: &str) {
    let fresh = r.upload_store(store);
    assert_eq!(mesh.chunks(), fresh.chunks(), "{what}: the chunk table");
    let (held, wanted) = (r.read_pools(mesh), r.read_pools(&fresh));
    assert!(
        held.expect("reads back") == wanted.expect("reads back"),
        "{what}: the GPU holds what a fresh upload holds"
    );
}

#[test]
fn hiding_faces_rewrites_only_their_nodes_and_draws_like_the_export() {
    let r = lavapipe();
    let mut store = sphere_store(16, 6, 40);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let first = r.read_pools(&mesh).expect("reads back");
    let view = front_ortho(Shading::Zebra);
    let whole = snap(r, &mesh, &view, 128);

    let written = hide_right_of(&mut store, 0.5);
    let upload = {
        let dirty = store.take_dirty();
        r.update_store(&mut mesh, &store, &dirty)
    };
    assert_eq!(
        (upload.nodes, upload.faces, upload.full),
        (0, written.len(), false),
        "only the index ranges of the nodes whose faces were hidden"
    );
    assert!(
        !written.is_empty() && written.len() < store.node_count(),
        "{} of {} nodes",
        written.len(),
        store.node_count()
    );
    holds_fresh(r, &mesh, &store, "after hiding");

    let shown = snap(r, &mesh, &view, 128);
    assert!(
        shown == snap(r, &exported(r, &store), &view, 128),
        "the store draws like its export, which leaves the same faces out"
    );
    assert!(
        covered(&shown) < covered(&whole) * 9 / 10,
        "hidden faces are not drawn ({} of {} pixels)",
        covered(&shown),
        covered(&whole)
    );

    store.edit_visibility(VisibilityEdit::ShowAll);
    store.mask_set(0.0, Scope::All);
    {
        let dirty = store.take_dirty();
        r.update_store(&mut mesh, &store, &dirty)
    };
    holds_fresh(r, &mesh, &store, "after showing everything");
    let now = r.read_pools(&mesh).expect("reads back");
    let same = [
        now.vertices == first.vertices,
        now.indices == first.indices,
        now.culling == first.culling,
        now.mask == first.mask,
    ];
    assert_eq!(
        same, [true; 4],
        "showing everything and clearing the mask gives back the first upload"
    );

    hide_right_of(&mut store, -2.0);
    {
        let dirty = store.take_dirty();
        r.update_store(&mut mesh, &store, &dirty)
    };
    holds_fresh(r, &mesh, &store, "after hiding everything");
    assert_eq!(covered(&snap(r, &mesh, &view, 128)), 0, "nothing is drawn");
}

/// Masks the store's points by their height: 0 below y = -0.3, 0.5 up to y = 0.3, 1 above.
fn mask_by_height(store: &mut MeshStore) {
    let mask = store.add_attr(AttrDef::mask()).expect("registers");
    let all: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    let ys: Vec<Vec<f32>> = (all.iter())
        .map(|&n| store.node(n).pos.iter().map(|p| p.y).collect())
        .collect();
    store.write(&all, ColumnSet::attr(mask)).each(|m| {
        let ys = &ys[m.id() as usize];
        if let AttrVec::F32(v) = m.attr(mask) {
            for (value, &y) in v.iter_mut().zip(ys) {
                *value = if y < -0.3 {
                    0.0
                } else if y < 0.3 {
                    0.5
                } else {
                    1.0
                };
            }
        }
    });
}

/// The mean brightness of the pixels of `image` in rows `rows` near the middle columns.
fn band(image: &Image, rows: std::ops::Range<u32>) -> f64 {
    let px: Vec<u32> = (rows.flat_map(|y| (56..72).map(move |x| (x, y))))
        .map(|(x, y)| luma(image.pixel(x, y)))
        .collect();
    px.iter().map(|&l| f64::from(l)).sum::<f64>() / px.len() as f64
}

#[test]
fn the_mask_darkens_in_proportion_and_the_export_agrees() {
    let r = lavapipe();
    let mut store = sphere_store(24, 7, 100);
    let view = front_ortho(Shading::Clay);
    let plain = snap(r, &r.upload_store(&store), &view, 128);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    mask_by_height(&mut store);
    let dirty = store.take_dirty();
    r.update_store(&mut mesh, &store, &dirty);
    holds_fresh(r, &mesh, &store, "after masking");
    let masked = snap(r, &mesh, &view, 128);
    assert!(
        masked == snap(r, &exported(r, &store), &view, 128),
        "the store and its export darken alike"
    );
    // Rows well inside each band (y grows downward on screen; the sphere spans rows 6 to 122).
    let (low, mid, high) = (90..110, 56..72, 18..38);
    let ratio = |rows: std::ops::Range<u32>| band(&masked, rows.clone()) / band(&plain, rows);
    let (r0, r5, r1) = (ratio(low), ratio(mid), ratio(high));
    assert!(
        (r0 - 1.0).abs() < 1e-9 && r5 < 0.95 && r1 < r5,
        "unmasked unchanged, half masked darker, fully masked darker still: {r0} {r5} {r1}"
    );
}
