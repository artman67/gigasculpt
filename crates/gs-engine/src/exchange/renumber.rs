//! A top that came back in Blender's numbering, matched to the part's own (decision 0032,
//! section 2). Blender's Multires top has the part's top faces, but Blender numbers a level's
//! points and faces its own way, and a base face can start at another corner. Both programs
//! keep the base's points first and in order, so those anchor the match: from a face at a base
//! point, faces are paired across shared edges until every face has its partner, trying each
//! face of the part at that point in turn. Every pair is checked as it is made (each point and
//! face taken once, the base points kept, corners agreeing), so a mesh that matches has the
//! part's faces exactly.

use std::collections::VecDeque;

use gs_protocol::{PlainMesh, arity};

const NONE: u32 = u32::MAX;

/// One mesh's faces with every directed edge, to find the face across an edge.
struct Sides<'a> {
    faces: &'a [[u32; 4]],
    /// `((from, to), face, corner)` for every edge of every face, sorted.
    edges: Vec<((u32, u32), u32, u8)>,
}

impl<'a> Sides<'a> {
    fn of(faces: &'a [[u32; 4]]) -> Sides<'a> {
        let mut edges = Vec::with_capacity(faces.len() * 4);
        for (face, f) in faces.iter().zip(0u32..) {
            let n = arity(*face);
            let ends = (0..n).map(|c| (face[c], face[(c + 1) % n]));
            edges.extend(ends.zip(0u8..).map(|(e, c)| (e, f, c)));
        }
        edges.sort_unstable();
        Sides { faces, edges }
    }

    /// The face and corner whose edge runs from `a` to `b`.
    fn edge(&self, a: u32, b: u32) -> Option<(u32, usize)> {
        let k = self.edges.binary_search_by_key(&(a, b), |e| e.0).ok()?;
        Some((self.edges[k].1, usize::from(self.edges[k].2)))
    }

    /// Every face and corner at point `p`.
    fn at(&self, p: u32) -> impl Iterator<Item = (u32, usize)> {
        let from = self.edges.partition_point(|e| e.0.0 < p);
        let to = self.edges.partition_point(|e| e.0.0 <= p);
        (self.edges[from..to].iter()).map(|e| (e.1, usize::from(e.2)))
    }
}

/// The pairs made so far, both ways, and the base points that must keep their numbers.
struct Pairs {
    point: Vec<u32>,
    point_back: Vec<u32>,
    face: Vec<u32>,
    face_back: Vec<u32>,
    fixed: u32,
}

impl Pairs {
    /// Pairs `came`'s point `p` with `part`'s point `q`, or says they cannot be.
    fn points(&mut self, p: u32, q: u32, made: &mut Vec<u32>) -> bool {
        if self.point[p as usize] == q {
            return true;
        }
        let free = self.point[p as usize] == NONE && self.point_back[q as usize] == NONE;
        if !free || ((p < self.fixed || q < self.fixed) && p != q) {
            return false;
        }
        self.point[p as usize] = q;
        self.point_back[q as usize] = p;
        made.push(p);
        true
    }

    /// Pairs `came`'s face `f` (its corner `i`) with `part`'s face `g` (corner `j`), then the
    /// faces across their edges, outwards. On a mismatch every pair it made is taken back.
    fn grow(&mut self, came: &Sides, part: &Sides, start: (u32, usize, u32, usize)) -> bool {
        let (mut points, mut faces) = (Vec::new(), Vec::new());
        let mut todo = VecDeque::from([start]);
        let ok = 'grow: loop {
            let Some((f, i, g, j)) = todo.pop_front() else {
                break true;
            };
            let (a, b) = (came.faces[f as usize], part.faces[g as usize]);
            let n = arity(a);
            if self.face[f as usize] == g {
                continue;
            }
            let free = self.face[f as usize] == NONE && self.face_back[g as usize] == NONE;
            if n != arity(b) || !free {
                break false;
            }
            self.face[f as usize] = g;
            self.face_back[g as usize] = f;
            faces.push(f);
            for c in 0..n {
                let d = (c + j + n - i) % n;
                if !self.points(a[c], b[d], &mut points) {
                    break 'grow false;
                }
                let across = (
                    came.edge(a[(c + 1) % n], a[c]),
                    part.edge(b[(d + 1) % n], b[d]),
                );
                match across {
                    (Some((f2, i2)), Some((g2, j2))) => todo.push_back((f2, i2, g2, j2)),
                    (None, None) => {}
                    _ => break 'grow false,
                }
            }
        };
        if !ok {
            for p in points {
                self.point_back[self.point[p as usize] as usize] = NONE;
                self.point[p as usize] = NONE;
            }
            for f in faces {
                self.face_back[self.face[f as usize] as usize] = NONE;
                self.face[f as usize] = NONE;
            }
        }
        ok
    }
}

/// For each point of `came`, its number in `part`: found when `came` has `part`'s faces up to
/// the order of its points, faces and corners, with its first `fixed` points (the base's) the
/// same as `part`'s. `None` when it does not, or when a point is in no face.
pub fn renumbering(came: &PlainMesh, part: &PlainMesh, fixed: u32) -> Option<Vec<u32>> {
    let (points, faces) = (part.positions.len(), part.faces.len());
    let counts = u32::try_from(points).ok().zip(u32::try_from(faces).ok());
    let same = came.positions.len() == points && came.faces.len() == faces;
    if counts.is_none() || !same || fixed == 0 {
        return None;
    }
    let (a, b) = (Sides::of(&came.faces), Sides::of(&part.faces));
    let mut pairs = Pairs {
        point: vec![NONE; points],
        point_back: vec![NONE; points],
        face: vec![NONE; faces],
        face_back: vec![NONE; faces],
        fixed,
    };
    for anchor in (0..fixed).take(points) {
        let Some((f, i)) = a.at(anchor).next() else {
            continue;
        };
        if pairs.point[anchor as usize] == NONE
            && !(b.at(anchor)).any(|(g, j)| pairs.grow(&a, &b, (f, i, g, j)))
        {
            return None;
        }
    }
    let whole = pairs.face.iter().all(|&g| g != NONE) && pairs.point.iter().all(|&q| q != NONE);
    whole.then_some(pairs.point)
}
