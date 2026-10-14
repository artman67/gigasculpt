// Noise on the vertex stage (decision 0041, amended 2026-10-13 for DF9b, section 1): gs-core's
// value noise with its hash to the bit. WGSL has no 64-bit integer, so a u64 is two words, the
// low one first; the add carries and the multiply keeps the low 64 bits, as Rust's wrapping ones.

// SplitMix64's step and its two multipliers, as gs-core `rng.rs` holds them.
const GAMMA: vec2<u32> = vec2<u32>(0x7F4A7C15u, 0x9E3779B9u);
const MIX_A: vec2<u32> = vec2<u32>(0x1CE4E5B9u, 0xBF58476Du);
const MIX_B: vec2<u32> = vec2<u32>(0x133111EBu, 0x94D049BBu);

fn add64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    return vec2<u32>(lo, a.y + b.y + select(0u, 1u, lo < a.x));
}

// The full 64-bit product of two words, from their 16-bit halves.
fn mul_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xFFFFu;
    let a1 = a >> 16u;
    let b0 = b & 0xFFFFu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let mid = (p00 >> 16u) + (p01 & 0xFFFFu) + (p10 & 0xFFFFu);
    let lo = (p00 & 0xFFFFu) | (mid << 16u);
    return vec2<u32>(lo, a1 * b1 + (p01 >> 16u) + (p10 >> 16u) + (mid >> 16u));
}

// The low 64 bits of `a b`.
fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let low = mul_wide(a.x, b.x);
    return vec2<u32>(low.x, low.y + a.x * b.y + a.y * b.x);
}

// `z ^ (z >> s)` for a shift of 1 to 31.
fn mix_shift(z: vec2<u32>, s: u32) -> vec2<u32> {
    return z ^ vec2<u32>((z.x >> s) | (z.y << (32u - s)), z.y >> s);
}

// Twin of gs-core `rng.rs` `CommandRng::u64_at`: value `index` of the stream seeded `seed`.
fn splitmix(seed: vec2<u32>, index: vec2<u32>) -> vec2<u32> {
    var z = add64(seed, mul64(add64(index, vec2<u32>(1u, 0u)), GAMMA));
    z = mul64(mix_shift(z, 30u), MIX_A);
    z = mul64(mix_shift(z, 27u), MIX_B);
    return mix_shift(z, 31u);
}

// A cell number as `i as u64` makes it: sign-extended.
fn cell64(i: i32) -> vec2<u32> {
    return vec2<u32>(bitcast<u32>(i), select(0u, 0xFFFFFFFFu, i < 0));
}

// Twin of gs-core `deform/noise.rs` `slice`, all three: bits 21 k to 21 k + 20 of `h` over 2^20,
// less 1.
fn slices(h: vec2<u32>) -> vec3<f32> {
    let mask = 0x1FFFFFu;
    let v = vec3<u32>(h.x & mask, ((h.x >> 21u) | (h.y << 11u)) & mask, (h.y >> 10u) & mask);
    return vec3<f32>(v) / 1048576.0 - 1.0;
}

// Twin of gs-core `deform/noise.rs` `noise`: the cell's 8 corners blended by the smoothstep,
// each corner's hash the seed taken through the cell numbers along x, y and z in turn, so the
// corners share their x and y rounds. The derivative along each axis is the smoothstep's slope.
fn noise(op: DeformOp, c: vec3<f32>, amount: f32) -> Change {
    var out: Change;
    let scale = op.axes[2].w;
    if amount == 0.0 || !(scale > 0.0) {
        return out;
    }
    let g = c / scale;
    let cell = floor(g);
    let f = g - cell;
    let t = f * f * (3.0 - 2.0 * f);
    let dt = 6.0 * f * (1.0 - f) / scale;
    let base = vec3<i32>(cell);
    let seed = vec2<u32>(op.code.y, op.code.z);
    for (var x = 0; x < 2; x++) {
        let hx = splitmix(seed, cell64(base.x + x));
        let wx = select(1.0 - t.x, t.x, x == 1);
        let sx = select(-dt.x, dt.x, x == 1);
        for (var y = 0; y < 2; y++) {
            let hy = splitmix(hx, cell64(base.y + y));
            let wy = select(1.0 - t.y, t.y, y == 1);
            let sy = select(-dt.y, dt.y, y == 1);
            for (var z = 0; z < 2; z++) {
                let v = slices(splitmix(hy, cell64(base.z + z)));
                let wz = select(1.0 - t.z, t.z, z == 1);
                let sz = select(-dt.z, dt.z, z == 1);
                out.d += wx * wy * wz * v;
                out.j[0] += sx * wy * wz * v;
                out.j[1] += wx * sy * wz * v;
                out.j[2] += wx * wy * sz * v;
            }
        }
    }
    out.d *= amount;
    out.j = out.j * amount;
    return out;
}
