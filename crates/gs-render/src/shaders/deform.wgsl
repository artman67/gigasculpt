// The open deformer's preview (decision 0041, amended 2026-10-12 for DF9a): while a deformer of
// DF2's closed forms is open, the part's buffers hold its points as they were at the open and
// the vertex stage moves each one by the operations in this block, at the point's own weight,
// and carries its normal by the map's derivative. Each operation is the twin of a gs-core
// function, named beside it; gs-render `tests/render/preview.rs` holds the two together.
// Amended 2026-10-13 for DF9b: noise (`shaders/deform_noise.wgsl`), the cage and the curve
// (`shaders/deform_tables.wgsl`, which read a table) too.
//
// Everything works as gs-core's does, in frame coordinates (the point's offset from the frame's
// origin along the frame's axes), but in f32: section 6 of the amendment says how close that
// lands. With no operation in the block every point and normal is drawn as stored, bit for bit.

// One operation about its frame, in the part's space.
struct DeformOp {
    // The frame's X, Y and Z. Their w: how far the frame's origin is from the part's (gs-core's
    // `far`), the amount or angle at full weight, and a second number: taper's exponent,
    // clip's depth or spherize's radius.
    axes: array<vec4<f32>, 3>,
    origin: vec4<f32>,
    // The box, in frame coordinates.
    lo: vec4<f32>,
    hi: vec4<f32>,
    // The operation (0 bend, 1 fold, 2 twist, 3 taper, 4 shear, 5 clip, 6 spherize, 7 noise,
    // 8 the cage, 9 the curve), its axis, the axis it moves toward, and flags: bits 0 to 2 the
    // frame axes kept, bit 3 a smooth shear. Noise holds its seed's low and high words in place
    // of the axes, its cell size as the second number; the cage holds its table's first entry
    // in place of `toward`, and the curve too, after its axis (`shaders/deform_tables.wgsl`).
    code: vec4<u32>,
};

// The chain, applied in order: `count.x` operations, none while no deformer is open.
struct Deforms {
    count: vec4<u32>,
    ops: array<DeformOp, 4>,
};

@group(3) @binding(0) var<uniform> deforms: Deforms;
// Each vertex slot's weight as unorm16, two to a word.
@group(3) @binding(1) var<storage, read> deform_weights: array<u32>;

const TAU: f32 = 6.28318530717959;

// A change of a point in frame coordinates, and its derivative: column j is the change's
// growth along frame axis j.
struct Change {
    d: vec3<f32>,
    j: mat3x3<f32>,
};

struct Moved {
    p: vec3<f32>,
    n: vec3<f32>,
};

// sin and cos of `t` taken back into [-pi, pi] first, where WGSL bounds their error.
fn turned_sin(t: f32) -> f32 {
    return sin(t - TAU * round(t / TAU));
}

fn turned_cos(t: f32) -> f32 {
    return cos(t - TAU * round(t / TAU));
}

// Twin of gs-core `deform/mod.rs` `sinc`: a series below half a radian, where sin(a) / a
// would multiply sin's error by the bend's radius.
fn sinc(a: f32) -> f32 {
    if abs(a) < 0.5 {
        let b = a * a;
        return 1.0 - b / 6.0 * (1.0 - b / 20.0 * (1.0 - b / 42.0 * (1.0 - b / 72.0)));
    }
    return turned_sin(a) / a;
}

// Twin of gs-core `deform/mod.rs` `Ready::place`, with its slope along the axis in `y`.
fn place(op: DeformOp, axis: u32, s: f32) -> vec2<f32> {
    let lo = op.lo[axis];
    let hi = op.hi[axis];
    if !(hi > lo) {
        return vec2<f32>(0.0);
    }
    let slope = select(0.0, 1.0 / (hi - lo), s > lo && s < hi);
    return vec2<f32>((clamp(s, lo, hi) - clamp(0.0, lo, hi)) / (hi - lo), slope);
}

// Twin of gs-core `deform/mod.rs` `Ready::near`.
fn near(op: DeformOp, c: vec3<f32>) -> f32 {
    return 1e-6 * (length(c) + op.axes[0].w);
}

// The frame axes the operation keeps, as 1 and 0.
fn kept(op: DeformOp) -> vec3<f32> {
    let bits = vec3<u32>(op.code.w, op.code.w >> 1u, op.code.w >> 2u) & vec3<u32>(1u);
    return vec3<f32>(bits);
}

// A turn by `t` in the plane of axes `a` and `b` as a change: its derivative less the identity.
fn turn_change(a: u32, b: u32, t: f32) -> mat3x3<f32> {
    var j = mat3x3<f32>();
    let sn = turned_sin(t);
    let cs = turned_cos(t);
    j[a][a] = cs - 1.0;
    j[a][b] = sn;
    j[b][a] = -sn;
    j[b][b] = cs - 1.0;
    return j;
}

// Twin of gs-core `deform/bend.rs` `bend`. Inside the box the derivative is the turn by `t`
// times diag(1 - k y, 1) in the (axis, toward) plane, beyond it the end's turn.
fn bend(op: DeformOp, c: vec3<f32>, turn: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let toward = op.code.z;
    let a = op.lo[axis];
    let b = op.hi[axis];
    if turn == 0.0 || !(b > a) {
        return out;
    }
    let s = c[axis];
    let inside = clamp(s, a, b);
    let x = inside - clamp(0.0, a, b);
    let e = s - inside;
    let y = c[toward];
    let k = turn / (b - a);
    let t = k * x;
    let sn = turned_sin(t);
    let half = turned_sin(t / 2.0);
    out.d[axis] = x * (sinc(t) - 1.0) - y * sn - 2.0 * e * half * half;
    out.d[toward] = x * half * sinc(t / 2.0) - 2.0 * y * half * half + e * sn;
    out.j = turn_change(axis, toward, t);
    let stretch = select(0.0, -k * y, s > a && s < b);
    out.j[axis][axis] += stretch * turned_cos(t);
    out.j[axis][toward] += stretch * sn;
    return out;
}

// Twin of gs-core `deform/bend.rs` `fold`: each side turned rigidly by half the angle.
fn fold(op: DeformOp, c: vec3<f32>, turn: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let toward = op.code.z;
    let s = c[axis];
    if turn == 0.0 || abs(s) <= near(op, c) {
        return out;
    }
    let phi = select(-turn, turn, s > 0.0) / 2.0;
    let sn = turned_sin(phi);
    let half = turned_sin(phi / 2.0);
    let y = c[toward];
    out.d[axis] = -2.0 * s * half * half - y * sn;
    out.d[toward] = s * sn - 2.0 * y * half * half;
    out.j = turn_change(axis, toward, phi);
    return out;
}

// Twin of gs-core `deform/twist.rs` `twist`: each slice turned; the turn's slope along the axis
// adds the turned offset, a quarter turn on, to the axis' column.
fn twist(op: DeformOp, c: vec3<f32>, turn: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let u = place(op, axis, c[axis]);
    let theta = turn * u.x;
    if theta == 0.0 {
        return out;
    }
    let p = (axis + 1u) % 3u;
    let q = (axis + 2u) % 3u;
    let sn = turned_sin(theta);
    let half = turned_sin(theta / 2.0);
    out.d[p] = -2.0 * c[p] * half * half - c[q] * sn;
    out.d[q] = c[p] * sn - 2.0 * c[q] * half * half;
    out.j = turn_change(p, q, theta);
    let slope = turn * u.y;
    out.j[axis][p] = -slope * (c[q] + out.d[q]);
    out.j[axis][q] = slope * (c[p] + out.d[p]);
    return out;
}

// Twin of gs-core `deform/taper.rs` `taper`: the other axes scaled by 1 - g.
fn taper(op: DeformOp, c: vec3<f32>, amount: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let exponent = op.axes[2].w;
    let u = place(op, axis, c[axis]);
    let g = amount * sign(u.x) * pow(abs(u.x), exponent);
    if g == 0.0 {
        return out;
    }
    let grow = amount * exponent * pow(abs(u.x), exponent - 1.0) * u.y;
    // An exponent below 1 is infinitely steep at the origin's slice: no slope there.
    let slope = select(0.0, grow, abs(grow) < 1e30);
    for (var k = 0u; k < 3u; k++) {
        if k != axis {
            out.d[k] = -g * c[k];
            out.j[k][k] = -g;
            out.j[axis][k] = -slope * c[k];
        }
    }
    return out;
}

// Twin of gs-core `deform/taper.rs` `shear`: each slice moved along `toward`.
fn shear(op: DeformOp, c: vec3<f32>, amount: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let toward = op.code.z;
    let eased = (op.code.w & 8u) != 0u;
    let u = place(op, axis, c[axis]);
    let a = abs(u.x);
    let g = select(u.x, sign(u.x) * a * a * (3.0 - 2.0 * a), eased);
    out.d[toward] = amount * g;
    out.j[axis][toward] = amount * select(1.0, 6.0 * a * (1.0 - a), eased) * u.y;
    return out;
}

// Twin of gs-core `deform/clip.rs` `clip`: the band past the plane squeezed onto it by `w`.
fn clip(op: DeformOp, c: vec3<f32>, w: f32) -> Change {
    var out: Change;
    let axis = op.code.y;
    let depth = op.axes[2].w;
    let a = op.lo[axis];
    let b = op.hi[axis];
    if depth == 0.0 || !(b >= a) {
        return out;
    }
    let s = c[axis];
    var change: f32;
    var band: bool;
    if depth > 0.0 {
        let plane = clamp(b - depth, a, b);
        change = -max(min(s, b) - plane, 0.0);
        band = s > plane && s < b;
    } else {
        let plane = clamp(a - depth, a, b);
        change = max(plane - max(s, a), 0.0);
        band = s < plane && s > a;
    }
    out.d[axis] = change * w;
    out.j[axis][axis] = select(0.0, -w, band);
    return out;
}

// Twin of gs-core `deform/clip.rs` `spherize`, over the kept axes: the change is f k, f =
// amount (r / L - 1), whose derivative is f I - (amount r / L^3) k k^T.
fn spherize(op: DeformOp, c: vec3<f32>, amount: f32) -> Change {
    var out: Change;
    let keep = kept(op);
    let k = c * keep;
    let l = length(k);
    let radius = op.axes[2].w;
    if amount == 0.0 || l <= 1e-6 * (radius + op.axes[0].w) {
        return out;
    }
    let f = amount * (radius / l - 1.0);
    let h = amount * radius / (l * l * l);
    out.d = f * k;
    out.j = mat3x3<f32>(f * keep.x * vec3<f32>(1.0, 0.0, 0.0) - h * k.x * k,
                        f * keep.y * vec3<f32>(0.0, 1.0, 0.0) - h * k.y * k,
                        f * keep.z * vec3<f32>(0.0, 0.0, 1.0) - h * k.z * k);
    return out;
}

// Twin of gs-core `deform/mod.rs` `Ready::change`, before the axes left out are taken away; no
// change for a code it does not know.
fn change(op: DeformOp, c: vec3<f32>, w: f32) -> Change {
    let amount = op.axes[1].w * w;
    var out: Change;
    switch op.code.x {
        case 0u: { out = bend(op, c, amount); }
        case 1u: { out = fold(op, c, amount); }
        case 2u: { out = twist(op, c, amount); }
        case 3u: { out = taper(op, c, amount); }
        case 4u: { out = shear(op, c, amount); }
        case 5u: { out = clip(op, c, w); }
        case 6u: { out = spherize(op, c, amount); }
        case 7u: { out = noise(op, c, amount); }
        case 8u: { out = lattice(op, c, w); }
        case 9u: { out = curve(op, c, w); }
        default: {}
    }
    return out;
}

// Twin of gs-core `transform/preview.rs` `Transform::deformed`, each operation of the chain in
// turn: point `p` with unit normal `n` at vertex slot `slot`, moved at the slot's weight. The
// normal goes by the cofactor of the map's derivative (the inverse transpose without the
// division by the determinant), so a fold past a half turn still faces out; one the map
// flattens to nothing keeps its direction.
fn deformed(p: vec3<f32>, n: vec3<f32>, slot: u32) -> Moved {
    var out = Moved(p, n);
    let count = min(deforms.count.x, 4u);
    if count == 0u {
        return out;
    }
    let w = unpack2x16unorm(deform_weights[slot / 2u])[slot % 2u];
    if w == 0.0 {
        return out;
    }
    for (var i = 0u; i < count; i++) {
        let op = deforms.ops[i];
        let frame = mat3x3<f32>(op.axes[0].xyz, op.axes[1].xyz, op.axes[2].xyz);
        let keep = kept(op);
        let moved = change(op, (out.p - op.origin.xyz) * frame, w);
        let d = moved.d * keep;
        if any(d != vec3<f32>(0.0)) {
            out.p = out.p + frame * d;
        }
        let f0 = vec3<f32>(1.0, 0.0, 0.0) + moved.j[0] * keep;
        let f1 = vec3<f32>(0.0, 1.0, 0.0) + moved.j[1] * keep;
        let f2 = vec3<f32>(0.0, 0.0, 1.0) + moved.j[2] * keep;
        let m = out.n * frame;
        let carried = frame * (m.x * cross(f1, f2) + m.y * cross(f2, f0) + m.z * cross(f0, f1));
        let size = length(carried);
        if size > 1e-12 {
            out.n = carried / size;
        }
    }
    return out;
}
