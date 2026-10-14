// The cage and the curve on the vertex stage (decision 0041, amended 2026-10-13 for DF9b,
// sections 2 to 4). Each reads its table from the operation's first entry (the third word of its
// code): gs-core's own grid or arcs in f32, written by `lattice::table` or `curve::table` and
// sent only when it changes.

@group(3) @binding(2) var<storage, read> deform_tables: array<vec4<f32>>;

// Twin of gs-core `deform/lattice.rs` `spline`: the uniform cubic B-spline's weights of a cell's
// four points at place `t` in it.
fn spline(t: f32) -> vec4<f32> {
    let s = 1.0 - t;
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(s * s * s, 3.0 * t3 - 6.0 * t2 + 4.0, -3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0,
                     t3) / 6.0;
}

// The slopes of `spline`'s weights.
fn spline_slope(t: f32) -> vec4<f32> {
    let s = 1.0 - t;
    return vec4<f32>(-s * s, 3.0 * t * t - 4.0 * t, -3.0 * t * t + 2.0 * t + 1.0, t * t) / 2.0;
}

// Twin of gs-core `deform/lattice.rs` `lattice`: the offsets around the point, each weighted by
// its place in its cell along each axis, summed, then times `w`. Along each axis the derivative
// is the weights' slopes times the cells a unit inside the box; beyond it the place is held.
fn lattice(op: DeformOp, c: vec3<f32>, w: f32) -> Change {
    var out: Change;
    let at = op.code.z;
    let head = deform_tables[at];
    let n = vec3<u32>(head.xyz);
    let soft = head.w != 0.0;
    if n.x < 2u || n.y < 2u || n.z < 2u {
        return out;
    }
    var first: vec3<u32>;
    var weights: array<vec4<f32>, 3>;
    var slopes: array<vec4<f32>, 3>;
    for (var k = 0u; k < 3u; k++) {
        let lo = op.lo[k];
        let hi = op.hi[k];
        let cells = f32(n[k] - 1u);
        var f = 0.0;
        var slope = 0.0;
        if hi > lo {
            f = (clamp(c[k], lo, hi) - lo) / (hi - lo) * cells;
            slope = select(0.0, cells / (hi - lo), c[k] > lo && c[k] < hi);
        }
        let cell = min(u32(f), n[k] - 2u);
        let t = f - f32(cell);
        // The padded index of the cell's first point: one before the cell when soft.
        first[k] = select(cell + 1u, cell, soft);
        weights[k] = select(vec4<f32>(1.0 - t, t, 0.0, 0.0), spline(t), soft);
        slopes[k] = slope * select(vec4<f32>(-1.0, 1.0, 0.0, 0.0), spline_slope(t), soft);
    }
    let reach = select(2u, 4u, soft);
    let row = n.x + 2u;
    let plane = row * (n.y + 2u);
    for (var z = 0u; z < reach; z++) {
        for (var y = 0u; y < reach; y++) {
            for (var x = 0u; x < reach; x++) {
                let p = first + vec3<u32>(x, y, z);
                let by = deform_tables[at + 1u + p.x + row * p.y + plane * p.z].xyz;
                let wx = weights[0][x];
                let wy = weights[1][y];
                let wz = weights[2][z];
                out.d += wx * wy * wz * by;
                out.j[0] += slopes[0][x] * wy * wz * by;
                out.j[1] += wx * slopes[1][y] * wz * by;
                out.j[2] += wx * wy * slopes[2][z] * by;
            }
        }
    }
    out.d *= w;
    out.j = out.j * w;
    return out;
}

// A point of the curve: where it is, the frame riding there (its columns the frame's axes in
// frame coordinates), and how fast that frame turns per unit of distance, in its own axes.
struct Riding {
    pos: vec3<f32>,
    frame: mat3x3<f32>,
    spin: vec3<f32>,
};

// sin(h) / h and cos(h) by their series to the 14th power: within f32 rounding up to a quarter
// turn, where WGSL's `sin` is bounded only to 2^-11 and a walk would add that up.
fn half_sinc(h: f32) -> f32 {
    let b = h * h;
    return 1.0 - b / 6.0 * (1.0 - b / 20.0 * (1.0 - b / 42.0 * (1.0 - b / 72.0
        * (1.0 - b / 110.0 * (1.0 - b / 156.0 * (1.0 - b / 210.0))))));
}

fn half_cos(h: f32) -> f32 {
    let b = h * h;
    return 1.0 - b / 2.0 * (1.0 - b / 12.0 * (1.0 - b / 30.0 * (1.0 - b / 56.0
        * (1.0 - b / 90.0 * (1.0 - b / 132.0 * (1.0 - b / 182.0))))));
}

fn curve_axis(axis: u32) -> vec3<f32> {
    var e = vec3<f32>(0.0);
    e[axis] = 1.0;
    return e;
}

// Twin of gs-core `deform/curve.rs` `turned`: the turn about unit `n` whose angle has sine `sn`
// and 1 - cos of `versed`, I + sn K + versed K², K the cross product with `n`.
fn turned(n: vec3<f32>, sn: f32, versed: f32) -> mat3x3<f32> {
    let k = mat3x3<f32>(vec3<f32>(0.0, n.z, -n.y), vec3<f32>(-n.z, 0.0, n.x),
                        vec3<f32>(n.y, -n.x, 0.0));
    let one = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0),
                          vec3<f32>(0.0, 0.0, 1.0));
    return one + k * sn + (k * k) * versed;
}

// Record `rec`'s point and frame: where its arc starts at full weight, or where the last ends.
fn curve_record(rec: u32) -> Riding {
    var out: Riding;
    out.pos = deform_tables[rec + 1u].xyz;
    out.frame = transpose(mat3x3<f32>(deform_tables[rec + 2u].xyz, deform_tables[rec + 3u].xyz,
                                      deform_tables[rec + 4u].xyz));
    return out;
}

// The riding frame `dx` straight on along its tangent, where it does not turn.
fn curve_on(at: Riding, axis: u32, dx: f32) -> Riding {
    var out = at;
    out.pos = at.pos + out.frame[axis] * dx;
    out.spin = vec3<f32>(0.0);
    return out;
}

// Twin of gs-core `deform/curve.rs` `Path::origin`: the curve's start at weight `w`, from the
// box's low face on the axis line, with the frame turned by `w` of the least turn.
fn curve_origin(base: u32, axis: u32, w: f32) -> Riding {
    let start = deform_tables[base + 1u];
    let h = w * start.w / 2.0;
    let sn = h * half_sinc(h);
    let cs = half_cos(h);
    var out: Riding;
    out.pos = curve_axis(axis) * deform_tables[base].x + start.xyz * w;
    out.frame = turned(deform_tables[base + 2u].xyz, 2.0 * sn * cs, 2.0 * sn * sn);
    return out;
}

// Twin of gs-core `deform/curve.rs` `Path::step`: from `at`, `sigma` along the arc of record
// `rec` turning `w` of its angle. The chord in the frame is the start tangent turned by half the
// angle so far; the frame spins by the arc's turn per unit of its length.
fn curve_step(at: Riding, rec: u32, axis: u32, w: f32, sigma: f32) -> Riding {
    let arc = deform_tables[rec];
    let span = deform_tables[rec + 1u].w;
    let half = w * arc.w * (sigma / span) / 2.0;
    let ratio = half_sinc(half);
    let sn = half * ratio;
    let cs = half_cos(half);
    let e = curve_axis(axis);
    let chord = sigma * ratio * (cs * e + sn * cross(arc.xyz, e));
    var out: Riding;
    out.pos = at.pos + at.frame * chord;
    out.frame = at.frame * turned(arc.xyz, 2.0 * sn * cs, 2.0 * sn * sn);
    out.spin = w * arc.w / span * arc.xyz;
    return out;
}

// Twin of gs-core `deform/curve.rs` `Path::locate`: the point at distance `x` along the curve at
// weight `w` and the riding frame there; before the start straight back along the start tangent,
// past the end straight on. At full weight a search of the arcs' starts and one step from the
// table; below it the walk from the start turned by `w`, every arc turned by `w`.
fn curve_locate(base: u32, axis: u32, x: f32, w: f32) -> Riding {
    let arcs = u32(deform_tables[base].y);
    let first = base + 3u;
    let last = first + 5u * arcs;
    if w > 0.99999 {
        // How many of the starts, the end's included, are at or before `x`.
        var lo = 0u;
        var hi = arcs + 1u;
        while lo < hi {
            let mid = (lo + hi) / 2u;
            if deform_tables[first + 5u * mid + 2u].w <= x {
                lo = mid + 1u;
            } else {
                hi = mid;
            }
        }
        if lo == 0u {
            return curve_on(curve_record(first), axis, x);
        }
        if lo > arcs {
            return curve_on(curve_record(last), axis, x - deform_tables[last + 2u].w);
        }
        let rec = first + 5u * (lo - 1u);
        return curve_step(curve_record(rec), rec, axis, 1.0, x - deform_tables[rec + 2u].w);
    }
    var at = curve_origin(base, axis, w);
    if x < 0.0 {
        return curve_on(at, axis, x);
    }
    for (var rec = first; rec < last; rec += 5u) {
        let begin = deform_tables[rec + 2u].w;
        if x < deform_tables[rec + 7u].w {
            return curve_step(at, rec, axis, w, x - begin);
        }
        at = curve_step(at, rec, axis, w, deform_tables[rec + 1u].w);
    }
    return curve_on(at, axis, x - deform_tables[last + 2u].w);
}

// The twist (`x`) and the scale (`y`) at a distance along the curve, and their slopes.
struct Along {
    value: vec2<f32>,
    slope: vec2<f32>,
};

// Twin of gs-core `deform/curve.rs` `along`, the twist and the scale at once, from the `knots`
// entries at `at`: linear between the points' distances and held past the ends.
fn curve_along(at: u32, knots: u32, x: f32) -> Along {
    var i = 0u;
    while i < knots && deform_tables[at + i].x <= x {
        i++;
    }
    var out: Along;
    if i == 0u || i == knots {
        out.value = deform_tables[at + max(i, 1u) - 1u].yz;
        return out;
    }
    let a = deform_tables[at + i - 1u];
    let b = deform_tables[at + i];
    let t = (x - a.x) / (b.x - a.x);
    out.value = a.yz + t * (b.yz - a.yz);
    out.slope = (b.yz - a.yz) / (b.x - a.x);
    return out;
}

// Twin of gs-core `deform/curve.rs` `shaped`: the change of cross-section point `b` turned by
// `twist` about `axis` and scaled by `scale`. Across the axis its derivative is the turn times
// the scale less the identity; along it, where the twist and scale have `slopes`, the turned
// point swings by the twist's and grows by the scale's.
fn shaped(b: vec3<f32>, axis: u32, twist: f32, scale: f32, slopes: vec2<f32>) -> Change {
    var out: Change;
    let p = (axis + 1u) % 3u;
    let q = (axis + 2u) % 3u;
    let sn = turned_sin(twist);
    let half = turned_sin(twist / 2.0);
    let versed = 2.0 * half * half;
    let turn_p = -b[p] * versed - b[q] * sn;
    let turn_q = b[p] * sn - b[q] * versed;
    out.d[p] = scale * turn_p + (scale - 1.0) * b[p];
    out.d[q] = scale * turn_q + (scale - 1.0) * b[q];
    out.j[p][p] = scale * (1.0 - versed) - 1.0;
    out.j[p][q] = scale * sn;
    out.j[q][p] = -scale * sn;
    out.j[q][q] = scale * (1.0 - versed) - 1.0;
    let rp = b[p] + turn_p;
    let rq = b[q] + turn_q;
    out.j[axis][p] = slopes.y * rp - scale * slopes.x * rq;
    out.j[axis][q] = slopes.y * rq + scale * slopes.x * rp;
    return out;
}

// Twin of gs-core `deform/curve.rs` `curve`: the cross-section turned by `w` of the twist and
// scaled by 1 + w (scale - 1), carried by the riding frame to the curve's point at the part's
// distance along it. Each column of the derivative is the frame times the section's own, less
// the identity's; along the axis the frame's tangent and its spin across the carried section
// add to it.
fn curve(op: DeformOp, c: vec3<f32>, w: f32) -> Change {
    let base = op.code.z;
    let head = deform_tables[base];
    let knots = u32(head.z);
    if knots == 0u || w == 0.0 {
        return Change();
    }
    let arcs = u32(head.y);
    let axis = op.code.y;
    let x = c[axis] - head.x;
    let records = select(0u, arcs + 1u, arcs > 0u);
    let along = curve_along(base + 3u + 5u * records, knots, x);
    var section = c;
    section[axis] = 0.0;
    let twist = w * along.value.x;
    let scale = 1.0 + w * (along.value.y - 1.0);
    var shape = shaped(section, axis, twist, scale, w * along.slope);
    if arcs == 0u {
        return shape;
    }
    let at = curve_locate(base, axis, x, w);
    let carried = section + shape.d;
    var out: Change;
    out.d = at.pos + at.frame * carried - c;
    for (var k = 0u; k < 3u; k++) {
        let e = curve_axis(k);
        let spun = select(vec3<f32>(0.0), cross(at.spin, carried), k == axis);
        out.j[k] = at.frame * (e + shape.j[k] + spun) - e;
    }
    return out;
}
