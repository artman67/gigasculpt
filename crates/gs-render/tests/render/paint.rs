//! Paint on screen (decision 0039, section 9): point colors multiply the albedo where a part
//! has them, face group colors show while the view asks for them, and a part without either,
//! or with its colors hidden or all white, draws byte for byte as before. A paint-only change
//! sends only the color stream, and so does its undo.

use crate::common::{BACKGROUND, front_ortho, lavapipe, shipped, snap, upload};
use crate::scene;
use crate::store::sphere_store;
use gs_core::groups::group_color;
use gs_core::transform::Similarity;
use gs_core::{AttrDef, AttrId, AttrVec, ColumnSet, MeshStore, NodeId, Vec3};
use gs_protocol::{AttrDomain, AttrValues, Attribute, NO_VERTEX, PlainMesh};
use gs_render::matcap::{Bake, Baked, MatCapImage, Options, Recipe, bake_under};
use gs_render::shading::Environment;
use gs_render::{
    GpuMatCap, GpuMesh, Image, Occlusion, RenderMesh, RenderSize, Shading, UploadOptions, View,
    Wire,
};

const SIZE: u32 = 96;
const RED: [u8; 3] = [200, 45, 45];

/// An `Rgba8` word: sRGB bytes, red in the low byte, opaque.
fn word([r, g, b]: [u8; 3]) -> u32 {
    u32::from_le_bytes([r, g, b, 255])
}

/// `mesh` with a `color` column: `color` of each point's position.
fn painted(mut mesh: PlainMesh, color: impl Fn([f32; 3]) -> [u8; 3]) -> PlainMesh {
    let words = mesh.positions.iter().map(|&p| word(color(p))).collect();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(words),
    };
    mesh.attributes.insert("color".into(), column);
    mesh
}

/// `mesh` with a `group` column: `group` of each face's center.
fn grouped(mut mesh: PlainMesh, group: impl Fn([f32; 3]) -> u32) -> PlainMesh {
    let ids = (mesh.faces.iter())
        .map(|f| {
            let corners: Vec<u32> = f.iter().copied().filter(|&v| v != NO_VERTEX).collect();
            let center = std::array::from_fn(|k| {
                let sum: f32 = corners.iter().map(|&v| mesh.positions[v as usize][k]).sum();
                sum / corners.len() as f32
            });
            group(center)
        })
        .collect();
    let column = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(ids),
    };
    mesh.attributes.insert("group".into(), column);
    mesh
}

/// Red where x is above 0.2, white elsewhere.
fn red_right(p: [f32; 3]) -> [u8; 3] {
    if p[0] > 0.2 { RED } else { [255; 3] }
}

/// Group 5 where x is above 0.3, group 9 below -0.3, none between.
fn three_bands(p: [f32; 3]) -> u32 {
    match p[0] {
        x if x > 0.3 => 5,
        x if x < -0.3 => 9,
        _ => 0,
    }
}

/// The shipped gray look baked small.
fn gray_look() -> GpuMatCap {
    let env = Environment::shipped("studio").expect("the studio builds");
    let recipe = Recipe {
        bake: Bake {
            size: 64,
            ..Bake::default()
        },
        ..shipped("gray")
    };
    lavapipe().upload_bake(&bake_under(&recipe, &env, Options::default()))
}

/// A white 8-bit MatCap: with occlusion off, each pixel shows its paint as it is stored.
fn white_image() -> GpuMatCap {
    let pixels = vec![[u16::MAX; 4]; 16];
    lavapipe().upload_matcap(&MatCapImage { size: 4, pixels })
}

/// Front orthographic, -1.1..1.1 both ways, in `shading`, with `wire` and face groups shown
/// or not.
fn view(shading: Shading<'_>, wire: Wire, face_groups: bool) -> View<'_> {
    View {
        wire,
        face_groups,
        ..front_ortho(shading)
    }
}

/// `view` with nothing on the paint's way to the screen: no occlusion, no dither.
fn exact(mut view: View<'_>) -> View<'_> {
    view.occlusion = Occlusion::OFF;
    view.display.dither = false;
    view
}

/// The covered pixels well inside the sphere's outline whose world x (at the image's scale)
/// is in `xs`.
fn inside(image: &Image, xs: std::ops::Range<f32>) -> Vec<[u8; 4]> {
    let scale = 2.2 / SIZE as f32;
    let mut out = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (wx, wy) = (
                (x as f32 + 0.5) * scale - 1.1,
                (y as f32 + 0.5) * scale - 1.1,
            );
            let p = image.pixel(x, y);
            if xs.contains(&wx) && wx * wx + wy * wy < 0.8 && p != BACKGROUND {
                out.push(p);
            }
        }
    }
    out
}

/// The largest channel difference between `pixels` and `want`.
fn off_by(pixels: &[[u8; 4]], want: [u8; 3]) -> u8 {
    let each = pixels
        .iter()
        .flat_map(|p| (0..3).map(|k| p[k].abs_diff(want[k])));
    each.max().unwrap_or(u8::MAX)
}

/// A part whose paint changes nothing draws exactly as the part without it, in every shading
/// and on both wire paths: an all-white color column, colors hidden (Colorize off), a group
/// column while the view shows no groups, and the view's groups on a part with none. Zebra and
/// normals show no paint at all.
#[test]
fn paint_that_changes_nothing_draws_byte_for_byte_as_before() {
    let r = lavapipe();
    let base = scene::icosphere(4);
    let plain = upload(r, &base);
    let white = upload(r, &painted(base.clone(), |_| [255; 3]));
    let red = upload(r, &painted(base.clone(), red_right));
    let hidden = upload(r, &painted(base.clone(), red_right)).painted(false);
    let grouped = upload(r, &grouped(base, three_bands));
    let (gray, clay) = (gray_look(), shipped("light-brown-clay").material);
    let shadings = [
        ("clay", Shading::Clay),
        ("matcap", Shading::MatCap(&gray)),
        ("lit", Shading::Lit(&clay)),
        ("zebra", Shading::Zebra),
        ("normals", Shading::Normals),
    ];
    for (name, shading) in shadings {
        for wire in [Wire::Off, Wire::VertexPulling] {
            let want = snap(r, &plain, &view(shading, wire, false), SIZE);
            let mut same = vec![
                ("white", &white, false),
                ("hidden", &hidden, false),
                ("grouped, groups off", &grouped, false),
                ("no groups, groups on", &plain, true),
            ];
            if matches!(shading, Shading::Zebra | Shading::Normals) {
                same.push(("painted", &red, false));
            }
            for (what, mesh, groups) in same {
                let got = snap(r, mesh, &view(shading, wire, groups), SIZE);
                assert!(got == want, "{name}, {wire:?}: {what} draws as before");
            }
        }
    }
}

/// A painted patch shows its own color: through a white image MatCap with occlusion off, the
/// pixels it covers show its sRGB bytes within a level, so the paint survives its trip through
/// the vertex stage, the rasterizer and the paint target. The unpainted side is byte for byte
/// the unpainted picture. In the baked MatCap, Clay and Lit the patch keeps more red than green
/// and blue, where the unpainted side is near neutral.
#[test]
fn a_painted_patch_shows_its_color_and_leaves_the_rest_alone() {
    let r = lavapipe();
    let base = scene::icosphere(4);
    let (plain, red) = (upload(r, &base), upload(r, &painted(base, red_right)));
    let image = white_image();
    let shown = |mesh: &GpuMesh| {
        let v = exact(view(Shading::MatCap(&image), Wire::Off, false));
        snap(r, mesh, &v, SIZE)
    };
    let (before, after) = (shown(&plain), shown(&red));
    let patch = inside(&after, 0.35..1.0);
    assert!(
        patch.len() > 100,
        "the patch covers pixels: {}",
        patch.len()
    );
    assert!(
        off_by(&patch, RED) <= 1,
        "the patch shows its bytes: off by {}",
        off_by(&patch, RED)
    );
    assert!(
        inside(&after, -1.0..0.0) == inside(&before, -1.0..0.0),
        "the unpainted side is unchanged"
    );
    let (gray, clay) = (gray_look(), shipped("light-brown-clay").material);
    for (name, shading) in [
        ("clay", Shading::Clay),
        ("matcap", Shading::MatCap(&gray)),
        ("lit", Shading::Lit(&clay)),
    ] {
        let tinted = snap(r, &red, &view(shading, Wire::Off, false), SIZE);
        let mean = |pixels: &[[u8; 4]], k: usize| {
            pixels.iter().map(|p| f32::from(p[k])).sum::<f32>() / pixels.len() as f32
        };
        let patch = inside(&tinted, 0.35..0.8);
        let (rr, g, b) = (mean(&patch, 0), mean(&patch, 1), mean(&patch, 2));
        assert!(
            rr > 1.5 * g && rr > 1.5 * b,
            "{name}: the patch is red ({rr:.0}, {g:.0}, {b:.0})"
        );
    }
}

/// A baked look with D empty and S a flat half gray, of `metalness`.
fn reflections_only(metalness: f32) -> GpuMatCap {
    let half = |v: f32| half::f16::from_f32(v).to_bits();
    let (zero, s) = (
        [0, 0, 0, half(1.0)],
        [half(0.5), half(0.5), half(0.5), half(1.0)],
    );
    lavapipe().upload_bake(&Baked {
        size: 4,
        d: vec![zero; 16],
        s: vec![s; 16],
        albedo: [1.0f32.to_bits(); 3],
        roughness: 1.0f32.to_bits(),
        metalness: metalness.to_bits(),
    })
}

/// Paint tints a metal's reflections and never a dielectric's (catalog item
/// `material.matcap-shading`, research 11, R7 in research 27): on a look with D empty, a red
/// patch leaves a dielectric byte for byte as before and turns a metal red, its unpainted side
/// unchanged.
#[test]
fn paint_tints_a_metals_reflections_and_never_a_dielectrics() {
    let r = lavapipe();
    let base = scene::icosphere(4);
    let (plain, red) = (upload(r, &base), upload(r, &painted(base, red_right)));
    let (dielectric, metal) = (reflections_only(0.0), reflections_only(1.0));
    let shown = |mesh: &GpuMesh, look: &GpuMatCap| {
        let v = exact(view(Shading::MatCap(look), Wire::Off, false));
        snap(r, mesh, &v, SIZE)
    };
    assert!(
        shown(&red, &dielectric) == shown(&plain, &dielectric),
        "paint leaves a dielectric's reflections alone"
    );
    let (before, after) = (shown(&plain, &metal), shown(&red, &metal));
    let patch = inside(&after, 0.35..0.8);
    let mean = |k: usize| patch.iter().map(|p| f32::from(p[k])).sum::<f32>() / patch.len() as f32;
    let (rr, g, b) = (mean(0), mean(1), mean(2));
    assert!(
        patch.len() > 100 && rr > 1.5 * g && rr > 1.5 * b,
        "the metal's patch is red ({rr:.0}, {g:.0}, {b:.0})"
    );
    assert!(
        inside(&after, -1.0..0.0) == inside(&before, -1.0..0.0),
        "the metal's unpainted side is unchanged"
    );
}

/// Face group colors: with the view's groups on, each group's pixels show its color by the
/// formula (`gs_core::groups::group_color`, seed 0) within a level, and pixels of no group show
/// white, with no wire drawn over them. Without the view's groups they show nothing.
#[test]
fn each_face_group_shows_its_own_color() {
    let r = lavapipe();
    let mesh = upload(r, &grouped(scene::icosphere(4), three_bands));
    let image = white_image();
    let v = exact(view(Shading::MatCap(&image), Wire::Off, true));
    let shown = snap(r, &mesh, &v, SIZE);
    for (xs, id) in [(0.45..1.0, 5), (-1.0..-0.45, 9), (-0.15..0.15, 0)] {
        let [cr, cg, cb, _] = group_color(id, 0);
        let pixels = inside(&shown, xs.clone());
        assert!(pixels.len() > 50, "group {id} covers pixels");
        assert!(
            off_by(&pixels, [cr, cg, cb]) <= 1,
            "group {id} shows its color: off by {}",
            off_by(&pixels, [cr, cg, cb])
        );
    }
    assert_ne!(
        group_color(5, 0),
        group_color(9, 0),
        "two groups, two colors"
    );
}

/// A part showing its groups draws whole beside a part with none, whichever is drawn first.
/// On a device without barycentrics the two take different paths through one geometry pass
/// (vertex pulling and indexed), and the indexed part's culled draw must not cut the other
/// part's draw short.
#[test]
fn a_part_showing_groups_draws_whole_beside_a_part_without() {
    let r = lavapipe();
    let mut store = sphere_store(12, 6, 40);
    group_by_node(&mut store);
    let grouped = r.upload_store(&store);
    assert!(grouped.chunks().len() > 1, "the grouped part has chunks");
    let corner = Similarity {
        translation: Vec3::new(0.95, 0.95, 0.0),
        scale: 0.1,
        ..Similarity::IDENTITY
    };
    let other = upload(r, &scene::icosphere(1)).placed(corner, true);
    assert!(other.culling(), "the other part is culled when drawn alone");
    let image = white_image();
    let v = exact(view(Shading::MatCap(&image), Wire::Off, true));
    let size = RenderSize::square(SIZE);
    let alone = inside(&r.snapshot(&grouped, &v, size).expect("renders"), -1.0..1.0);
    for (order, meshes) in [("first", [&grouped, &other]), ("last", [&other, &grouped])] {
        // A culled frame first, as the window draws before its groups are turned on.
        r.snapshot(&other, &v, size).expect("renders");
        let both = r.snapshot(&meshes[..], &v, size).expect("renders");
        assert!(
            inside(&both, -1.0..1.0) == alone,
            "drawn {order}, the grouped part draws as it does alone"
        );
    }
}

/// A store's colors and groups draw like its plain export's: each point's color reaches its
/// vertex slot and each face's group its triangles, wherever the node layout puts them.
#[test]
fn a_painted_store_draws_like_its_plain_export() {
    let r = lavapipe();
    let mut store = sphere_store(12, 6, 40);
    let color = store.add_attr(AttrDef::color()).expect("registers");
    let nodes = all(&store);
    paint(&mut store, color, &nodes, |p| word(red_right(p)));
    group_by_node(&mut store);
    let direct = r.upload_store(&store);
    // The store's own normals, as `store_draws_like_its_plain_export` passes them.
    let normals: Vec<[f32; 3]> = (nodes.iter())
        .flat_map(|&n| store.node(n).nrm.iter().map(|m| m.to_array()))
        .collect();
    let plain = store.to_plain();
    let exported = r
        .upload(
            RenderMesh::from(&plain).with_normals(&normals),
            UploadOptions::default(),
        )
        .expect("uploads");
    let (gray, image) = (gray_look(), white_image());
    for (what, shading, groups) in [
        ("colors", Shading::MatCap(&gray), false),
        ("groups", Shading::MatCap(&image), true),
    ] {
        let v = view(shading, Wire::Off, groups);
        assert!(
            snap(r, &direct, &v, SIZE) == snap(r, &exported, &v, SIZE),
            "the store's {what} draw like its export's"
        );
    }
}

/// Every node of `store`.
fn all(store: &MeshStore) -> Vec<NodeId> {
    (0..store.node_count() as NodeId).collect()
}

/// Gives `store` a group column: node n's faces in group 3 + n % 4.
fn group_by_node(store: &mut MeshStore) {
    let group = store.add_attr(AttrDef::face_group()).expect("registers");
    let faces: Vec<Vec<u32>> = (0..store.node_count() as NodeId)
        .map(|n| vec![3 + n % 4; store.node(n).faces.len()])
        .collect();
    write_words(store, group, &all(store), &faces);
}

/// Writes column `col` of `nodes` from each point's position.
fn paint(store: &mut MeshStore, col: AttrId, nodes: &[NodeId], color: impl Fn([f32; 3]) -> u32) {
    let words: Vec<Vec<u32>> = (0..store.node_count() as NodeId)
        .map(|n| {
            store
                .node(n)
                .pos
                .iter()
                .map(|p| color(p.to_array()))
                .collect()
        })
        .collect();
    write_words(store, col, nodes, &words);
}

/// Writes word column `col` of `nodes` from `words`, one list per node of the store.
fn write_words(store: &mut MeshStore, col: AttrId, nodes: &[NodeId], words: &[Vec<u32>]) {
    store.write(nodes, ColumnSet::attr(col)).each(|m| {
        let node = m.id() as usize;
        let AttrVec::U32(v) = m.attr(col) else {
            panic!("a word column")
        };
        v.copy_from_slice(&words[node]);
    });
}

/// The first paint on a part sends its whole color stream once; after that a paint-only change
/// sends the color words of the nodes it painted and nothing else (no positions, normals,
/// faces or culling records), and its undo sends the same. The GPU then holds what a fresh
/// upload holds.
#[test]
fn a_paint_only_change_sends_only_its_nodes_colors_and_so_does_its_undo() {
    let r = lavapipe();
    let mut store = sphere_store(8, 6, 40);
    let color = store.add_attr(AttrDef::color()).expect("registers");
    store.take_dirty();
    let mut mesh = r.upload_store(&store);
    let mut sync = |store: &mut MeshStore| {
        let dirty = store.take_dirty();
        assert!(!dirty.is_empty(), "the change is reported dirty");
        r.update_store(&mut mesh, store, &dirty)
    };
    paint(&mut store, color, &[0], |_| word(RED));
    let slots: u64 = all(&store)
        .iter()
        .map(|&n| store.node(n).pos.len() as u64)
        .sum();
    let first = sync(&mut store);
    assert_eq!(
        (first.nodes, first.faces, first.full, first.bytes),
        (0, 0, false, 4 * slots),
        "the first paint sends the whole stream once"
    );
    let words = 4 * store.node(2).pos.len() as u64;
    store.begin_step();
    paint(&mut store, color, &[2], |p| word(red_right(p)));
    store.end_step();
    for what in ["the paint", "its undo"] {
        let sent = sync(&mut store);
        assert_eq!(
            (sent.nodes, sent.faces, sent.full, sent.bytes),
            (0, 0, false, words),
            "{what} sends node 2's colors only"
        );
        if what == "the paint" {
            assert!(store.undo().is_some(), "the paint undoes");
        }
    }
    let (held, fresh) = (r.read_pools(&mesh), r.read_pools(&r.upload_store(&store)));
    assert!(
        held.expect("reads back") == fresh.expect("reads back"),
        "the GPU holds what a fresh upload holds, colors included"
    );
}
