//! Judging what the program made (decision 0038, section 3, step 10), since its exit code
//! says nothing: every face four distinct points, no edge on more than two faces, and the same
//! shells, open boundary loops and Euler characteristic as the triangles it was given.

use super::prep::shells;

/// What a mesh's shape is, up to how finely it is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Pieces joined by faces.
    pub shells: usize,
    /// Open boundary loops: holes and cut edges.
    pub loops: usize,
    /// Points minus edges plus faces, over the points faces use.
    pub euler: i64,
}

impl std::fmt::Display for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            shells,
            loops,
            euler,
        } = self;
        write!(
            f,
            "{shells} shells, {loops} open loops, Euler characteristic {euler}"
        )
    }
}

/// The shape of `faces` over `points` points, or the first edge on more than two faces.
pub(crate) fn shape<'a>(
    points: usize,
    faces: impl Iterator<Item = &'a [u32]> + Clone,
) -> Result<Shape, String> {
    let mut used = vec![false; points];
    let mut edges: Vec<[u32; 2]> = Vec::new();
    let mut face_count = 0_i64;
    for face in faces.clone() {
        face_count += 1;
        for (j, &v) in face.iter().enumerate() {
            used[v as usize] = true;
            let w = face[(j + 1) % face.len()];
            edges.push([v.min(w), v.max(w)]);
        }
    }
    gs_par::sort(&mut edges);
    let mut rims = Vec::new();
    let mut edge_count = 0_i64;
    for run in edges.chunk_by(|a, b| a == b) {
        edge_count += 1;
        match run.len() {
            1 => rims.push(run[0]),
            2 => {}
            n => return Err(format!("edge {}-{} is on {n} faces", run[0][0], run[0][1])),
        }
    }
    let shell = shells(points, faces);
    let roots = |on: &[bool], root: &[u32]| {
        (0..points)
            .filter(|&v| on[v] && root[v] == v as u32)
            .count()
    };
    let mut on_rim = vec![false; points];
    for &v in rims.iter().flatten() {
        on_rim[v as usize] = true;
    }
    let rim = shells(points, rims.iter().map(<[u32; 2]>::as_slice));
    let point_count: i64 = used.iter().map(|&u| i64::from(u)).sum();
    Ok(Shape {
        shells: roots(&used, &shell),
        loops: roots(&on_rim, &rim),
        euler: point_count - edge_count + face_count,
    })
}

/// Why the quads `faces` over `points` points fail, if they do, against the shape `before`
/// of the triangles they were made from.
pub(crate) fn judge(points: usize, faces: &[Vec<u32>], before: Shape) -> Result<(), String> {
    if faces.is_empty() {
        return Err("no faces".into());
    }
    for (f, face) in faces.iter().enumerate() {
        let distinct = (1..face.len()).all(|i| !face[..i].contains(&face[i]));
        if face.len() != 4 || !distinct {
            return Err(format!("face {f} is not a quad of four distinct points"));
        }
    }
    let after = shape(points, faces.iter().map(Vec::as_slice))?;
    if after != before {
        return Err(format!("the quads have {after}; the input had {before}"));
    }
    Ok(())
}
