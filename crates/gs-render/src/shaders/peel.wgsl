// Live Boolean's peel (peel.rs; decision 0057, section 9, as amended for LB8): a group's cut
// drawn from its operands. Each layer pass keeps, per pixel, the nearest operand fragment behind
// the layer before (`fs_layer`); a full-screen pass then sets or clears that operand's bit in the
// pixel's mask, folds the bits, and keeps the first layer the fold finds inside as the chosen
// one (`fs_resolve`). A last pass draws each operand again into the frame's own targets where
// it is the chosen fragment (`fs_peel`). Comes after geometry.wgsl, whose `vs`, `part`,
// `linear` and `surface` it uses.
//
// A peeled operand's `part.flags`: in x bit 7 set and bits 4 and 5 its operation (0 the base,
// 1 Add, 2 Subtract, 3 Intersect); in y its walls' color, sRGB bytes; in z its place in the
// group; in w its tie key, a signed whole number.

// One step of a tie key, as a fraction of the depth. The library breaks a tie by growing or
// shrinking an operand a hair; the key does the same in depth, with front faces nearer and back
// faces farther for a positive key. On lavapipe two meshes' flush faces seen at a slant keep
// their order at 2^-22 and lose it at 2^-24 (tests/render/peel.rs's flush pocket), so 2^-20.
const NUDGE: f32 = 1.0 / 1048576.0;
// The nearest depth a moved fragment may take, just short of the near plane.
const NEAREST: f32 = 0.99999994;
// How far, as a fraction of the depth, a fragment may be from the chosen depth and still be the
// chosen one: the same fragment rasterized by another pipeline.
const MATCH: f32 = 1.0 / 262144.0;

// The fragment's depth moved by its operand's tie key.
fn keyed(z: f32, front: bool) -> f32 {
    let e = f32(bitcast<i32>(part.flags.w)) * NUDGE;
    return min(z * (1.0 + select(-e, e, front)), NEAREST);
}

// What a layer keeps of a fragment: 0 for none, else twice the operand's place plus one, plus
// one on a front face.
fn hit(front: bool) -> u32 {
    return ((part.flags.z + 1u) << 1u) | u32(front);
}

@group(4) @binding(0) var layer_before: texture_depth_2d;

struct Layer {
    @location(0) hit: u32,
    @builtin(frag_depth) depth: f32,
};

// One layer: every fragment strictly behind the layer before; the depth test keeps the nearest.
@fragment fn fs_layer(i: VsOut, @builtin(front_facing) front: bool) -> Layer {
    let z = keyed(i.clip.z, front);
    if z >= textureLoad(layer_before, vec2<i32>(i.clip.xy), 0) {
        discard;
    }
    return Layer(hit(front), z);
}

// The group's fold: how many operands, a bit per Subtract, a bit per Intersect, and 1 when the
// base or an Add operand is the active part.
struct Fold {
    count: u32,
    differences: u32,
    intersects: u32,
    marked: u32,
};

@group(4) @binding(1) var layer_depth: texture_depth_2d;
@group(4) @binding(2) var layer_hit: texture_2d<u32>;
@group(4) @binding(3) var mask_before: texture_2d<u32>;
@group(4) @binding(4) var<uniform> cut: Fold;

// Whether a ray with operand bits `mask` is inside the cut: the bits folded left to right from
// outside, the base and each Add "or", each Subtract "and not", each Intersect "and".
fn inside(mask: u32) -> bool {
    var inside = false;
    for (var i = 0u; i < cut.count; i++) {
        let b = 1u << i;
        let bit = (mask & b) != 0u;
        if (cut.differences & b) != 0u {
            inside = inside && !bit;
        } else if (cut.intersects & b) != 0u {
            inside = inside && bit;
        } else {
            inside = inside || bit;
        }
    }
    return inside;
}

struct Resolved {
    @location(0) mask: u32,
    @location(1) hit: u32,
    @builtin(frag_depth) depth: f32,
};

// The mask after the layer: a front face sets its operand's bit, a back face clears it. Where
// the fold says inside, the layer's depth and hit go to the chosen targets, whose test ("greater
// or equal" from 0) keeps the first; elsewhere 0, which never replaces a chosen layer.
@fragment fn fs_resolve(@builtin(position) at: vec4<f32>) -> Resolved {
    let p = vec2<i32>(at.xy);
    let before = textureLoad(mask_before, p, 0).x;
    let hit = textureLoad(layer_hit, p, 0).x;
    if hit == 0u {
        return Resolved(before, 0u, 0.0);
    }
    let b = 1u << ((hit >> 1u) - 1u);
    let mask = select(before & ~b, before | b, (hit & 1u) == 1u);
    let kept = inside(mask);
    let depth = select(0.0, textureLoad(layer_depth, p, 0), kept);
    return Resolved(mask, select(0u, hit, kept), depth);
}

@group(4) @binding(5) var chosen_depth: texture_depth_2d;
@group(4) @binding(6) var chosen_hit: texture_2d<u32>;

// The last pass: the chosen fragment, shaded as the geometry pass shades a part, a Subtract's or
// Intersect's (only ever a wall) times its walls' color, with the group's mark as its flags.
@fragment fn fs_peel(i: VsOut, @builtin(front_facing) front: bool) -> Written {
    let p = vec2<i32>(i.clip.xy);
    let c = textureLoad(chosen_depth, p, 0);
    if hit(front) != textureLoad(chosen_hit, p, 0).x || abs(keyed(i.clip.z, front) - c) > c * MATCH {
        discard;
    }
    let wall = ((part.flags.x >> 4u) & 3u) >= 2u;
    let paint = select(i.paint, i.paint * linear(part.flags.y), wall);
    var written = surface(i.n_world, front, 1.0, i.mask, paint);
    written.parts = cut.marked;
    return written;
}
