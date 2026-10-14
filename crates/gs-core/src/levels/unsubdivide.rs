//! The level below level 0 (decision 0014, section 12): a port of Blender's unsubdivide
//! (`blenkernel/intern/multires_unsubdivide.cc`, GPL-2.0-or-later) onto canonical faces.
//!
//! Blender tags the points that would be the coarse faces' centres by a flood fill along quad
//! diagonals, checks the tagging, and dissolves the tagged points and their edge neighbours.
//! Here the coarse faces are built directly from the tagged centres, and one check covers what
//! the tagging rules miss: dividing the coarse faces by decision 0006's rule must give exactly
//! level 0's faces, each as the same cycle of points.

use std::collections::VecDeque;

use crate::config::MeshError;
use crate::math::{NO, arity};
use crate::subdivide::{Step, children};

/// The level below level 0, and how its division maps onto level 0.
pub(super) struct Below {
    /// The new base's faces, by new point index.
    pub faces: Vec<[u32; 4]>,
    /// Each new base point's level-0 index, ascending.
    pub corners: Vec<u32>,
    /// Each point of the new base divided once (new level 1): its level-0 index.
    pub points: Vec<u32>,
    /// Each face of new level 1: its level-0 face and the turn of its corners (new corner `i`
    /// is level-0 corner `(i + turn) % 4`).
    pub faces_up: Vec<(u32, u8)>,
}

/// Level 0 as faces around each point.
struct Mesh<'a> {
    faces: &'a [[u32; 4]],
    /// Per point, where its faces start in `ring` (one more at the end).
    start: Vec<u32>,
    /// Faces around each point, ascending.
    ring: Vec<u32>,
}

impl<'a> Mesh<'a> {
    fn new(faces: &'a [[u32; 4]], n: usize) -> Mesh<'a> {
        let mut start = vec![0u32; n + 1];
        for f in faces {
            for &v in f {
                start[v as usize + 1] += 1;
            }
        }
        for v in 1..=n {
            start[v] += start[v - 1];
        }
        let mut at = start.clone();
        let mut ring = vec![0u32; start[n] as usize];
        for (fi, f) in faces.iter().enumerate() {
            for &v in f {
                ring[at[v as usize] as usize] = fi as u32;
                at[v as usize] += 1;
            }
        }
        Mesh { faces, start, ring }
    }

    fn around(&self, v: u32) -> &[u32] {
        &self.ring[self.start[v as usize] as usize..self.start[v as usize + 1] as usize]
    }

    /// Face `f`'s corners turned so `v` is first.
    fn turned(&self, f: u32, v: u32) -> [u32; 4] {
        let mut q = self.faces[f as usize];
        let i = q.iter().position(|&x| x == v).unwrap_or(0);
        q.rotate_left(i);
        q
    }

    /// True if `u` and `v` share an edge (`BM_edge_exists`).
    fn edge(&self, u: u32, v: u32) -> bool {
        self.around(u).iter().any(|&f| {
            let q = self.turned(f, u);
            q[1] == v || q[3] == v
        })
    }

    /// The points across a quad diagonal from `v` (`is_vertex_diagonal`): in each face around
    /// `v`, the other corners not joined to `v` by an edge.
    fn diagonals(&self, v: u32, out: &mut Vec<u32>) {
        for &f in self.around(v) {
            for &u in &self.faces[f as usize] {
                if u != v && !self.edge(u, v) {
                    out.push(u);
                }
            }
        }
    }

    /// The ends of the edges at `v`, once per face using the edge, sorted.
    fn edge_ends(&self, v: u32) -> Vec<u32> {
        let mut ends: Vec<u32> = (self.around(v).iter())
            .flat_map(|&f| {
                let q = self.turned(f, v);
                [q[1], q[3]]
            })
            .collect();
        ends.sort_unstable();
        ends
    }

    /// The number of edges at `v` (`BM_vert_edge_count`) and whether one has a single face
    /// (`BM_vert_is_boundary`).
    fn valence(&self, v: u32) -> (usize, bool) {
        let ends = self.edge_ends(v);
        let (mut edges, mut boundary, mut i) = (0, false, 0);
        while i < ends.len() {
            let run = ends[i..].iter().take_while(|&&x| x == ends[i]).count();
            edges += 1;
            boundary |= run == 1;
            i += run;
        }
        (edges, boundary)
    }
}

fn bad(s: String) -> MeshError {
    MeshError::Input(s)
}

/// Pieces of level 0 joined by edges, numbered from the lowest point up
/// (`unsubdivide_init_elem_ids`): each point's piece, and each piece's points, ascending.
fn pieces(m: &Mesh) -> (Vec<u32>, Vec<Vec<u32>>) {
    let n = m.start.len() - 1;
    let mut piece = vec![NO; n];
    let mut points: Vec<Vec<u32>> = Vec::new();
    for v in 0..n as u32 {
        if piece[v as usize] != NO {
            continue;
        }
        let id = points.len() as u32;
        let mut members = vec![v];
        piece[v as usize] = id;
        let mut i = 0;
        while i < members.len() {
            for u in m.edge_ends(members[i]) {
                if piece[u as usize] == NO {
                    piece[u as usize] = id;
                    members.push(u);
                }
            }
            i += 1;
        }
        members.sort_unstable();
        points.push(members);
    }
    (piece, points)
}

/// Where a search for coarse corners starts in one piece (`unsubdivide_find_any_pole` and
/// `unsubdivide_tag_disconnected_mesh_element`): the first inner point of three edges, else the
/// last inner point of three or more than four, then the corners of the piece's first face.
fn starts(m: &Mesh, points: &[u32], boundary: &[bool]) -> Vec<u32> {
    let mut pole = None;
    for &v in points {
        let (edges, _) = m.valence(v);
        if boundary[v as usize] {
            continue;
        }
        if edges == 3 {
            pole = Some(v);
            break;
        }
        if edges > 4 {
            pole = Some(v);
        }
    }
    let first = points.iter().filter_map(|&v| m.around(v).first()).min();
    let mut out: Vec<u32> = pole.into_iter().collect();
    if let Some(&f) = first {
        out.extend_from_slice(&m.faces[f as usize]);
    }
    out
}

/// Tags the points two diagonal steps apart from the start's diagonal neighbours
/// (`unsubdivide_face_center_vertex_tag`): the coarse face centres, if the start is a corner.
fn flood(m: &Mesh, start: u32, tag: &mut [bool], seen: &mut [bool]) {
    let mut queue = VecDeque::new();
    let (mut first, mut second) = (Vec::new(), Vec::new());
    m.diagonals(start, &mut first);
    for &u in &first {
        queue.push_back(u);
        seen[u as usize] = true;
        tag[u as usize] = true;
    }
    while let Some(from) = queue.pop_front() {
        first.clear();
        m.diagonals(from, &mut first);
        for &d in &first {
            second.clear();
            m.diagonals(d, &mut second);
            for &u in &second {
                if !seen[u as usize] {
                    queue.push_back(u);
                    seen[u as usize] = true;
                    tag[u as usize] = true;
                }
            }
        }
    }
}

/// True if the tagging is a solution (`unsubdivide_is_center_vertex_tag_valid`): no tagged point
/// on a boundary or on a face with another tagged point, every boundary point on a face with a
/// tagged one.
fn valid(m: &Mesh, points: &[u32], tag: &[bool], boundary: &[bool]) -> bool {
    points.iter().all(|&v| {
        let near = (m.around(v).iter())
            .any(|&f| (m.faces[f as usize].iter()).any(|&u| u != v && tag[u as usize]));
        let b = boundary[v as usize];
        if tag[v as usize] && (b || near) {
            return false;
        }
        !b || near
    })
}

/// One coarse face, by level-0 indices: its corners, each corner's level-0 face, the edge
/// point after each corner, and its centre.
#[derive(Clone, Copy)]
struct Coarse {
    corners: [u32; 4],
    faces: [u32; 4],
    after: [u32; 4],
    centre: u32,
    n: usize,
}

/// The coarse face around centre `c`: the faces around it chained in their winding, each turned
/// to start at `c` as `(c, edge point before, corner, edge point after)`. `None` unless they
/// close into one fan of three or four.
fn coarse(m: &Mesh, c: u32) -> Option<Coarse> {
    let around = m.around(c);
    let n = around.len();
    if !(3..=4).contains(&n) {
        return None;
    }
    let mut out = Coarse {
        corners: [NO; 4],
        faces: [NO; 4],
        after: [NO; 4],
        centre: c,
        n,
    };
    let mut used = [false; 4];
    let mut at = 0;
    for k in 0..n {
        if k > 0 {
            let prev = out.after[k - 1];
            at = (0..n).find(|&i| !used[i] && m.turned(around[i], c)[1] == prev)?;
        }
        used[at] = true;
        let q = m.turned(around[at], c);
        (out.corners[k], out.faces[k], out.after[k]) = (q[2], around[at], q[3]);
    }
    (m.turned(around[at], c)[3] == m.turned(out.faces[0], c)[1]).then_some(out)
}

/// The coarse faces of every piece, from its tagging. Fails naming the first piece that has no
/// solution, or a face-group border through a coarse face.
fn solve(m: &Mesh, groups: Option<&[u32]>) -> Result<Vec<Coarse>, MeshError> {
    let n = m.start.len() - 1;
    let (piece, members) = pieces(m);
    let boundary: Vec<bool> = gs_par::map(n, |v| m.valence(v as u32).1);
    let (mut tag, mut seen) = (vec![false; n], vec![false; n]);
    let mut out = Vec::new();
    for (p, points) in members.iter().enumerate() {
        let name = || {
            let first = points.iter().filter_map(|&v| m.around(v).first()).min();
            let faces = (0..m.faces.len()).filter(|&f| piece[m.faces[f][0] as usize] == p as u32);
            format!(
                "piece {} (from face {}, point {}, {} faces)",
                p + 1,
                first.copied().unwrap_or(NO),
                points[0],
                faces.count()
            )
        };
        let mut found = false;
        for s in starts(m, points, &boundary) {
            flood(m, s, &mut tag, &mut seen);
            found = valid(m, points, &tag, &boundary);
            for &v in points {
                seen[v as usize] = false;
                tag[v as usize] &= found;
            }
            if found {
                break;
            }
        }
        if !found {
            return Err(bad(format!(
                "{}: no choice of coarse corners passes the tagging check",
                name()
            )));
        }
        for &c in points.iter().filter(|&&v| tag[v as usize]) {
            let Some(f) = coarse(m, c) else {
                return Err(bad(format!(
                    "{}: the faces around point {c} do not make a coarse quad or triangle",
                    name()
                )));
            };
            let split = groups.is_some_and(|g| {
                (1..f.n).any(|k| g[f.faces[k] as usize] != g[f.faces[0] as usize])
            });
            if split {
                return Err(bad(format!(
                    "{}: a face-group border runs through the coarse face around point {c}",
                    name()
                )));
            }
            out.push(f);
        }
    }
    Ok(out)
}

/// The level below level 0 (`faces`, all quads, over `n` points), with `groups` the face group
/// each level-0 face shows, if the part has groups. See the module docs.
pub(super) fn below(
    faces: &[[u32; 4]],
    n: usize,
    groups: Option<&[u32]>,
) -> Result<Below, MeshError> {
    if faces.len() < 3 {
        return Err(bad("level 0 has fewer than 3 faces".into()));
    }
    if let Some(f) = faces.iter().position(|&f| arity(f) != 4) {
        return Err(bad(format!(
            "level 0 has a triangle (face {f}); reconstruct needs all quads"
        )));
    }
    let m = Mesh::new(faces, n);
    if let Some(v) = (0..n as u32).find(|&v| m.around(v).is_empty()) {
        return Err(bad(format!("point {v} of level 0 has no face")));
    }
    let mut coarse = solve(&m, groups)?;
    let mut corners: Vec<u32> = coarse
        .iter()
        .flat_map(|c| c.corners[..c.n].to_vec())
        .collect();
    corners.sort_unstable();
    corners.dedup();
    let new_id = |v: u32| corners.binary_search(&v).map_or(NO, |i| i as u32);
    // Each coarse face starts at the corner of its lowest level-0 child, and the faces go in
    // the order of that child. Decision 0006 numbers child k of face p as 4p + k, holding
    // corner k, so a level-0 that came from dividing gets back the faces, corners and turns it
    // came from, and every level keeps its numbering.
    for c in &mut coarse {
        let turn = (0..c.n).min_by_key(|&k| c.faces[k]).unwrap_or(0);
        c.corners[..c.n].rotate_left(turn);
        c.faces[..c.n].rotate_left(turn);
        c.after[..c.n].rotate_left(turn);
    }
    coarse.sort_unstable_by_key(|c| c.faces[..c.n].iter().copied().min());
    let base: Vec<[u32; 4]> = (coarse.iter())
        .map(|c| c.corners.map(|v| if v == NO { NO } else { new_id(v) }))
        .collect();
    map_up(faces, n, &coarse, base, corners)
}

/// Divides the coarse faces once and maps the result onto level 0, checking it is level 0.
fn map_up(
    faces: &[[u32; 4]],
    n: usize,
    coarse: &[Coarse],
    base: Vec<[u32; 4]>,
    corners: Vec<u32>,
) -> Result<Below, MeshError> {
    let differs = || bad("the coarse faces do not divide back into level 0".into());
    let step = Step::new(&base, corners.len() as u32).ok_or_else(differs)?;
    if step.count as usize != n {
        return Err(differs());
    }
    let mut points = vec![NO; n];
    points[..corners.len()].copy_from_slice(&corners);
    let mut put = |at: u32, v: u32| match points[at as usize] {
        NO => {
            points[at as usize] = v;
            true
        }
        held => held == v,
    };
    for (fi, c) in coarse.iter().enumerate() {
        let edges = step.edges(fi);
        let ok = (0..c.n).all(|k| put(edges[k], c.after[k])) && put(step.center[fi], c.centre);
        if !ok {
            return Err(differs());
        }
    }
    let mut used = vec![false; faces.len()];
    let mut faces_up = Vec::with_capacity(faces.len());
    for (fi, c) in coarse.iter().enumerate() {
        let (kids, k_n) = children(base[fi], step.edges(fi), step.center[fi]);
        for (kid, &g) in kids[..k_n].iter().zip(&c.faces) {
            let mapped = kid.map(|v| points[v as usize]);
            let old = faces[g as usize];
            let turn = (0..4).find(|&r| (0..4).all(|i| mapped[i] == old[(i + r) % 4]));
            match turn {
                Some(r) if !std::mem::replace(&mut used[g as usize], true) => {
                    faces_up.push((g, r as u8));
                }
                _ => return Err(differs()),
            }
        }
    }
    if faces_up.len() != faces.len() {
        return Err(differs());
    }
    Ok(Below {
        faces: base,
        corners,
        points,
        faces_up,
    })
}
