//! Shared test oracle for gs-core: a mesh generator, a snapshot of everything the store shows
//! through its public views, an invariant checker that recomputes all derived data the slow
//! obvious way, a hasher, and brute-force references. It reads gs-core only through public
//! views and shares no code with it. Generic oracles (the mesh validator, the random generator,
//! fixtures, bounds) come from gs-testkit; what is here looks inside the store's nodes.

use std::collections::BTreeMap;

use gs_core::{
    Aabb, AttrVec, Kernel, MeshStore, NO, NodeConfig, NodeView, PosRead, SculptSurface, TopoRead,
    Vec3,
};
use gs_protocol::{AttrDomain, AttrValues, Attribute, PlainMesh};
use gs_testkit::rng::signed_unit;
use gs_testkit::{Expect, SplitMix64, fixtures, measure, validate};

// ---------------------------------------------------------------- generator

/// `unwrap` for test helpers (the lint allows it only inside `#[test]` functions).
pub fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    r.unwrap_or_else(|e| panic!("{e:?}"))
}

/// `unwrap` for test helpers, on an `Option`.
pub fn some<T>(o: Option<T>) -> T {
    o.unwrap_or_else(|| panic!("expected a value"))
}

/// A well-mixed 64-bit hash of a counter, from the testkit's generator.
pub fn mix(x: u64) -> u64 {
    SplitMix64::new(x).next_u64()
}

/// Uniform in [-1, 1).
pub fn unit(h: u64) -> f32 {
    signed_unit(h)
}

fn v3(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

/// The testkit's `n` by `n` cube pushed out to a lumpy sphere: vertices jittered sideways, a
/// fraction `tris` of the quads split into two triangles (1.0 gives a triangle mesh), and the
/// vertices stored in shuffled order. With `attrs`, it carries a vertex mask, face groups and
/// vertex colors.
pub fn sphere(n: u32, tris: f32, seed: u64, attrs: bool) -> PlainMesh {
    let cube = fixtures::cube(n);
    let pos: Vec<[f32; 3]> = (cube.positions.iter().enumerate())
        .map(|(i, &c)| {
            let d = v3(c).normalized();
            let h = mix(seed ^ i as u64);
            let jitter = Vec3::new(unit(h), unit(mix(h)), unit(mix(h ^ 7))) * (0.15 / n as f32);
            let bump = 1.0 + 0.08 * d.x * d.y + 0.05 * d.z * d.z * d.x;
            ((d + jitter).normalized() * bump).to_array()
        })
        .collect();
    let faces: Vec<[u32; 4]> = (cube.faces.iter().enumerate())
        .flat_map(|(k, &[a, b, c, d])| {
            let h = mix(seed.wrapping_add(k as u64 * 31));
            if (h % 1000) as f32 >= tris * 1000.0 {
                vec![[a, b, c, d]]
            } else if h & 1 == 0 {
                vec![[a, b, c, NO], [a, c, d, NO]]
            } else {
                vec![[a, b, d, NO], [b, c, d, NO]]
            }
        })
        .collect();
    shuffle(&pos, &faces, seed, attrs)
}

/// Store the vertices in a shuffled order, so the Morton sort has real work to do.
fn shuffle(pos: &[[f32; 3]], faces: &[[u32; 4]], seed: u64, attrs: bool) -> PlainMesh {
    let mut order: Vec<u32> = (0..pos.len() as u32).collect();
    SplitMix64::new(seed).shuffle(&mut order);
    let mut to_new = vec![0u32; pos.len()];
    for (new, &old) in order.iter().enumerate() {
        to_new[old as usize] = new as u32;
    }
    let positions: Vec<[f32; 3]> = order.iter().map(|&o| pos[o as usize]).collect();
    let faces: Vec<[u32; 4]> = (faces.iter())
        .map(|f| f.map(|v| if v == NO { NO } else { to_new[v as usize] }))
        .collect();
    let mut m = PlainMesh {
        positions,
        faces,
        ..PlainMesh::default()
    };
    if attrs {
        let nv = m.positions.len() as u64;
        // Settled, as a store holds a mask, so an import gives back the same bits.
        let mask = (0..nv)
            .map(|i| gs_core::mask::settle((unit(mix(i ^ seed)) + 1.0) * 0.4))
            .collect();
        let color = (0..nv).map(|i| mix(i + seed) as u32).collect();
        let group = (0..m.faces.len() as u64)
            .map(|f| (mix(f * 3 + seed) % 5) as u32)
            .collect();
        let at = |domain, values| Attribute { domain, values };
        m.attributes
            .insert("mask".into(), at(AttrDomain::Vertex, AttrValues::F32(mask)));
        m.attributes.insert(
            "color".into(),
            at(AttrDomain::Vertex, AttrValues::U32(color)),
        );
        m.attributes
            .insert("group".into(), at(AttrDomain::Face, AttrValues::U32(group)));
    }
    m
}

/// Small nodes, so a test mesh of a few thousand vertices has dozens of nodes and seams.
pub fn small_nodes() -> NodeConfig {
    ok(NodeConfig::new(7, 64))
}

pub fn store(m: &PlainMesh, cfg: NodeConfig) -> MeshStore {
    ok(MeshStore::from_plain(m, cfg))
}

// ---------------------------------------------------------------- a test kernel

/// Pull each vertex toward the mean of its neighbours' stroke-start positions, with a smooth
/// falloff. A trivial kernel that reads across nodes, which is what stresses the framework.
pub struct Relax {
    pub c: Vec3,
    pub r: f32,
    pub s: f32,
}

impl Kernel for Relax {
    fn spheres(&self) -> Vec<(Vec3, f32)> {
        vec![(self.c, self.r)]
    }
    fn deltas(&self, surface: &SculptSurface<'_>, node: &NodeView<'_>, out: &mut [Vec3]) {
        let (start, topo) = (surface.start_pos(), surface.topo());
        for (l, &p) in node.pos.iter().enumerate() {
            let d2 = (p - self.c).len2();
            if p.is_dead() || d2 >= self.r * self.r {
                continue;
            }
            let w = 1.0 - d2 / (self.r * self.r);
            let v = node.base + l as u32;
            let (mut acc, mut k) = (Vec3::ZERO, 0.0);
            for &f in topo.ring(v) {
                let fc = topo.face(f);
                for &u in fc.iter().filter(|&&u| u != NO && u != v) {
                    acc += start.get(u);
                    k += 1.0;
                }
            }
            out[l] = (acc * (1.0 / k) - p) * (w * w * self.s);
        }
    }
}

/// A point on the test sphere's surface in direction `d`, near enough for brushes.
pub fn surface_point(m: &MeshStore, d: Vec3) -> Vec3 {
    let d = d.normalized();
    let pos = m.pos();
    let mut best = (f32::INFINITY, Vec3::ZERO);
    for n in 0..m.node_count() as u32 {
        let view = m.node(n);
        for l in 0..view.pos.len() as u32 {
            let p = pos.get(view.base + l);
            let e = (p.normalized() - d).len2();
            if e < best.0 {
                best = (e, p);
            }
        }
    }
    best.1
}

// ---------------------------------------------------------------- snapshot

/// Everything one node shows through the public views, copied.
#[derive(Clone, Debug)]
pub struct NodeSnap {
    pub base: u32,
    pub first_face: u32,
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub faces: Vec<[u32; 4]>,
    pub rings: Vec<Vec<u32>>,
    pub seam: Vec<u32>,
    pub shared: Vec<u32>,
    pub bounds: Aabb,
    pub halo: Aabb,
    pub attrs: Vec<Option<AttrVec>>,
}

#[derive(Clone, Debug)]
pub struct Snap {
    pub cfg: NodeConfig,
    pub nodes: Vec<NodeSnap>,
}

pub fn snap(s: &MeshStore) -> Snap {
    let nodes = (0..s.node_count() as u32)
        .map(|n| {
            let v = s.node(n);
            NodeSnap {
                base: v.base,
                first_face: v.first_face,
                pos: v.pos.to_vec(),
                nrm: v.nrm.to_vec(),
                faces: v.faces.to_vec(),
                rings: (0..v.pos.len()).map(|l| v.ring(l).to_vec()).collect(),
                seam: v.seam.to_vec(),
                shared: s.shared(n).to_vec(),
                bounds: s.bounds(n),
                halo: s.halo(n),
                attrs: (0..s.attrs().len())
                    .map(|a| v.attr(some(s.attr_id(&s.attrs()[a].name))).cloned())
                    .collect(),
            }
        })
        .collect();
    Snap {
        cfg: s.config(),
        nodes,
    }
}

impl Snap {
    pub fn node_of(&self, v: u32) -> usize {
        (v >> self.cfg.shift()) as usize
    }
    pub fn pos(&self, v: u32) -> Vec3 {
        let n = &self.nodes[self.node_of(v)];
        n.pos[(v - n.base) as usize]
    }
    /// Local slot of `v` in its node, if the node exists and has that slot.
    fn slot(&self, v: u32) -> Option<usize> {
        let n = self.nodes.get(self.node_of(v))?;
        let l = v.checked_sub(n.base)? as usize;
        (l < n.pos.len()).then_some(l)
    }
    pub fn face_node(&self, f: u32) -> usize {
        (f >> self.cfg.fshift()) as usize
    }
    pub fn face(&self, f: u32) -> [u32; 4] {
        let n = &self.nodes[self.face_node(f)];
        n.faces[(f - n.first_face) as usize]
    }
    /// Every live face id with its corners, ascending by id.
    pub fn live_faces(&self) -> Vec<(u32, Vec<u32>)> {
        let mut out = Vec::new();
        for n in &self.nodes {
            for (j, f) in n.faces.iter().enumerate() {
                if f[0] != NO {
                    let corners = f.iter().copied().filter(|&v| v != NO).collect();
                    out.push((n.first_face + j as u32, corners));
                }
            }
        }
        out
    }
    /// Whether `v` names a live vertex slot.
    pub fn live(&self, v: u32) -> bool {
        self.slot(v)
            .is_some_and(|l| !self.nodes[self.node_of(v)].pos[l].x.is_nan())
    }
    pub fn live_vertices(&self) -> usize {
        self.nodes
            .iter()
            .flat_map(|n| &n.pos)
            .filter(|p| !p.x.is_nan())
            .count()
    }
}

// ---------------------------------------------------------------- checker

/// `a x b`, each component a 2 by 2 determinant of the other two axes, in `f32` like the store.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let det = |p: f32, q: f32, r: f32, t: f32| p * t - q * r;
    [
        det(a[1], a[2], b[1], b[2]),
        det(a[2], a[0], b[2], b[0]),
        det(a[0], a[1], b[0], b[1]),
    ]
}

fn sub(a: Vec3, b: Vec3) -> [f32; 3] {
    [a.x - b.x, a.y - b.y, a.z - b.z]
}

/// The definition of a vertex normal: the sum, in ascending face id order, of each face's
/// area-weighted normal (`(b-a)x(c-a)`, or the cross product of a quad's diagonals), then
/// scaled to unit length.
pub fn reference_normals(s: &Snap) -> BTreeMap<u32, [f32; 3]> {
    let mut acc: BTreeMap<u32, [f32; 3]> = BTreeMap::new();
    for (_, c) in s.live_faces() {
        let n = face_normal(s, &c);
        for &v in &c {
            let a = acc.entry(v).or_insert([0.0; 3]);
            *a = [a[0] + n[0], a[1] + n[1], a[2] + n[2]];
        }
    }
    for a in acc.values_mut() {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        *a = if l > 0.0 {
            a.map(|x| x * (1.0 / l))
        } else {
            [0.0; 3]
        };
    }
    acc
}

/// The area-weighted normal of the face with corners `c`.
fn face_normal(s: &Snap, c: &[u32]) -> [f32; 3] {
    let p: Vec<Vec3> = c.iter().map(|&v| s.pos(v)).collect();
    if c.len() == 3 {
        cross(sub(p[1], p[0]), sub(p[2], p[0]))
    } else {
        cross(sub(p[2], p[0]), sub(p[3], p[1]))
    }
}

/// How many live faces are turned and how many have zero area. A turned face points against
/// the sum of its corners' [`reference_normals`]: it lies folded over the surface around it. A
/// zero-area face has no normal, so it is never turned.
pub fn turned_and_flat(s: &Snap) -> (usize, usize) {
    let normals = reference_normals(s);
    let (mut turned, mut flat) = (0, 0);
    for (_, c) in s.live_faces() {
        let n = face_normal(s, &c);
        let around = (c.iter()).fold([0.0f32; 3], |a, v| {
            let m = normals[v];
            [a[0] + m[0], a[1] + m[1], a[2] + m[2]]
        });
        if n.iter().all(|&x| x == 0.0) {
            flat += 1;
        } else if n[0] * around[0] + n[1] * around[1] + n[2] * around[2] < 0.0 {
            turned += 1;
        }
    }
    (turned, flat)
}

fn same(a: Aabb, b: Aabb) -> bool {
    a.lo.bits() == b.lo.bits() && a.hi.bits() == b.hi.bits()
}

/// Every invariant of the store, recomputed from scratch: slot ownership and free slots, faces
/// on live vertices, rings equal to the incident faces in ascending order, seam and shared
/// lists, bounds and halo boxes, attribute lengths, and every normal bit for bit.
pub fn check(s: &Snap) -> Result<(), String> {
    let cap = s.cfg.capacity() as usize;
    for (k, n) in s.nodes.iter().enumerate() {
        if n.base != (k as u32) << s.cfg.shift() || n.pos.len() > cap {
            return Err(format!("node {k} owns ids outside its range"));
        }
        if n.nrm.len() != n.pos.len() || n.rings.len() != n.pos.len() {
            return Err(format!("node {k} columns differ in length"));
        }
    }
    let mut incident: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (f, c) in s.live_faces() {
        for &v in &c {
            if !s.live(v) {
                return Err(format!("face {f} uses missing or dead vertex {v}"));
            }
            incident.entry(v).or_default().push(f);
        }
    }
    let normals = reference_normals(s);
    for k in 0..s.nodes.len() {
        check_node(s, k, &incident, &normals)?;
    }
    Ok(())
}

fn check_node(
    s: &Snap,
    k: usize,
    incident: &BTreeMap<u32, Vec<u32>>,
    normals: &BTreeMap<u32, [f32; 3]>,
) -> Result<(), String> {
    let n = &s.nodes[k];
    let (mut own, mut near, mut seam) = (Vec::new(), Vec::new(), Vec::new());
    for (l, p) in n.pos.iter().enumerate() {
        let v = n.base + l as u32;
        let want: &[u32] = incident.get(&v).map_or(&[], |r| r);
        if n.rings[l] != want {
            return Err(format!(
                "ring of vertex {v} is {:?}, faces using it {want:?}",
                n.rings[l]
            ));
        }
        let nrm = normals.get(&v).copied().unwrap_or([0.0; 3]);
        if n.nrm[l].bits() != nrm.map(f32::to_bits) {
            return Err(format!("stale normal at vertex {v}"));
        }
        if p.x.is_nan() {
            continue;
        }
        own.push(p.to_array());
        let foreign = want.iter().any(|&f| {
            s.face_node(f) != k || s.face(f).iter().any(|&u| u != NO && s.node_of(u) != k)
        });
        if foreign {
            seam.push(l as u32);
            for &f in want {
                s.face(f)
                    .iter()
                    .filter(|&&u| u != NO)
                    .for_each(|&u| near.push(s.pos(u).to_array()));
            }
        }
    }
    let shared: std::collections::BTreeSet<u32> = (n.faces.iter().flatten())
        .copied()
        .filter(|&u| u != NO && s.node_of(u) != k)
        .collect();
    if n.seam != seam || n.shared != shared.iter().copied().collect::<Vec<_>>() {
        return Err(format!("node {k} seam or shared list is wrong"));
    }
    // Every face is stored in a node owning one of its corners, so the seam walk above finds
    // every corner of the node's faces.
    let homeless = (n.faces.iter()).find(|fc| fc[0] != NO && fc.iter().all(|&u| s.node_of(u) != k));
    if let Some(fc) = homeless {
        return Err(format!(
            "node {k} stores face {fc:?} but owns none of its corners"
        ));
    }
    let as_box = |b: Option<[[f32; 3]; 2]>| {
        b.map_or(Aabb::EMPTY, |[lo, hi]| Aabb {
            lo: v3(lo),
            hi: v3(hi),
        })
    };
    near.extend_from_slice(&own);
    let (bounds, halo) = (
        as_box(measure::bounds(&own)),
        as_box(measure::bounds(&near)),
    );
    if !same(n.bounds, bounds) || !same(n.halo, halo) {
        return Err(format!("node {k} bounds or halo box is wrong"));
    }
    for (a, col) in n.attrs.iter().enumerate() {
        let lens = [n.pos.len(), n.faces.len()];
        if col
            .as_ref()
            .is_some_and(|c| !lens.contains(&attr_bits(c).len()))
        {
            return Err(format!("node {k} attribute {a} has the wrong length"));
        }
    }
    Ok(())
}

/// The store's mesh, exported, passes the testkit validator as a closed manifold sphere
/// (Euler characteristic 2), which also rejects unused vertices, repeated corners and faces
/// wound against their neighbours. It has no zero-area check: zero-area faces are allowed
/// (review 0002, M2); [`turned_and_flat`] counts them, and faces turned over.
pub fn check_closed(s: &MeshStore) -> Result<(), String> {
    validate(&s.to_plain(), Expect::closed(2)).map_err(|d| format!("{d:?}"))
}

// ---------------------------------------------------------------- hasher

/// FNV-1a over the bits of everything in a snapshot.
pub fn fingerprint(s: &Snap) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut w = |x: u32| h = (h ^ u64::from(x)).wrapping_mul(0x0000_0100_0000_01b3);
    w(s.nodes.len() as u32);
    for n in &s.nodes {
        let boxes = [n.bounds.lo, n.bounds.hi, n.halo.lo, n.halo.hi];
        for p in n.pos.iter().chain(&n.nrm).chain(&boxes) {
            p.bits().into_iter().for_each(&mut w);
        }
        n.faces.iter().flatten().copied().for_each(&mut w);
        for list in n.rings.iter().chain([&n.seam, &n.shared]) {
            w(list.len() as u32);
            list.iter().copied().for_each(&mut w);
        }
        for a in &n.attrs {
            match a {
                None => w(u32::MAX),
                Some(a) => attr_bits(a).into_iter().for_each(&mut w),
            }
        }
    }
    h
}

/// Every value of an attribute column as raw bits.
pub fn attr_bits(a: &AttrVec) -> Vec<u32> {
    a.words()
}

// ---------------------------------------------------------------- references

/// Nodes whose bounds touch the sphere, and the other nodes whose halo does, by brute force.
pub fn brute_query(s: &Snap, c: Vec3, r: f32) -> (Vec<u32>, Vec<u32>) {
    let hits = |b: Aabb| {
        let d = |lo: f32, hi: f32, v: f32| (lo - v).max(v - hi).max(0.0);
        let (x, y, z) = (
            d(b.lo.x, b.hi.x, c.x),
            d(b.lo.y, b.hi.y, c.y),
            d(b.lo.z, b.hi.z, c.z),
        );
        x * x + y * y + z * z <= r * r
    };
    let mut brushed = Vec::new();
    let mut shell = Vec::new();
    for (k, n) in s.nodes.iter().enumerate() {
        if hits(n.bounds) {
            brushed.push(k as u32);
        } else if hits(n.halo) {
            shell.push(k as u32);
        }
    }
    (brushed, shell)
}

/// A plain mesh in a form that ignores vertex order: each face as the bits of its corner
/// positions and vertex attributes, rotated to start at the smallest corner, followed by its
/// face attributes; the faces sorted. Positions must be distinct.
pub fn canonical(m: &PlainMesh) -> Vec<Vec<u32>> {
    let vertex_attrs: Vec<&AttrValues> = (m.attributes.values())
        .filter(|a| a.domain == AttrDomain::Vertex)
        .map(|a| &a.values)
        .collect();
    let face_attrs: Vec<&AttrValues> = (m.attributes.values())
        .filter(|a| a.domain == AttrDomain::Face)
        .map(|a| &a.values)
        .collect();
    let bits = |v: &AttrValues, i: usize| match v {
        AttrValues::F32(x) => x[i].to_bits(),
        AttrValues::U32(x) => x[i],
    };
    let corner = |v: u32| {
        let mut c: Vec<u32> = m.positions[v as usize].map(f32::to_bits).to_vec();
        c.extend(vertex_attrs.iter().map(|a| bits(a, v as usize)));
        c
    };
    let mut out: Vec<Vec<u32>> = (m.faces.iter().enumerate())
        .map(|(i, f)| {
            let mut cs: Vec<Vec<u32>> =
                f.iter().filter(|&&v| v != NO).map(|&v| corner(v)).collect();
            let first = some((0..cs.len()).min_by(|&a, &b| cs[a].cmp(&cs[b])));
            cs.rotate_left(first);
            let mut row: Vec<u32> = cs.concat();
            row.extend(face_attrs.iter().map(|a| bits(a, i)));
            row
        })
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------- scripted edits

/// An open fan of `k` triangles around a center at the minimum corner, which sorts first.
pub fn fan(k: u32) -> PlainMesh {
    let mut positions = vec![[0.0, 0.0, 0.0]];
    positions.extend((0..=k).map(|i| [1.0 + i as f32, 1.0 + (i % 3) as f32, 1.0]));
    PlainMesh {
        positions,
        faces: (1..=k).map(|i| [0, i, i + 1, NO]).collect(),
        ..PlainMesh::default()
    }
}

/// A direction from a counter, for brush centers.
pub fn dir(i: u64) -> Vec3 {
    let h = mix(i * 7919 + 1);
    Vec3::new(unit(h), unit(mix(h)), unit(mix(h ^ 3))).normalized()
}

/// Split triangle edges longer than `len` with an end inside the sphere (`c`, `r`), longest
/// first, for `rounds` rounds. Returns how many nodes were cut in two.
pub fn refine(s: &mut MeshStore, c: Vec3, r: f32, len: f32, rounds: usize) -> usize {
    let mut node_splits = 0;
    for _ in 0..rounds {
        let sn = snap(s);
        let mut edges: Vec<(u32, u32, u32)> = Vec::new();
        for (_, cs) in sn.live_faces().into_iter().filter(|f| f.1.len() == 3) {
            for i in 0..3 {
                let (a, b) = (cs[i], cs[(i + 1) % 3]);
                let (pa, pb) = (sn.pos(a), sn.pos(b));
                let l2 = (pa - pb).len2();
                if a < b && (pa - c).len2() < r * r && l2 > len * len {
                    edges.push((u32::MAX - l2.to_bits(), a, b));
                }
            }
        }
        edges.sort_unstable();
        let mut ed = s.edit_topology();
        for &(_, a, b) in &edges {
            ed.split_edge(a, b);
        }
        node_splits += ed.node_splits();
    }
    node_splits
}

/// One stroke of `dabs` Relax dabs walking from direction `from`, as one history step.
pub fn stroke(s: &mut MeshStore, from: u64, dabs: u64, r: f32) {
    let mut sc = gs_core::Scratch::default();
    s.begin_step();
    for i in 0..dabs {
        let c = surface_point(s, dir(from) + dir(from + 1) * (i as f32 * 0.05));
        s.run_kernel(&Relax { c, r, s: 0.7 }, &mut sc);
    }
    s.end_step();
}
