//! Drawing straight from a gs-core `MeshStore`: node-ordered pools with room for each node to
//! grow, one chunk per node, and updates (the dirty nodes fed through gs-core's node sink and
//! packed, refining strokes included) that leave the GPU holding exactly what a full upload
//! holds.

use crate::common::{fitted, lavapipe, snap};
use crate::scene;
use gs_core::live::{LiveDetail, Target};
use gs_core::{
    AttrDef, AttrVec, BrushPreset, ColumnSet, MeshStore, NO, NodeConfig, NodeId, PosRead, Stroke,
    StrokeParams, StrokeSample, Vec3, View,
};
use gs_protocol::PlainMesh;
use gs_render::{
    Bounds, GpuMesh, Image, Preset, RenderMesh, Renderer, Shading, Sides, UploadOptions, Wire,
};

/// A dented quad sphere cut into nodes of `fill` vertices (room for `1 << shift`).
pub fn sphere_store(n: u32, shift: u32, fill: u32) -> MeshStore {
    let mut mesh = scene::quad_sphere(n);
    scene::dent(&mut mesh, 0.12);
    store_of(&mesh, shift, fill)
}

pub fn store_of(mesh: &PlainMesh, shift: u32, fill: u32) -> MeshStore {
    let cfg = NodeConfig::new(shift, fill).expect("valid node size");
    MeshStore::from_plain(mesh, cfg).expect("valid mesh")
}

/// A brush-like push: every vertex within `radius` of `center` moves toward the sphere's
/// center by up to `depth`, with a smooth falloff. Only the nodes the brush touches are written.
fn stroke(store: &mut MeshStore, center: [f32; 3], radius: f32, depth: f32) {
    let c = Vec3::from_array(center);
    let mut nodes = store.query(c, radius).brushed;
    nodes.sort_unstable();
    store.write(&nodes, ColumnSet::POSITION).each(|m| {
        for p in m.positions() {
            let d2 = (*p - c).len2() / (radius * radius);
            if d2 < 1.0 {
                let w = (1.0 - d2) * (1.0 - d2) * depth;
                *p = *p - p.normalized() * w;
            }
        }
    });
}

/// Images that depend on every part of the pools, from every side: positions and normals
/// (normals from all six axis views, zebra) and the index and hidden-edge layout (wireframe).
fn pictures(r: &Renderer, mesh: &GpuMesh) -> Vec<Image> {
    let axes = Preset::ALL.map(|p| (p, Shading::Normals, Wire::Off));
    let extra = [
        (Preset::Iso, Shading::Zebra, Wire::Off),
        (Preset::Top, Shading::Clay, Wire::VertexPulling),
    ];
    axes.into_iter()
        .chain(extra)
        .map(|(preset, shading, wire)| snap(r, mesh, &fitted(mesh, preset, shading, wire), 80))
        .collect()
}

/// A shipped brush preset.
pub fn preset(name: &str) -> BrushPreset {
    let path = format!(
        "{}/../../assets/brushes/{name}.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).expect("the preset file reads");
    BrushPreset::from_toml(&text).expect("the preset parses")
}

/// Six samples of radius `radius` on the unit sphere near `towards`, 0.13 radii apart, and the
/// view along it.
pub fn path(towards: Vec3, radius: f32) -> (Vec<StrokeSample>, StrokeParams) {
    let d = towards.normalized();
    let side = d.cross(Vec3::new(0.3, 0.9, -0.2)).normalized();
    let samples: Vec<StrokeSample> = (0..6)
        .map(|k| StrokeSample {
            pos: (d + side * (0.13 * radius * k as f32)).normalized(),
            pressure: 1.0,
            tilt: [0.0; 2],
            radius,
        })
        .collect();
    (samples, StrokeParams::new(View { dir: -d, eye: None }))
}

/// A real brush stroke of six samples on the sphere near `towards`, through gs-core's stroke
/// pipeline.
fn brush(store: &mut MeshStore, brush: &BrushPreset, towards: Vec3) {
    let (samples, params) = path(towards, 0.3);
    gs_core::stroke(store, brush, params, &samples).expect("strokes");
}

/// A Clay stroke near `towards` that refines toward edges of `size` (decision 0021), sent to
/// `mesh` after every dab and at its end, as the window sends it; `check` sees each update.
fn refine(
    store: &mut MeshStore,
    towards: Vec3,
    (radius, size): (f32, f32),
    mut check: impl FnMut(&mut MeshStore, &str),
) {
    let (samples, mut params) = path(towards, radius);
    params.live_detail = Some(LiveDetail {
        target: Target::Constant(size),
        ..LiveDetail::default()
    });
    let mut stroke = Stroke::begin(store, &preset("clay"), params).expect("begins");
    for (k, sample) in samples.iter().enumerate() {
        stroke
            .add(store, std::slice::from_ref(sample))
            .expect("dabs");
        check(store, &format!("dab {k}"));
    }
    stroke.finish(store);
    check(store, "the stroke's end");
}

/// What a mesh draws of each chunk wherever it sits: its triangles and bounds.
fn drawn(mesh: &GpuMesh) -> Vec<(u32, Bounds)> {
    (mesh.chunks().iter())
        .map(|c| (c.triangle_count, c.bounds))
        .collect()
}

/// `mesh` draws what a fresh upload of `store` draws: the same chunks, and the same pools read
/// back (which are read in node order, wherever each node sits). The pools read back have no
/// hidden-edge words, so `refined_wireframes_match_a_fresh_upload` checks those by drawing.
pub fn holds_fresh(r: &Renderer, mesh: &GpuMesh, store: &MeshStore, what: &str) {
    let fresh = r.upload_store(store);
    assert_eq!(drawn(mesh), drawn(&fresh), "{what}: the chunks");
    let (held, wanted) = (r.read_pools(mesh), r.read_pools(&fresh));
    let (held, wanted) = (held.expect("reads back"), wanted.expect("reads back"));
    let same = [
        held.vertices == wanted.vertices,
        held.indices == wanted.indices,
        held.culling == wanted.culling,
        held.mask == wanted.mask,
    ];
    assert_eq!(
        same, [true; 4],
        "{what}: the GPU holds exactly what a fresh upload holds (vertices, indices, culling, mask)"
    );
}

/// The nodes `take_dirty` reported with moved positions or normals.
pub fn moved(dirty: &[(NodeId, ColumnSet)]) -> Vec<NodeId> {
    dirty
        .iter()
        .filter(|d| d.1.contains(ColumnSet::POSITION) || d.1.contains(ColumnSet::NORMAL))
        .map(|d| d.0)
        .collect()
}

/// Brings `mesh` up to date and checks it holds, byte for byte, what a fresh upload of `store`
/// holds: vertex pool, culling records and chunk table.
fn sync_and_compare(r: &Renderer, mesh: &mut GpuMesh, store: &mut MeshStore, what: &str) {
    let dirty = store.take_dirty();
    let upload = r.update_store(mesh, store, &dirty);
    assert!(!upload.full, "{what}: not a topology change");
    let fresh = r.upload_store(store);
    assert_eq!(mesh.chunks(), fresh.chunks(), "{what}: the chunk table");
    let (held, wanted) = (r.read_pools(mesh), r.read_pools(&fresh));
    assert!(
        held.expect("reads back") == wanted.expect("reads back"),
        "{what}: the GPU holds exactly what a fresh upload holds"
    );
}

/// A sphere store and its GPU copy drawn front faces only, so it keeps every normal cone, as a
/// fresh upload has.
fn sculpted() -> (MeshStore, GpuMesh) {
    let mut store = sphere_store(20, 7, 100);
    store.take_dirty();
    let mut mesh = lavapipe().upload_store(&store);
    mesh.set_sides(Sides::Front);
    (store, mesh)
}

/// An update sends the nodes the store reports moved, each one's vertex range, and the
/// culling table once.
#[test]
fn a_stroke_sends_the_nodes_it_moved() {
    let r = lavapipe();
    let (mut store, mut mesh) = sculpted();
    let start = r.read_pools(&mesh).expect("reads back");
    brush(&mut store, &preset("draw"), Vec3::new(0.2, 0.9, 0.4));
    let dirty = store.take_dirty();
    let nodes = moved(&dirty);
    let upload = r.update_store(&mut mesh, &store, &dirty);
    assert_eq!(
        upload.nodes,
        nodes.len(),
        "every moved node, and only those"
    );
    let slots: u64 = nodes.iter().map(|&n| store.node(n).pos.len() as u64).sum();
    let culled = r.gpu().capabilities().multi_draw_indirect_count;
    let table = if culled {
        48 * store.node_count() as u64
    } else {
        0
    };
    // And the runs of triangles, 12 bytes each, from the first quad whose drawn split flipped
    // to the last, in each node with one (decision 0040, section 2).
    let flips = upload.bytes.checked_sub(16 * slots + table);
    assert!(
        flips.is_some_and(|f| f % 12 == 0 && (f > 0) == (upload.flipped > 0)),
        "their vertex ranges, the culling table and the flipped quads' runs: {} bytes, {} \
         nodes flipped",
        upload.bytes,
        upload.flipped
    );
    assert!(
        upload.flipped > 0,
        "the stroke flips quads, so the check can fail"
    );
    sync_and_compare(r, &mut mesh, &mut store, "after a stroke");
    assert!(
        r.read_pools(&mesh).expect("reads back") != start,
        "the stroke changed the pools, so the comparison can fail"
    );
}

/// An update packs each node straight into its write view a block of slots at a time: nodes
/// of several blocks, the last one partial, still end up holding what a fresh upload holds.
#[test]
fn nodes_of_several_blocks_update_like_a_fresh_upload() {
    let r = lavapipe();
    let mut store = sphere_store(40, 10, 600);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    mesh.set_sides(Sides::Front);
    brush(&mut store, &preset("draw"), Vec3::new(-0.5, 0.4, 0.7));
    sync_and_compare(
        r,
        &mut mesh,
        &mut store,
        "after a stroke on nodes of 600 slots",
    );
}

#[test]
fn updated_pools_hold_what_a_fresh_upload_holds() {
    let r = lavapipe();
    let (mut store, mut mesh) = sculpted();
    let (draw, drag) = (preset("draw"), preset("move"));
    brush(&mut store, &draw, Vec3::new(-0.5, 0.4, 0.7));
    sync_and_compare(r, &mut mesh, &mut store, "after a stroke");
    // A drag goes back to the stroke start before every update.
    brush(&mut store, &drag, Vec3::new(0.7, -0.2, 0.6));
    sync_and_compare(r, &mut mesh, &mut store, "after a drag");
    // Undo and redo swap whole node columns.
    store.undo().expect("undoes");
    sync_and_compare(r, &mut mesh, &mut store, "after undo");
    store.redo().expect("redoes");
    sync_and_compare(r, &mut mesh, &mut store, "after redo");
    brush(&mut store, &draw, Vec3::new(0.1, -0.9, 0.3));
    store.undo().expect("undoes");
    sync_and_compare(
        r,
        &mut mesh,
        &mut store,
        "after a stroke undone before the update",
    );
    // A job lands: its copy was edited elsewhere, then adopted as one step.
    let mut copy = store.share();
    brush(&mut copy, &draw, Vec3::new(0.0, 0.3, -0.9));
    store.adopt(copy).expect("the job lands");
    sync_and_compare(r, &mut mesh, &mut store, "after a job commit");
}

/// Drawn both sides, a mesh never culls by facing, so the nodes it is sent get no normal cone:
/// its vertices still match a fresh upload at once, and once it is drawn front faces only, the
/// next update measures the missing cones and it holds exactly what a fresh upload holds.
#[test]
fn cones_are_measured_once_the_mesh_is_drawn_front_only() {
    let r = lavapipe();
    let mut store = sphere_store(20, 7, 100);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    brush(&mut store, &preset("draw"), Vec3::new(0.2, 0.9, 0.4));
    let dirty = store.take_dirty();
    r.update_store(&mut mesh, &store, &dirty);
    let (held, fresh) = (r.read_pools(&mesh), r.read_pools(&r.upload_store(&store)));
    let (held, fresh) = (held.expect("reads back"), fresh.expect("reads back"));
    assert!(held.vertices == fresh.vertices, "the vertices are current");
    mesh.set_sides(Sides::Front);
    let upload = r.update_store(&mut mesh, &store, &[]);
    assert!(
        upload.nodes > 0,
        "the nodes sent without a cone are measured"
    );
    let held = r.read_pools(&mesh).expect("reads back");
    assert!(
        held == fresh,
        "then the mesh holds what a fresh upload holds"
    );
}

/// After edits, culling still draws exactly what drawing everything draws, from all sides and
/// both with and without front-face culling: no stale box or cone can drop a node in view.
#[test]
fn culling_never_drops_a_visible_node_after_an_edit() {
    let r = lavapipe();
    let mut store = sphere_store(20, 7, 100);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let draw = preset("draw");
    for towards in [Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.2, 0.1)] {
        brush(&mut store, &draw, towards);
    }
    let dirty = store.take_dirty();
    r.update_store(&mut mesh, &store, &dirty);
    for sides in [Sides::Both, Sides::Front] {
        mesh.set_sides(sides);
        mesh.set_culling(true);
        let culled = pictures(r, &mesh);
        mesh.set_culling(false);
        let all = pictures(r, &mesh);
        assert!(culled == all, "{sides:?}: culling changed the picture");
    }
}

/// Every GPU buffer a mesh holds, by the layout `upload_store` documents: 16-byte vertices,
/// u32 indices, 4 bits of hidden edges per triangle, each node's triangles in whole words of 8,
/// two draw records per chunk, a culling record and a culled draw record per chunk where the
/// device culls, and a mask stream, 16 zero bytes for a mesh without a mask.
#[test]
fn a_mesh_holds_its_pools_and_records() {
    let r = lavapipe();
    let store = sphere_store(12, 6, 40);
    let mesh = r.upload_store(&store);
    let slots: u64 = (0..store.node_count() as NodeId)
        .map(|n| store.node(n).pos.len() as u64)
        .sum();
    let triangles: u64 = (mesh.chunks().iter())
        .map(|c| u64::from(c.triangle_count).next_multiple_of(8))
        .sum();
    let chunks = mesh.chunks().len() as u64;
    let culling = if r.gpu().capabilities().multi_draw_indirect_count {
        (48 + 20) * chunks
    } else {
        0
    };
    assert_eq!(
        mesh.bytes(),
        16 * slots + 12 * triangles + triangles.div_ceil(8) * 4 + (20 + 16) * chunks + culling + 16,
        "the buffers' sizes add up"
    );
}

#[test]
fn a_small_stroke_uploads_few_nodes() {
    let r = lavapipe();
    let mut store = sphere_store(64, 9, 256);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    stroke(&mut store, [0.0, 0.0, 1.0], 0.1, 0.05);
    let dirty = store.take_dirty();
    let upload = r.update_store(&mut mesh, &store, &dirty);
    assert!(
        upload.nodes >= 1 && upload.nodes * 2 < store.node_count(),
        "a brush covering about 0.3% of the surface rewrites {} of {} nodes",
        upload.nodes,
        store.node_count()
    );
}

/// A refining stroke changes topology dab by dab: each update sends only the nodes the dab
/// changed, in place or moved to the end of the pools, never everything, and the GPU then holds
/// what a fresh upload holds; so do undo and redo. (Replaces a test from before node ranges,
/// which expected every topology change to upload everything again.)
#[test]
fn refining_strokes_send_only_the_nodes_they_change() {
    let r = lavapipe();
    let mut store = sphere_store(40, 8, 200);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    mesh.set_sides(Sides::Front);
    let start = r.read_pools(&mesh).expect("reads back");
    let mut moved = 0;
    let mut send = |store: &mut MeshStore, what: &str| {
        let dirty = store.take_dirty();
        let fed = (dirty.iter())
            .filter(|d| {
                [ColumnSet::POSITION, ColumnSet::NORMAL, ColumnSet::TOPOLOGY]
                    .iter()
                    .any(|&c| d.1.contains(c))
            })
            .count();
        let upload = r.update_store(&mut mesh, store, &dirty);
        assert!(!upload.full, "{what}: not everything");
        assert_eq!(
            upload.nodes, fed,
            "{what}: every changed node, and only those"
        );
        holds_fresh(r, &mesh, store, what);
        moved += upload.moved;
    };
    for towards in [
        Vec3::new(0.2, 0.9, 0.4),
        Vec3::new(0.25, 0.85, 0.45),
        Vec3::new(-0.6, 0.2, 0.7),
    ] {
        refine(&mut store, towards, (0.3, 0.03), &mut send);
    }
    for _ in 0..3 {
        store.undo().expect("undoes");
        send(&mut store, "after undo");
    }
    store.redo().expect("redoes");
    send(&mut store, "after redo");
    assert!(moved > 0, "a node outgrew its room, so moving was tested");
    assert!(
        r.read_pools(&mesh).expect("reads back") != start,
        "the strokes changed the pools, so the comparison can fail"
    );
}

/// After each refining stroke, a mesh brought up to date draws the same wireframe as a fresh
/// upload from three sides. The wireframe reads each node's hidden-edge words, which
/// `holds_fresh` cannot compare (the pools read back carry no hidden edges).
#[test]
fn refined_wireframes_match_a_fresh_upload() {
    let r = lavapipe();
    let wires = |mesh: &GpuMesh| -> Vec<Image> {
        [Preset::Top, Preset::Front, Preset::Iso]
            .map(|p| {
                snap(
                    r,
                    mesh,
                    &fitted(mesh, p, Shading::Clay, Wire::VertexPulling),
                    160,
                )
            })
            .to_vec()
    };
    let mut store = sphere_store(24, 8, 200);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    mesh.set_sides(Sides::Front);
    for towards in [
        Vec3::new(0.0, 1.0, 0.2),
        Vec3::new(0.1, 0.9, 0.4),
        Vec3::new(-0.3, 0.8, 0.5),
    ] {
        let (samples, mut params) = path(towards, 0.35);
        params.live_detail = Some(LiveDetail {
            target: Target::Constant(0.05),
            ..LiveDetail::default()
        });
        gs_core::stroke(&mut store, &preset("clay"), params, &samples).expect("strokes");
        let dirty = store.take_dirty();
        r.update_store(&mut mesh, &store, &dirty);
        let mut fresh = r.upload_store(&store);
        fresh.set_sides(Sides::Front);
        assert!(
            wires(&mesh) == wires(&fresh),
            "the wireframe after a refining stroke"
        );
    }
}

/// Once the ranges nodes left behind would fill more than half of a pool, an update packs the
/// pools afresh: a full upload, laid out exactly as a fresh one. Undoing a stroke that grew a
/// small mesh many times over leaves most of its pools behind.
#[test]
fn a_mesh_left_mostly_unused_is_packed_again() {
    let r = lavapipe();
    let mut store = sphere_store(6, 6, 40);
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    mesh.set_sides(Sides::Front);
    let mut update = |store: &mut MeshStore, what: &str| {
        let dirty = store.take_dirty();
        let upload = r.update_store(&mut mesh, store, &dirty);
        holds_fresh(r, &mesh, store, what);
        upload
    };
    refine(
        &mut store,
        Vec3::new(0.0, 0.3, 1.0),
        (1.5, 0.08),
        |store, what| {
            let upload = update(store, what);
            assert!(
                !upload.full,
                "{what}: growing never leaves half a pool behind"
            );
        },
    );
    let nodes = store.node_count();
    store.undo().expect("undoes");
    assert!(
        store.node_count() * 4 < nodes,
        "the stroke grew the mesh: {} nodes, then {nodes}",
        store.node_count()
    );
    assert!(update(&mut store, "after undo").full, "packed afresh");
    let fresh = r.upload_store(&store);
    assert_eq!(mesh.chunks(), fresh.chunks(), "laid out as a fresh upload");
}

#[test]
fn a_mesh_from_elsewhere_is_uploaded_again() {
    let r = lavapipe();
    let store = sphere_store(6, 6, 40);
    let mut plain = r
        .upload(&store.to_plain(), UploadOptions::default())
        .expect("uploads");
    let upload = r.update_store(&mut plain, &store, &[]);
    assert!(
        upload.full,
        "a PlainMesh upload has no node layout to update"
    );
    let mut other = r.upload_store(&sphere_store(8, 6, 40));
    assert!(
        r.update_store(&mut other, &store, &[]).full,
        "another store's layout does not fit"
    );
    assert!(
        pictures(r, &other) == pictures(r, &r.upload_store(&store)),
        "either way the result draws the store"
    );
}

/// A mask write sends the mask stream of the nodes written and nothing else, and the mesh then
/// holds what a fresh upload holds (decision 0015). A group write rewrites the faces of the
/// nodes written, whose group colors are written with them (decision 0039, section 9), and no
/// vertex. (Replaces a test from before masks were drawn, which expected a mask write to send
/// nothing, and was `..._and_a_group_write_nothing` until face groups were drawn.)
#[test]
fn a_mask_write_sends_only_its_nodes_mask_and_a_group_write_only_its_nodes_faces() {
    let r = lavapipe();
    let mut store = sphere_store(8, 6, 40);
    let mask = store.add_attr(AttrDef::mask()).expect("registers");
    let group = store.add_attr(AttrDef::face_group()).expect("registers");
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let mut paint = |store: &mut MeshStore, nodes: &[NodeId], col| {
        store
            .write(nodes, ColumnSet::attr(col))
            .each(|m| match m.attr(col) {
                AttrVec::F32(v) => v.fill(0.5),
                AttrVec::U32(v) => v.fill(3),
                AttrVec::Bits(_) => {}
            });
        let dirty = store.take_dirty();
        assert!(!dirty.is_empty(), "the write is reported dirty");
        r.update_store(&mut mesh, store, &dirty)
    };
    let upload = paint(&mut store, &[0, 1], group);
    assert_eq!(
        (upload.nodes, upload.faces, upload.full),
        (0, 2, false),
        "a group write rewrites its nodes' faces and no vertex"
    );
    let first = paint(&mut store, &[0, 1], mask);
    assert!(
        !first.full && first.bytes > 0,
        "the first mask makes the stream"
    );
    let slots = store.node(2).pos.len() as u64;
    let next = paint(&mut store, &[2], mask);
    let fresh = r.upload_store(&store);
    assert!(
        (next.nodes, next.faces) == (0, 0) && next.bytes <= slots + 6,
        "then only node 2's bytes, word-aligned: {}",
        next.bytes
    );
    assert!(
        r.read_pools(&mesh).expect("reads back") == r.read_pools(&fresh).expect("reads back"),
        "the GPU holds what a fresh upload holds, group colors included"
    );
}

/// Each node is one chunk: its live faces' triangles, in node order, with the bounds of every
/// vertex they use (including other nodes' seam vertices) and of its own live vertices, each
/// starting on a whole word of 8 triangles. (Was `every_node_is_one_tight_chunk`, from before
/// node ranges.)
#[test]
fn every_node_is_one_chunk_on_a_whole_word() {
    let r = lavapipe();
    let mut store = sphere_store(12, 6, 40);
    // A split leaves dead slots and spare face slots behind, which must not be drawn.
    let edge = {
        let f = store.node(0).faces[0];
        (f[0], f[1])
    };
    store.edit_topology().split_edge(edge.0, edge.1);
    let mesh = r.upload_store(&store);
    assert_eq!(
        mesh.chunks().len(),
        store.node_count(),
        "one chunk per node"
    );
    let pos = store.pos();
    let mut first = 0;
    for (k, chunk) in mesh.chunks().iter().enumerate() {
        let faces: Vec<[u32; 4]> = (store.node(k as NodeId).faces.iter())
            .filter(|f| f[0] != NO)
            .copied()
            .collect();
        let triangles: u32 = faces.iter().map(|f| if f[3] == NO { 1 } else { 2 }).sum();
        let mut bounds = Bounds::EMPTY;
        for p in store.node(k as NodeId).pos {
            bounds.include(p.to_array());
        }
        for f in &faces {
            let corners = if f[3] == NO { &f[..3] } else { &f[..] };
            for &v in corners {
                bounds.include(pos.get(v).to_array());
            }
        }
        assert_eq!(
            (chunk.first_triangle, chunk.triangle_count, chunk.bounds),
            (first, triangles, bounds),
            "chunk {k}"
        );
        first += triangles.next_multiple_of(8);
    }
}

/// The store path draws exactly what the `PlainMesh` path draws from the store's own export and
/// normals: same triangles, same octahedral normals, only the pool layout differs.
#[test]
fn store_draws_like_its_plain_export() {
    let r = lavapipe();
    let store = sphere_store(16, 7, 100);
    let plain = store.to_plain();
    let normals: Vec<[f32; 3]> = (0..store.node_count() as NodeId)
        .flat_map(|n| store.node(n).nrm.iter().map(|m| m.to_array()))
        .collect();
    let exported = r
        .upload(
            RenderMesh::from(&plain).with_normals(&normals),
            UploadOptions::default(),
        )
        .expect("uploads");
    let direct = r.upload_store(&store);
    assert_eq!(
        direct.bounds(),
        exported.bounds(),
        "the same vertices are drawn"
    );
    for (preset, shading) in [
        (Preset::Iso, Shading::Zebra),
        (Preset::Back, Shading::Normals),
    ] {
        let view = fitted(&exported, preset, shading, Wire::Off);
        assert!(
            snap(r, &direct, &view, 128) == snap(r, &exported, &view, 128),
            "{preset:?}: the store renders like its export"
        );
    }
}
