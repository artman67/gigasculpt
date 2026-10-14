//! The deformer preview on the graphics card (decision 0041, amended 2026-10-12 for `DF9a`): the
//! vertex stage moves each point of a part's open-time buffers by DF2's seven closed forms, from
//! a small uniform block and one weight per slot. Read back from the GPU, every point lands
//! within section 6's tolerance of where gs-core's own transform (Accept's code) puts it, under
//! a mask, a falloff, a gradient and axes left out too, and its normal is the map's (section
//! 4); a drawn frame shows it. This is the test that keeps the WGSL twins and gs-core's
//! formulas together (section 9): a change to either side alone fails it. Amended for `DF9b`:
//! noise, the cage and the curve too, their tables written by gs-core and read by the vertex
//! stage, each table sent only when it changes.

use gs_core::deform::{Curve, Deform, Gradient, Lattice, Offset, Scope};
use gs_core::symmetry::Symmetry;
use gs_core::transform::{Distance, Frame, Op, RIGID, Transform, Weights};
use gs_core::{Aabb, AttrDef, AttrVec, ColumnSet, MeshStore, NodeId, Quat, Vec3};
use gs_render::{Image, Preset, Preview, Renderer, Shading, Wire};

use crate::common::{covered, fitted, lavapipe, snap};
use crate::store::sphere_store;

/// Section 6: a point of the preview is within this fraction of the part's diagonal of
/// Accept's.
const TOLERANCE: f32 = 1e-3;

/// The seven closed forms, each moving the test part well past rounding.
fn operations() -> [Deform; 7] {
    [
        Deform::Bend {
            axis: 1,
            toward: 0,
            angle: 2.4,
        },
        Deform::Fold {
            axis: 0,
            toward: 2,
            angle: 1.2,
        },
        Deform::Twist {
            axis: 2,
            angle: 2.0,
        },
        Deform::Taper {
            axis: 1,
            amount: 0.6,
            exponent: 1.5,
        },
        Deform::Shear {
            axis: 0,
            toward: 1,
            amount: 0.4,
            smooth: true,
        },
        Deform::Clip {
            axis: 2,
            depth: 0.5,
        },
        Deform::Spherize {
            amount: 0.7,
            radius: Some(0.8),
        },
    ]
}

/// Noise, the hard cage, the soft cage, a curve through three points and one through 32, each
/// with a twist and a scale, each moving the test part well past rounding (`DF9b`).
fn drawn() -> [Deform; 5] {
    let offset = |at, by| Offset { at, by };
    let few = Curve {
        axis: 1,
        points: Some(vec![[0.0, -1.1, 0.0], [0.4, -0.1, 0.1], [0.1, 0.95, 0.3]]),
        twist: Some(vec![0.0, 1.2, -0.6]),
        scale: Some(vec![1.0, 1.3, 0.7]),
    };
    [
        Deform::Noise {
            amount: 0.08,
            scale: 0.3,
            seed: 9,
        },
        Deform::Lattice(Lattice {
            soft: false,
            size: [4, 3, 5],
            offsets: vec![
                offset([1, 1, 2], [0.2, -0.15, 0.1]),
                offset([2, 0, 3], [-0.1, 0.25, 0.0]),
                offset([3, 2, 4], [0.05, 0.1, -0.2]),
            ],
        }),
        Deform::Lattice(Lattice {
            soft: true,
            size: [4, 4, 4],
            offsets: vec![
                offset([1, 2, 1], [0.0, 0.3, 0.1]),
                offset([2, 1, 2], [0.2, -0.1, 0.15]),
                offset([0, 3, 3], [-0.15, 0.0, 0.2]),
            ],
        }),
        Deform::Curve(few),
        Deform::Curve(wave(0.0)),
    ]
}

/// A curve along the frame's z through 32 points, a wave across both other axes, twisted by up
/// to 2.5 radians plus `turn` and scaled between 0.7 and 1.3.
fn wave(turn: f32) -> Curve {
    let at = |i: usize| i as f32 / 31.0;
    let points = (0..32)
        .map(|i| {
            let z = -1.0 + 2.05 * at(i);
            [
                0.25 * libm::sinf(3.0 * z),
                0.2 * libm::cosf(2.2 * z) - 0.2,
                z,
            ]
        })
        .collect();
    Curve {
        axis: 2,
        points: Some(points),
        twist: Some((0..32).map(|i| 2.5 * at(i) + turn).collect()),
        scale: Some(
            (0..32)
                .map(|i| 1.0 + 0.3 * libm::sinf(i as f32 / 4.0))
                .collect(),
        ),
    }
}

/// How a transform weighs the part's points.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Weighing {
    Rigid,
    /// The part's mask, which changes from point to point.
    Mask,
    Falloff,
    Gradient,
    /// The frame's Y left out.
    Axes,
}

/// `op` about a frame off the part's centre and turned, with a box the sphere pokes out of.
fn transform(op: Deform, weighing: Weighing) -> Transform {
    let soft = weighing == Weighing::Falloff;
    let gradient = Gradient {
        axis: 1,
        from: 0.2,
        to: 0.8,
    };
    Transform {
        frame: Frame {
            origin: Vec3::new(0.05, -0.04, 0.03),
            axes: Quat::about(Vec3::new(0.3, 1.0, 0.2).normalized(), 0.4),
        },
        op: Op::Deform(op),
        weights: Weights {
            mask: true,
            falloff: if soft { 0.0 } else { RIGID },
            reach: 1.3,
            distance: Distance::Straight,
        },
        symmetry: Symmetry::default(),
        scope: Scope {
            axes: [true, weighing != Weighing::Axes, true],
            gradient: (weighing == Weighing::Gradient).then_some(gradient),
            bounds: Some(Aabb {
                lo: Vec3::new(-0.9, -1.1, -1.0),
                hi: Vec3::new(1.0, 0.95, 1.05),
            }),
        },
    }
}

/// The test part: a dented quad sphere in nodes of up to 40 points, with a mask from free to
/// fully held that changes from point to point when `masked`.
fn part(masked: bool) -> MeshStore {
    let mut store = sphere_store(12, 6, 40);
    if masked {
        let mask = store.add_attr(AttrDef::mask()).expect("registers");
        let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
        store.write(&nodes, ColumnSet::attr(mask)).each(|m| {
            if let AttrVec::F32(v) = m.attr(mask) {
                for (i, x) in v.iter_mut().enumerate() {
                    *x = (i % 5) as f32 / 4.0;
                }
            }
        });
    }
    store.take_dirty();
    store
}

/// The diagonal of the box around the part's live points.
fn diagonal(store: &MeshStore) -> f32 {
    let far = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
    let (mut lo, mut hi) = (far, -far);
    for n in 0..store.node_count() as NodeId {
        for &p in store.node(n).pos.iter().filter(|p| !p.is_dead()) {
            (lo, hi) = (lo.min(p), hi.max(p));
        }
    }
    (hi - lo).len2().sqrt()
}

/// One chain run both ways: each slot's position and normal as the GPU draws it, the part as
/// Accept leaves it, and the weights the GPU was given.
struct Run {
    gpu: Vec<([f32; 3], [f32; 3])>,
    cpu: MeshStore,
    weights: Vec<f32>,
}

fn run(r: &Renderer, store: &MeshStore, ops: &[Transform]) -> Run {
    let weights = store.weights(&ops[0]).expect("no plane splits the part");
    let mut mesh = r.upload_store(store);
    let preview = Preview {
        ops,
        weights: &weights,
        weighed: 1,
    };
    r.preview(&mut mesh, Some(preview));
    let gpu = r.read_preview(&mesh).expect("reads back");
    let mut cpu = store.share();
    for t in ops {
        cpu.transform(t);
    }
    Run { gpu, cpu, weights }
}

/// Normal `n` at `p` carried by the chain at weight `w`: the cofactor of the chain's
/// derivative, by central differences `h` apart of gs-core's own formula, then by each choice
/// of one-sided ones, since at a kink (a box face, a fold's plane, a cage's cell plane, a
/// curve's junction or point) the map has a derivative each side.
fn mapped(ops: &[Transform], (p, n): (Vec3, Vec3), w: f32, h: f32) -> Vec<Vec3> {
    let at = |q: Vec3| ops.iter().fold(q, |q, t| t.deformed(q, w));
    let unit = [
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let here = at(p);
    let sides = unit.map(|e| {
        [
            (at(p + e * h) - here) * (1.0 / h),
            (here - at(p - e * h)) * (1.0 / h),
        ]
    });
    let cofactor = |f: [Vec3; 3]| {
        (f[1].cross(f[2]) * n.x + f[2].cross(f[0]) * n.y + f[0].cross(f[1]) * n.z).normalized()
    };
    let central = cofactor(sides.map(|[a, b]| (a + b) * 0.5));
    let one_sided = (0..8).map(|k| cofactor([0, 1, 2].map(|a| sides[a][k >> a & 1])));
    std::iter::once(central).chain(one_sided).collect()
}

fn degrees(a: Vec3, b: Vec3) -> f32 {
    let cos = a.normalized().dot(b.normalized()).clamp(-1.0, 1.0);
    libm::acosf(cos).to_degrees()
}

/// The worst position error as a fraction of the part's diagonal, and the share of moved
/// points whose normal is more than a degree off the map's each way (a point a difference's
/// step from two kinks straddles one, so a few are).
fn errors(store: &MeshStore, ops: &[Transform], run: &Run) -> (f32, f32) {
    let diag = diagonal(store);
    let (mut worst, mut off, mut moved, mut i) = (0.0_f32, 0, 0, 0);
    for n in 0..store.node_count() as NodeId {
        let (open, done) = (store.node(n), run.cpu.node(n));
        for l in 0..open.pos.len() {
            let ((p, nrm), w) = (run.gpu[i], run.weights[i]);
            i += 1;
            if open.pos[l].is_dead() {
                continue;
            }
            worst = worst.max((Vec3::from_array(p) - done.pos[l]).len2().sqrt() / diag);
            if w != 0.0 {
                moved += 1;
                let want = mapped(ops, (open.pos[l], open.nrm[l]), w, 1e-4 * diag);
                let drawn = Vec3::from_array(nrm);
                off += usize::from(want.iter().all(|&m| degrees(drawn, m) > 1.0));
            }
        }
    }
    assert_eq!(i, run.gpu.len(), "one point read back per slot");
    (worst, off as f32 / moved.max(1) as f32)
}

#[test]
fn each_operation_on_the_card_lands_where_accept_puts_it() {
    use Weighing::{Axes, Falloff, Gradient, Mask, Rigid};
    let r = lavapipe();
    for weighing in [Rigid, Mask, Falloff, Gradient, Axes] {
        let store = part(weighing == Mask);
        for op in operations() {
            let ops = [transform(op.clone(), weighing)];
            let (worst, off) = errors(&store, &ops, &run(r, &store, &ops));
            assert!(
                worst <= TOLERANCE && off <= 0.03,
                "{op:?} under {weighing:?}: points off by {worst:.2e} of the diagonal, \
                 {:.1}% of normals off the map's",
                off * 100.0
            );
        }
    }
}

#[test]
fn a_chain_of_four_rigid_operations_lands_where_accept_puts_it() {
    let r = lavapipe();
    let store = part(true);
    let ops = [
        Deform::Twist {
            axis: 0,
            angle: 1.1,
        },
        Deform::Twist {
            axis: 2,
            angle: -0.8,
        },
        Deform::Bend {
            axis: 1,
            toward: 2,
            angle: 0.9,
        },
        Deform::Spherize {
            amount: 0.3,
            radius: Some(1.0),
        },
    ]
    .map(|op| transform(op, Weighing::Mask));
    let (worst, off) = errors(&store, &ops, &run(r, &store, &ops));
    assert!(
        worst <= TOLERANCE && off <= 0.03,
        "points off by {worst:.2e} of the diagonal, {:.1}% of normals off",
        off * 100.0
    );
}

#[test]
fn noise_the_cage_and_the_curve_land_where_accept_puts_them() {
    use Weighing::{Axes, Falloff, Gradient, Mask, Rigid};
    let r = lavapipe();
    for weighing in [Rigid, Mask, Falloff, Gradient, Axes] {
        let store = part(weighing == Mask);
        for (i, op) in drawn().into_iter().enumerate() {
            let ops = [transform(op, weighing)];
            let (worst, off) = errors(&store, &ops, &run(r, &store, &ops));
            assert!(
                worst <= TOLERANCE && off <= 0.03,
                "operation {i} of `drawn` under {weighing:?}: points off by {worst:.2e} of the \
                 diagonal, {:.1}% of normals off the map's",
                off * 100.0
            );
        }
    }
}

#[test]
fn a_chain_of_cages_noise_and_a_curve_reads_each_table_where_it_lies() {
    let r = lavapipe();
    let store = part(true);
    let [noise, hard, soft, few, _] = drawn();
    let ops = [hard, few, noise, soft].map(|op| transform(op, Weighing::Mask));
    let (worst, off) = errors(&store, &ops, &run(r, &store, &ops));
    assert!(
        worst <= TOLERANCE && off <= 0.03,
        "points off by {worst:.2e} of the diagonal, {:.1}% of normals off",
        off * 100.0
    );
}

/// Pixels that differ by more than a little in any channel.
fn differ(a: &Image, b: &Image) -> usize {
    (a.rgba.chunks(4).zip(b.rgba.chunks(4)))
        .filter(|(p, q)| p.iter().zip(*q).any(|(x, y)| x.abs_diff(*y) > 24))
        .count()
}

#[test]
fn the_drawn_preview_looks_as_accept_leaves_the_part() {
    let r = lavapipe();
    let store = part(false);
    let bend = Deform::Bend {
        axis: 1,
        toward: 0,
        angle: 1.4,
    };
    let ops = [transform(bend, Weighing::Rigid)];
    let weights = store.weights(&ops[0]).expect("no plane splits the part");
    let mut mesh = r.upload_store(&store);
    let accepted = r.upload_store(&run(r, &store, &ops).cpu);
    for wire in [Wire::Off, Wire::VertexPulling] {
        let view = fitted(&accepted, Preset::Front, Shading::Clay, wire);
        let open = snap(r, &mesh, &view, 96);
        let preview = Preview {
            ops: &ops,
            weights: &weights,
            weighed: 1,
        };
        r.preview(&mut mesh, Some(preview));
        let shown = snap(r, &mesh, &view, 96);
        r.preview(&mut mesh, None);
        let (back, kept) = (snap(r, &mesh, &view, 96), snap(r, &accepted, &view, 96));
        let all = covered(&kept);
        assert!(
            differ(&open, &shown) > all / 10,
            "{wire:?}: the bend is drawn"
        );
        let off = differ(&shown, &kept);
        assert!(
            off <= all / 50,
            "{wire:?}: {off} of {all} pixels off Accept's"
        );
        assert!(
            back.rgba == open.rgba,
            "{wire:?}: dropped, it draws as before"
        );
    }
}

#[test]
fn the_weights_go_up_once_and_the_block_only_when_it_changes() {
    let r = lavapipe();
    let store = part(false);
    let bend = |angle| {
        let op = Deform::Bend {
            axis: 1,
            toward: 0,
            angle,
        };
        [transform(op, Weighing::Rigid)]
    };
    let weights = store
        .weights(&bend(1.0)[0])
        .expect("no plane splits the part");
    let slots: u64 = (0..store.node_count() as NodeId)
        .map(|n| store.node(n).pos.len() as u64)
        .sum();
    let mut mesh = r.upload_store(&store);
    let view = fitted(&mesh, Preset::Front, Shading::Clay, Wire::Off);
    let plain = snap(r, &mesh, &view, 64);
    let mut send = |ops: &[Transform]| {
        let preview = Preview {
            ops,
            weights: &weights,
            weighed: 7,
        };
        r.preview(&mut mesh, Some(preview))
    };
    let first = send(&bend(1.0));
    assert!(
        first >= 2 * slots && first <= 2 * slots + 1024,
        "the weights, 2 bytes a slot, and the block: {first} for {slots} slots"
    );
    assert_eq!(send(&bend(1.0)), 0, "the same block again sends nothing");
    let block = send(&bend(0.5));
    assert!(
        block > 0 && block <= 1024,
        "a new angle sends the block: {block}"
    );
    assert_eq!(send(&[]), 0, "no operations bind the still block");
    assert!(send(&bend(0.5)) <= 1024, "and the weights stay");
    assert_eq!(r.preview(&mut mesh, None), 0, "dropping it sends nothing");
    let back = snap(r, &mesh, &view, 64);
    assert!(back.rgba == plain.rgba, "and the part draws as before");
}

#[test]
fn a_table_goes_up_only_when_it_changes() {
    let r = lavapipe();
    let store = part(false);
    let cage = |lift: f32| {
        let offsets = vec![Offset {
            at: [1, 2, 1],
            by: [0.0, lift, 0.1],
        }];
        let op = Deform::Lattice(Lattice {
            soft: true,
            size: [4, 4, 4],
            offsets,
        });
        [transform(op, Weighing::Rigid)]
    };
    let curve = |turn: f32| [transform(Deform::Curve(wave(turn)), Weighing::Rigid)];
    let weights = (store.weights(&cage(0.3)[0])).expect("no plane splits the part");
    let slots = weights.len() as u64;
    let mut mesh = r.upload_store(&store);
    let mut send = |ops: &[Transform]| {
        let preview = Preview {
            ops,
            weights: &weights,
            weighed: 3,
        };
        r.preview(&mut mesh, Some(preview))
    };
    // A header, then the 4 by 4 by 4 cage padded to 6 by 6 by 6, 16 bytes an entry.
    let table = 16 * (1 + 6 * 6 * 6);
    let first = send(&cage(0.3));
    assert!(
        first >= 2 * slots + table && first <= 2 * slots + table + 1024,
        "the weights, the block and the cage's table: {first} for {slots} slots"
    );
    assert_eq!(send(&cage(0.3)), 0, "the same cage again sends nothing");
    assert_eq!(
        send(&cage(0.5)),
        table,
        "a moved cage point sends its table alone"
    );
    let bent = send(&curve(0.0));
    assert!(
        bent > 0 && bent <= 1024 + 16 * 350,
        "a curve sends its table and the block: {bent}"
    );
    let twisted = send(&curve(0.4));
    assert!(
        twisted > 0 && twisted <= 16 * 350,
        "a twist sends the curve's table alone: {twisted}"
    );
    assert_eq!(send(&curve(0.4)), 0, "the same curve again sends nothing");
    assert_eq!(send(&[]), 0, "Reset sends nothing");
    assert_eq!(send(&curve(0.4)), 0, "and the curve after it neither");
    let noise = [transform(drawn()[0].clone(), Weighing::Rigid)];
    let block = send(&noise);
    assert!(
        block > 0 && block <= 1024,
        "noise sends the block alone: {block}"
    );
}
