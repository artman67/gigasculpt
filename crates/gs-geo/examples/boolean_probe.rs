//! Measures decision 0057's two open questions for step LB2 on research 26's meshes
//! (`tests/boolean/shapes.rs`), calling manifold-rust directly:
//!
//! - `tolerance`: the 32-bit export rule. Each case is cut once in 64 bits, then exported to 32
//!   bits plainly and after `set_tolerance` at 1e-7, 3e-7, 1e-6 and 3e-6 times the result's
//!   bounding box diagonal (with `collapse`, after raising epsilon to the same tolerance, so
//!   the library's collapse reaches it). Each export is imported again and scanned with the
//!   library's `has_self_intersections`, checked by `tests/boolean/crossing.rs`, compared with
//!   the plain export (faces of it not in the ruled one, bit for bit) and with the 64-bit volume.
//! - `robust`: `rebuild_solid` ("at least 1") with the operand and face number as constant
//!   channels on every corner: the two-shells and folded 20k operands at 1, 4 and 24 threads
//!   (output hash), the 1M self-crossing operand at 24 threads (time), and whether every output
//!   triangle names a valid source.
//! - `small`: the pair at 10k to 200k triangles and at 10k moved away from the origin, for a cut
//!   whose plain 32-bit export crosses itself, small enough for a gate test.
//!
//! ```text
//! gate/heavy.sh cargo run --release --locked -p gs-geo --example boolean_probe -- tolerance [case] [collapse]
//! ```
//!
//! Decision 0057's amendment of 2026-10-12 (step LB2) records what it measured.

#[expect(dead_code, reason = "shared with the tests, which use the rest")]
#[path = "../tests/boolean/shapes.rs"]
mod shapes;

#[path = "../tests/boolean/crossing.rs"]
mod crossing;

use std::io::Write;
use std::time::Instant;

use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_protocol::{NO_VERTEX, PlainMesh};
use manifold_rust::csg_tree::{CsgLeafNode, CsgNode};
use manifold_rust::linalg::{Mat3x4, Vec3 as V64};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{MeshGL, MeshGL64, OpType, WindingRule};

const FACTORS: [f64; 4] = [1e-7, 3e-7, 1e-6, 3e-6];

#[expect(clippy::disallowed_methods, reason = "a measurement reads the clock")]
fn now() -> Instant {
    Instant::now()
}

fn say(line: &str) {
    if let Err(e) = writeln!(std::io::stdout().lock(), "{line}") {
        panic!("{e}");
    }
}

/// The process's peak resident memory in GB, from `/proc/self/status`.
fn peak_gb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let line = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap_or("");
    let kb: f64 = (line.split_whitespace().nth(1)).map_or(0.0, |k| k.parse().unwrap_or(0.0));
    kb / 1024.0 / 1024.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let what = args.first().map_or("tolerance", String::as_str);
    let collapse = args.iter().any(|a| a == "collapse");
    let case = args.get(1).map(String::as_str).filter(|&c| c != "collapse");
    match what {
        "tolerance" => tolerance(case, collapse),
        "robust" => robust(),
        "small" => small(collapse),
        _ => say("usage: boolean_probe tolerance [case] [collapse] | robust | small [collapse]"),
    }
    say(&format!("peak memory {:.2} GB", peak_gb()));
}

/// The mesh imported with each triangle's face id its index, in one run named `id`.
fn import(mesh: &PlainMesh, id: u32) -> Manifold {
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    let gl = MeshGL {
        num_prop: 3,
        vert_properties: mesh.positions.iter().flatten().copied().collect(),
        tri_verts: tris.iter().flatten().copied().collect(),
        run_original_id: vec![id],
        run_index: vec![0, 3 * tris.len() as u32],
        face_id: (0..tris.len() as u32).collect(),
        ..MeshGL::default()
    };
    Manifold::from_mesh_gl(&gl)
}

/// The similarity as the library's 3 by 4 matrix (columns).
fn matrix(s: &Similarity) -> Mat3x4 {
    let [x, y, z, w] = s
        .matrix()
        .map(|c| V64::new(f64::from(c[0]), f64::from(c[1]), f64::from(c[2])));
    Mat3x4::from_cols(x, y, z, w)
}

fn leaf(m: &Manifold, s: &Similarity) -> CsgNode {
    CsgNode::leaf_node(CsgLeafNode::with_transform(m.as_impl().clone(), matrix(s)))
}

/// A 32-bit triangle mesh.
struct Mesh32 {
    positions: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
}

impl Mesh32 {
    fn plain(&self) -> PlainMesh {
        PlainMesh {
            positions: self.positions.clone(),
            faces: self
                .tris
                .iter()
                .map(|t| [t[0], t[1], t[2], NO_VERTEX])
                .collect(),
            ..PlainMesh::default()
        }
    }

    fn volume(&self) -> f64 {
        let at = |v: u32| self.positions[v as usize].map(f64::from);
        let six: f64 = (self.tris.iter())
            .map(|t| {
                let [a, b, c] = t.map(at);
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum();
        six / 6.0
    }

    /// Each triangle's three corner positions as bits, turned to start at the least.
    fn keys(&self) -> Vec<[u32; 9]> {
        let mut keys = gs_par::map(self.tris.len(), |t| {
            let c = self.tris[t].map(|v| self.positions[v as usize].map(f32::to_bits));
            let k = (0..3).min_by_key(|&k| c[k]).unwrap_or(0);
            let r = [c[k], c[(k + 1) % 3], c[(k + 2) % 3]];
            [r[0], r[1], r[2]].concat().try_into().unwrap_or([0; 9])
        });
        gs_par::sort(&mut keys);
        keys
    }
}

/// The manifold rounded to 32 bits, as an export would hand it on.
fn export32(m: &Manifold) -> Mesh32 {
    let gl = m.get_mesh_gl64(-1);
    let np = gl.num_prop as usize;
    let positions = (gl.vert_properties.chunks_exact(np))
        .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32])
        .collect();
    let tris = (gl.tri_verts.as_chunks::<3>().0.iter())
        .map(|t| t.map(|v| v as u32))
        .collect();
    Mesh32 { positions, tris }
}

/// How a 32-bit export fares: the library's scan after a fresh import, the crossing pairs, the
/// plain export's faces it lacks and its volume's relative error against the 64-bit result.
fn judge(out: &Mesh32, plain: &[[u32; 9]], volume64: f64) -> String {
    let t = now();
    let again = import(&out.plain(), 0);
    let scan = if again.status() == manifold_rust::types::Error::NoError {
        if again.has_self_intersections() {
            "FAILS"
        } else {
            "passes"
        }
    } else {
        "does not import"
    };
    let scanned = t.elapsed().as_secs_f64();
    let pairs = crossing::crossings(&out.positions, &out.tris);
    let keys = out.keys();
    let changed = plain
        .iter()
        .filter(|k| keys.binary_search(k).is_err())
        .count();
    let rel = (out.volume() - volume64).abs() / volume64;
    format!(
        "{} tris, scan {scan} ({scanned:.2} s with import), {} crossing pairs, {changed} faces \
         changed ({:.4}%), volume error {rel:.2e}",
        out.tris.len(),
        pairs.len(),
        100.0 * changed as f64 / plain.len().max(1) as f64
    )
}

fn diagonal(m: &Manifold) -> f64 {
    let b = m.bounding_box();
    let d = b.max - b.min;
    (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
}

/// The export rule at tolerance `tol`: the library's `set_tolerance`, which outside a boolean
/// collapses edges only up to the mesh's epsilon (its #1671), or with `collapse` the same after
/// raising epsilon to `tol`, so edges shorter than `tol` are collapsed too.
fn rule(result: &Manifold, tol: f64, collapse: bool) -> Manifold {
    if !collapse {
        return result.set_tolerance(tol);
    }
    let mut imp = result.as_impl().clone();
    imp.epsilon = imp.epsilon.max(tol);
    Manifold::from_impl(imp).set_tolerance(tol)
}

/// One cut and its exports under each tolerance.
fn measure(name: &str, a: &PlainMesh, b: &PlainMesh, at: &Similarity, op: OpType, collapse: bool) {
    let t = now();
    let (ma, mb) = (import(a, 0), import(b, 1));
    let imported = t.elapsed().as_secs_f64();
    let t = now();
    let result = Manifold::from_impl(
        CsgNode::op(op, leaf(&ma, &Similarity::IDENTITY), leaf(&mb, at)).evaluate(),
    );
    let cut = t.elapsed().as_secs_f64();
    drop((ma, mb));
    let (volume, diag) = (result.volume(), diagonal(&result));
    let scale = result.bounding_box().scale();
    say(&format!(
        "{name}: {} + {} tris, import {imported:.2} s, cut {cut:.2} s, {} tris, volume {volume:.6}, \
         diagonal {diag:.4}, largest coordinate {scale:.4}",
        a.triangles().count(),
        b.triangles().count(),
        result.num_tri()
    ));
    let plain = export32(&result);
    let plain_keys = plain.keys();
    say(&format!("  plain: {}", judge(&plain, &plain_keys, volume)));
    for k in FACTORS {
        let t = now();
        let ruled = rule(&result, k * diag, collapse);
        let ruled_time = t.elapsed().as_secs_f64();
        let out = export32(&ruled);
        say(&format!(
            "  {k:.0e} x diagonal ({ruled_time:.2} s): {}",
            judge(&out, &plain_keys, volume)
        ));
    }
}

fn tolerance(only: Option<&str>, collapse: bool) {
    let want = |name: &str| only.is_none_or(|o| name.contains(o));
    let pairs = [
        ("5M pair subtract", OpType::Subtract),
        ("5M pair add", OpType::Add),
        ("5M pair intersect", OpType::Intersect),
    ];
    if pairs.iter().any(|(name, _)| want(name)) {
        let a = shapes::sphere(456);
        let b = shapes::pair_b();
        for (name, op) in pairs.into_iter().filter(|(name, _)| want(name)) {
            measure(name, &a, &a, &b, op, collapse);
        }
    }
    if want("5M body cut") {
        let body = shapes::sphere(645);
        let cutter = shapes::sphere(41);
        measure(
            "5M body cut",
            &body,
            &cutter,
            &shapes::cutter(),
            OpType::Subtract,
            collapse,
        );
    }
}

/// The operand with its operand and face numbers as constant channels on every corner: each
/// triangle has its own three property vertices, merged to the first at the same point.
fn import_channels(mesh: &PlainMesh, operand: u32) -> Manifold {
    let tris: Vec<[u32; 3]> = mesh.triangles().collect();
    let mut props = Vec::with_capacity(15 * tris.len());
    let mut first = vec![u64::MAX; mesh.positions.len()];
    let (mut from, mut to) = (Vec::new(), Vec::new());
    for (t, tri) in tris.iter().enumerate() {
        for (k, &v) in tri.iter().enumerate() {
            let idx = (3 * t + k) as u64;
            let p = mesh.positions[v as usize].map(f64::from);
            props.extend([p[0], p[1], p[2], f64::from(operand), t as f64]);
            if first[v as usize] == u64::MAX {
                first[v as usize] = idx;
            } else {
                from.push(idx);
                to.push(first[v as usize]);
            }
        }
    }
    let gl = MeshGL64 {
        num_prop: 5,
        vert_properties: props,
        tri_verts: (0..3 * tris.len() as u64).collect(),
        merge_from_vert: from,
        merge_to_vert: to,
        ..MeshGL64::default()
    };
    Manifold::from_mesh_gl64(&gl)
}

/// FNV-1a over 64-bit words.
fn fnv(words: impl Iterator<Item = u64>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in words {
        for byte in w.to_le_bytes() {
            h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// One rebuild: its time, output hash (positions, corners and channels), triangle count,
/// volume, and the output triangles whose corners do not name one valid source.
fn rebuild(mesh: &PlainMesh) -> (f64, f64, u64, usize, f64, usize) {
    let faces = mesh.triangles().count() as f64;
    let t = now();
    let m = import_channels(mesh, 0);
    let imported = t.elapsed().as_secs_f64();
    let t = now();
    let r = m.rebuild_solid(WindingRule::Positive);
    let rebuilt = t.elapsed().as_secs_f64();
    let gl = r.get_mesh_gl64(-1);
    let np = gl.num_prop as usize;
    let channel = |v: u64, c: usize| gl.vert_properties[v as usize * np + c];
    let bad = (gl.tri_verts.as_chunks::<3>().0.iter())
        .filter(|t| {
            let ok = |v: u64| {
                let f = channel(v, 4);
                np == 5 && channel(v, 3) == 0.0 && f.fract() == 0.0 && (0.0..faces).contains(&f)
            };
            let same = |v: u64| channel(v, 4).to_bits() == channel(t[0], 4).to_bits();
            !(t.iter().all(|&v| ok(v) && same(v)))
        })
        .count();
    let words =
        (gl.vert_properties.iter().map(|x| x.to_bits())).chain(gl.tri_verts.iter().copied());
    (imported, rebuilt, fnv(words), r.num_tri(), r.volume(), bad)
}

fn robust() {
    let ball = shapes::sphere(41);
    for (name, mesh) in [
        ("two shells 20k", shapes::shells(&ball)),
        ("folded 20k", shapes::fold(&ball)),
    ] {
        for threads in [1, 4, 24] {
            let (i, r, hash, tris, volume, bad) = gs_par::with_threads(threads, || rebuild(&mesh));
            say(&format!(
                "{name} at {threads} threads: hash {hash:016x}, {tris} tris, volume {volume:.6}, \
                 {bad} without a valid source, import {i:.3} s, rebuild {r:.3} s"
            ));
        }
    }
    for n in [204, 289] {
        let mesh = shapes::shells(&shapes::sphere(n));
        let count = mesh.triangles().count();
        let (i, r, hash, tris, volume, bad) = gs_par::with_threads(24, || rebuild(&mesh));
        say(&format!(
            "self-crossing {count} tris at 24 threads: import {i:.2} s, rebuild {r:.2} s, total \
             {:.2} s, hash {hash:016x}, {tris} tris, volume {volume:.6}, {bad} without a valid \
             source",
            i + r
        ));
    }
}

/// Plain 32-bit crossings of a cut, and those left after the rule at each factor of the
/// diagonal and then of the larger of the diagonal and the largest coordinate.
fn small_case(
    (a, b): (&Manifold, &Manifold),
    (at_a, at_b): (&Similarity, &Similarity),
    op: OpType,
    collapse: bool,
) -> Vec<usize> {
    let result = Manifold::from_impl(CsgNode::op(op, leaf(a, at_a), leaf(b, at_b)).evaluate());
    let mut counts = vec![{
        let p = export32(&result);
        crossing::crossings(&p.positions, &p.tris).len()
    }];
    let diag = diagonal(&result);
    let size = diag.max(result.bounding_box().scale());
    for size in [diag, size] {
        for k in FACTORS {
            let p = export32(&rule(&result, k * size, collapse));
            counts.push(crossing::crossings(&p.positions, &p.tris).len());
        }
    }
    counts
}

fn small(collapse: bool) {
    let ops = [
        ("subtract", OpType::Subtract),
        ("add", OpType::Add),
        ("intersect", OpType::Intersect),
    ];
    for n in [29, 41, 65, 91, 129] {
        let ball = shapes::sphere(n);
        let (a, b) = (import(&ball, 0), import(&ball, 1));
        for (name, op) in ops {
            let at = (&Similarity::IDENTITY, &shapes::pair_b());
            let c = small_case((&a, &b), at, op, collapse);
            say(&format!(
                "pair n {n} ({} tris) {name}: crossings plain, by diagonal, by size {c:?}",
                12 * n * n
            ));
        }
    }
    let ball = shapes::sphere(29);
    let (a, b) = (import(&ball, 0), import(&ball, 1));
    for offset in [0.0, 4.0, 16.0, 64.0, 256.0] {
        let shift = Vec3::new(offset, offset * 0.5, offset * 0.25);
        let at_a = Similarity::moving(shift);
        let mut at_b = shapes::pair_b();
        at_b.translation += shift;
        let c = small_case((&a, &b), (&at_a, &at_b), OpType::Subtract, collapse);
        say(&format!(
            "pair n 29 moved by {offset}: crossings plain, by diagonal, by size {c:?}"
        ));
    }
}
