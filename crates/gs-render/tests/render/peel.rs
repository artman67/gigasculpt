//! Live Boolean's graphics card preview (decision 0057, section 9, as amended for LB8): a
//! group's operands peeled in depth draw the cut the CPU makes of them, wherever the operands
//! are now. Each picture here paints a point by where it is (red, green and blue for x, y and
//! z) and draws it flat, so a pixel tells the depth of the surface it shows: two pictures of the
//! same cut agree but at silhouettes and seams.

use crate::common::{BACKGROUND, fitted, front_ortho, lavapipe, snap, upload};
use crate::scene;
use gs_core::columns::{linear_to_srgb, srgb_to_linear};
use gs_core::transform::Similarity;
use gs_core::{Quat, Vec3};
use gs_geo::boolean::{Operand as Cut, fold};
use gs_geo::operation::Operation;
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_render::{
    GpuMesh, Image, Operand, PEEL_OPERANDS, Preset, RenderSize, Scene, Shading, UiLayer, View, Wire,
};

const SIZE: u32 = 128;
const WHITE: [u8; 3] = [255; 3];
const RED: [u8; 3] = [230, 40, 40];
const BLUE: [u8; 3] = [40, 90, 240];
/// The paint as stored: no light, so a pixel's color is its surface's place.
const FLAT: Shading<'static> = Shading::Flat([1.0; 3]);
/// The coordinates a color shows, from -RANGE to RANGE.
const RANGE: f32 = 1.6;
/// Pixels where the peel and the CPU's cut may disagree, at silhouettes and seams, and how far
/// apart in z a pixel both draw may be. Measured on lavapipe: no pixel in any picture here, and
/// at most 0.0254 in z, one step of the colors near the top of the range (about 0.025); bounded
/// with margin for other drivers' seams, and two steps.
const EDGE_PIXELS: usize = 16;
const DEPTH: f32 = 0.05;

/// A part of a test group: its mesh in its own space, where it is and how it joins.
type Part<'a> = (&'a PlainMesh, Similarity, Operation);

/// The byte that shows coordinate `v`: its place in the range through the sRGB curve, so paint
/// interpolated in linear light stays linear in position across any triangle.
fn encode(v: f32) -> u8 {
    let t = ((v + RANGE) / (2.0 * RANGE)).clamp(0.0, 1.0);
    (linear_to_srgb(t) * 255.0).round() as u8
}

/// The coordinate byte `b` shows (`encode` undone).
fn decode(b: u8) -> f32 {
    srgb_to_linear(f32::from(b) / 255.0) * 2.0 * RANGE - RANGE
}

/// `mesh` with a `color` column of `words`, one a point.
fn painted(mesh: &PlainMesh, words: Vec<u32>) -> PlainMesh {
    let mut mesh = mesh.clone();
    let column = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::U32(words),
    };
    mesh.attributes.insert("color".into(), column);
    mesh
}

/// `mesh` with each point painted by where `at` puts it.
fn located(mesh: &PlainMesh, at: Similarity) -> PlainMesh {
    let words = (mesh.positions.iter())
        .map(|&[x, y, z]| {
            let [r, g, b] = at.point(Vec3::new(x, y, z)).to_array().map(encode);
            u32::from_le_bytes([r, g, b, 255])
        })
        .collect();
    painted(mesh, words)
}

/// `mesh` all in `color`.
fn colored(mesh: &PlainMesh, [r, g, b]: [u8; 3]) -> PlainMesh {
    let word = u32::from_le_bytes([r, g, b, 255]);
    painted(mesh, vec![word; mesh.positions.len()])
}

/// The box from `lo` to `hi`, its quads wound outward.
fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> PlainMesh {
    let corner = |i: u32| std::array::from_fn(|k| if i >> k & 1 == 1 { hi[k] } else { lo[k] });
    let faces = [
        [0, 4, 6, 2],
        [1, 3, 7, 5],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 2, 3, 1],
        [4, 5, 7, 6],
    ];
    scene::plain((0..8).map(corner).collect(), faces.to_vec())
}

/// A scale by `scale` about the origin, then a move to `to`.
fn at(to: [f32; 3], scale: f32) -> Similarity {
    Similarity {
        translation: Vec3::new(to[0], to[1], to[2]),
        scale,
        ..Similarity::IDENTITY
    }
}

/// The unit sphere and a turned box poking out of it at its front right, and where the box is.
fn sphere_and_box() -> (PlainMesh, PlainMesh, Similarity) {
    let place = Similarity {
        rotation: Quat::about(Vec3::new(0.6, 0.8, 0.0), 0.5),
        ..at([0.5, 0.2, 0.45], 1.0)
    };
    (scene::icosphere(3), cuboid([-0.4; 3], [0.4; 3]), place)
}

/// The CPU's cut of `parts` (gs-geo's fold, what a live run makes), painted by place.
fn cut(parts: &[Part]) -> PlainMesh {
    let group: Vec<Cut> = (parts.iter())
        .map(|&(mesh, at, operation)| Cut {
            mesh,
            at,
            operation,
        })
        .collect();
    let made = fold(&group).expect("the group cuts");
    assert!(made.repaired.is_empty(), "no operand needs a repair");
    located(&made.mesh, Similarity::IDENTITY)
}

/// `operation` as a peeled operand with white walls, or the base for the first.
fn operand(first: bool, operation: Operation) -> Operand {
    match operation {
        _ if first => Operand::Base,
        Operation::Union => Operand::Union,
        Operation::Difference => Operand::Difference(WHITE),
        Operation::Intersect => Operand::Intersect(WHITE),
    }
}

/// `parts` uploaded painted by place, at their places, each peeled as its operation says (the
/// first as the base) or, without `peel`, drawn plainly.
fn uploaded(parts: &[Part], peel: bool) -> Vec<GpuMesh> {
    (parts.iter().enumerate())
        .map(|(i, &(mesh, place, operation))| {
            let peeled = peel.then(|| operand(i == 0, operation));
            (upload(lavapipe(), &located(mesh, place)).placed(place, true)).peeled(peeled)
        })
        .collect()
}

/// The CPU's cut of `parts`, uploaded, as LB7's path draws it.
fn cpu(parts: &[Part]) -> Vec<GpuMesh> {
    vec![upload(lavapipe(), &cut(parts))]
}

fn picture(meshes: &[GpuMesh], view: &View) -> Image {
    let meshes: Vec<&GpuMesh> = meshes.iter().collect();
    snap(lavapipe(), &meshes[..], view, SIZE)
}

/// Where two pictures disagree: pixels only one covers, and pixels both cover more than `depth`
/// apart in z (from their blue), with the largest gap between pixels both cover.
#[derive(Debug)]
struct Mismatch {
    coverage: usize,
    deeper: usize,
    worst: f32,
}

impl Mismatch {
    fn of(a: &Image, b: &Image, depth: f32) -> Mismatch {
        let mut m = Mismatch {
            coverage: 0,
            deeper: 0,
            worst: 0.0,
        };
        for (p, q) in a.rgba.chunks(4).zip(b.rgba.chunks(4)) {
            match (p == BACKGROUND, q == BACKGROUND) {
                (true, true) => {}
                (false, false) => {
                    let gap = (decode(p[2]) - decode(q[2])).abs();
                    m.worst = m.worst.max(gap);
                    m.deeper += usize::from(gap > depth);
                }
                _ => m.coverage += 1,
            }
        }
        m
    }

    fn pixels(&self) -> usize {
        self.coverage + self.deeper
    }
}

/// The peel's picture of `peel` agrees with the CPU's of `cpu` but at silhouettes and seams.
fn assert_matches(peel: &Image, cpu: &Image, what: &str) {
    let m = Mismatch::of(peel, cpu, DEPTH);
    assert!(
        m.pixels() <= EDGE_PIXELS,
        "{what}: the peel draws the CPU's cut: {m:?}"
    );
}

/// The front view and a turned one with perspective, fitted to `mesh`.
fn views(mesh: &GpuMesh) -> [(View<'static>, &'static str); 2] {
    [
        (front_ortho(FLAT), "front"),
        (fitted(mesh, Preset::Iso, FLAT, Wire::Off), "iso"),
    ]
}

#[test]
fn a_peeled_sphere_and_box_draw_the_cpus_difference_intersect_and_union() {
    let (sphere, cube, place) = sphere_and_box();
    let base = (&sphere, Similarity::IDENTITY, Operation::Union);
    let alone = uploaded(&[base], false);
    for operation in [
        Operation::Difference,
        Operation::Intersect,
        Operation::Union,
    ] {
        let parts = [base, (&cube, place, operation)];
        for (view, name) in views(&alone[0]) {
            let made = picture(&cpu(&parts), &view);
            let what = format!("{operation:?} from the {name}");
            assert_matches(&picture(&uploaded(&parts, true), &view), &made, &what);
            let changed = Mismatch::of(&made, &picture(&alone, &view), DEPTH).pixels();
            assert!(
                changed > 10 * EDGE_PIXELS,
                "{what}: the cut shows: {changed}"
            );
        }
    }
}

#[test]
fn a_moved_cutter_peels_its_hole_where_it_is_now() {
    let (sphere, cube, was) = sphere_and_box();
    let now = Similarity {
        translation: Vec3::new(-0.3, -0.35, 0.5),
        ..was
    };
    let base = (&sphere, Similarity::IDENTITY, Operation::Union);
    let view = front_ortho(FLAT);
    let last = picture(&cpu(&[base, (&cube, was, Operation::Difference)]), &view);
    let parts = [base, (&cube, now, Operation::Difference)];
    let peel = picture(&uploaded(&parts, true), &view);
    assert_matches(&peel, &picture(&cpu(&parts), &view), "moved");
    let moved = Mismatch::of(&peel, &last, DEPTH).pixels();
    assert!(
        moved > 10 * EDGE_PIXELS,
        "LB7's last result draws the hole where it was: {moved}"
    );
}

#[test]
fn three_operands_peel_as_the_cpu_cuts_them() {
    let (sphere, cube, place) = sphere_and_box();
    let small = scene::icosphere(2);
    let parts = [
        (&sphere, Similarity::IDENTITY, Operation::Union),
        (&cube, place, Operation::Difference),
        (&small, at([0.45, 0.2, 0.55], 0.35), Operation::Union),
    ];
    let peel = uploaded(&parts, true);
    for (view, name) in views(&peel[0]) {
        let made = picture(&cpu(&parts), &view);
        assert_matches(&picture(&peel, &view), &made, name);
    }
}

/// Frame pixel `(x, y)`'s center in the front view.
fn center(x: usize, y: usize) -> [f32; 2] {
    let step = 2.2 / SIZE as f32;
    [(x as f32 + 0.5) * step - 1.1, 1.1 - (y as f32 + 0.5) * step]
}

#[test]
fn equal_depth_goes_to_the_earlier_operand_and_a_flush_cutter_opens_its_pocket() {
    let r = lavapipe();
    let left = cuboid([-0.8, -0.5, -0.5], [0.2, 0.5, 0.5]);
    let right = cuboid([-0.2, -0.5, -0.5], [0.8, 0.5, 0.5]);
    let view = front_ortho(FLAT);
    let flush = [
        upload(r, &colored(&left, RED)).peeled(Some(Operand::Base)),
        upload(r, &colored(&right, BLUE)).peeled(Some(Operand::Union)),
    ];
    let image = picture(&flush, &view);
    let mut shared = 0;
    for (i, p) in image.rgba.chunks(4).enumerate() {
        let [x, y] = center(i % SIZE as usize, i / SIZE as usize);
        if x.abs() < 0.18 && y.abs() < 0.45 {
            shared += 1;
            assert_eq!(
                p[..3],
                RED,
                "({x}, {y}): the base's face, drawn first, shows"
            );
        }
    }
    assert!(shared > 300, "the shared face covers {shared} pixels");
    let id = Similarity::IDENTITY;
    let parts = [
        (&left, id, Operation::Union),
        (&right, id, Operation::Difference),
    ];
    let peel = uploaded(&parts, true);
    for (view, name) in views(&peel[0]) {
        let made = picture(&cpu(&parts), &view);
        let what = format!("flush pocket from the {name}");
        assert_matches(&picture(&peel, &view), &made, &what);
    }
}

/// A base sphere and `cutters` small spheres cutting its front, peeled or drawn plainly.
fn many(cutters: usize, peel: bool) -> Vec<GpuMesh> {
    let (sphere, small) = (scene::icosphere(3), scene::icosphere(1));
    let places: Vec<Similarity> = (0..cutters)
        .map(|i| {
            let (u, v) = ((i % 6) as f32, (i / 6) as f32);
            at([-0.75 + 0.3 * u, -0.75 + 0.3 * v, 0.7], 0.12)
        })
        .collect();
    let base = (&sphere, Similarity::IDENTITY, Operation::Union);
    let parts: Vec<Part> = std::iter::once(base)
        .chain(places.iter().map(|&p| (&small, p, Operation::Difference)))
        .collect();
    uploaded(&parts, peel)
}

#[test]
fn more_than_32_operands_draw_as_they_would_unpeeled() {
    let view = front_ortho(FLAT);
    let over = picture(&many(PEEL_OPERANDS, true), &view);
    assert!(
        over == picture(&many(PEEL_OPERANDS, false), &view),
        "a group of {} draws its operands plainly",
        PEEL_OPERANDS + 1
    );
    let fits = Mismatch::of(
        &picture(&many(PEEL_OPERANDS - 1, true), &view),
        &picture(&many(PEEL_OPERANDS - 1, false), &view),
        DEPTH,
    );
    assert!(fits.pixels() > 300, "a group of 32 peels: {fits:?}");
}

#[test]
fn an_operand_left_out_of_the_peel_draws_plainly_beside_the_cut() {
    let (sphere, cube, place) = sphere_and_box();
    let parts = [
        (&sphere, Similarity::IDENTITY, Operation::Union),
        (&cube, place, Operation::Difference),
    ];
    let messy = scene::icosphere(2);
    let aside = at([-0.55, -0.5, 0.5], 0.4);
    let plain = || upload(lavapipe(), &located(&messy, aside)).placed(aside, true);
    let mut peel = uploaded(&parts, true);
    peel.push(plain());
    let mut made = cpu(&parts);
    made.push(plain());
    for (view, name) in views(&peel[0]) {
        let what = format!("messy from the {name}");
        assert_matches(&picture(&peel, &view), &picture(&made, &view), &what);
    }
}

fn frame(target: &gs_render::FrameTarget, meshes: &[GpuMesh], view: View) -> Image {
    let r = lavapipe();
    let meshes: Vec<&GpuMesh> = meshes.iter().collect();
    let scene = Scene {
        meshes: &meshes,
        view,
        viewport: [0, 0, SIZE, SIZE],
    };
    r.draw_frame(target, &scene, &mut r.ui_painter(), &UiLayer::default())
        .expect("draws");
    r.read_frame(target).expect("reads")
}

#[test]
fn the_peel_switched_off_draws_as_if_never_on_and_holds_no_targets() {
    let (sphere, cube, place) = sphere_and_box();
    let parts = [
        (&sphere, Similarity::IDENTITY, Operation::Union),
        (&cube, place, Operation::Difference),
    ];
    let view = front_ortho(FLAT);
    let target = lavapipe()
        .frame_target(RenderSize::square(SIZE))
        .expect("target");
    assert_eq!(target.peel_bytes(), 0, "nothing before a frame peels");
    frame(&target, &uploaded(&parts, true), view);
    let held = target.peel_bytes();
    assert!(held >= 22 * u64::from(SIZE * SIZE), "the targets: {held}");
    let off = frame(&target, &uploaded(&parts, false), view);
    assert_eq!(
        target.peel_bytes(),
        0,
        "a frame that peels nothing frees them"
    );
    assert!(
        off == picture(&uploaded(&parts, false), &view),
        "the frame after is byte for byte one never peeled"
    );
}

#[test]
fn the_peel_draws_the_same_bytes_at_1_and_24_threads() {
    let (sphere, cube, place) = sphere_and_box();
    let small = scene::icosphere(2);
    let parts = [
        (&sphere, Similarity::IDENTITY, Operation::Union),
        (&cube, place, Operation::Difference),
        (&small, at([0.45, 0.2, 0.55], 0.35), Operation::Union),
    ];
    let peel = uploaded(&parts, true);
    let [(view, _), (iso, _)] = views(&peel[0]);
    let draw =
        |threads| gs_par::with_threads(threads, || [picture(&peel, &view), picture(&peel, &iso)]);
    assert!(draw(1) == draw(24), "the same bytes");
}
