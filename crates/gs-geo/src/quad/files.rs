//! The files the program reads and writes (decision 0038, sections 2 and 3): triangles out as
//! OBJ, written in a fixed order with Rust's shortest exact float formatting so the same mesh
//! gives the same bytes; meshes back from OBJ; the sharp-edge file; the two settings files.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::QuadError;
use super::run::{MAIN_CONFIG, io};

/// Step 1's settings: research 20's organic preset (`basic_setup_Organic.txt`), the program's
/// own remesh on and its angle detection off.
pub(crate) const PREP: &str = "do_remesh 1\nsharp_feature_thr -1\nalpha 0.02\nscaleFact 1\n";

/// A mesh read from an OBJ file: points and faces of any size.
#[derive(Debug, Default)]
pub(crate) struct Obj {
    pub points: Vec<[f64; 3]>,
    pub faces: Vec<Vec<u32>>,
}

/// Writes `points` and triangles `tris` to `path` as OBJ.
pub(crate) fn write_obj(
    path: &Path,
    points: &[[f32; 3]],
    tris: &[[u32; 3]],
) -> Result<(), QuadError> {
    let mut text = String::with_capacity(32 * (points.len() + tris.len()));
    for [x, y, z] in points {
        writeln!(text, "v {x} {y} {z}").unwrap_or_default();
    }
    for [a, b, c] in tris {
        writeln!(text, "f {} {} {}", a + 1, b + 1, c + 1).unwrap_or_default();
    }
    fs::write(path, text).map_err(|e| io(path, &e))
}

/// Reads the OBJ file a step left at `path`: its `v` and `f` lines, a face corner's index before
/// any `/`. Every error names the file.
pub(crate) fn read_obj(path: &Path) -> Result<Obj, String> {
    let text = fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("it left no {}", file_name(path)),
        _ => format!("{}: {e}", file_name(path)),
    })?;
    let mut obj = Obj::default();
    for (n, line) in text.lines().enumerate() {
        let mut words = line.split_whitespace();
        let bad = || format!("line {} of {} is not understood", n + 1, file_name(path));
        match words.next() {
            Some("v") => {
                let mut p = [0.0; 3];
                for c in &mut p {
                    *c = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
                }
                obj.points.push(p);
            }
            Some("f") => {
                let face = words
                    .map(|w| w.split('/').next().and_then(|i| i.parse::<u32>().ok()))
                    .map(|i| i.filter(|&i| i >= 1).map(|i| i - 1))
                    .collect::<Option<Vec<u32>>>()
                    .ok_or_else(bad)?;
                obj.faces.push(face);
            }
            _ => {}
        }
    }
    let n = obj.points.len();
    if let Some(f) = (obj.faces.iter()).position(|f| f.iter().any(|&v| v as usize >= n)) {
        return Err(format!(
            "face {f} of {} uses a missing point",
            file_name(path)
        ));
    }
    Ok(obj)
}

/// The file name of `path`, for messages.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Writes the sharp-edge file: a count, then one `kind,face,edge` line per edge (kind 1
/// convex, 0 concave; edge `j` runs from corner `j` to corner `j + 1` of triangle `face`, in
/// the order of the OBJ file).
pub(crate) fn write_sharp(path: &Path, edges: &[super::prep::Sharp]) -> Result<(), QuadError> {
    let mut text = format!("{}\n", edges.len());
    for e in edges {
        writeln!(text, "{},{},{}", u8::from(e.convex), e.face, e.slot).unwrap_or_default();
    }
    fs::write(path, text).map_err(|e| io(path, &e))
}

/// Step 2's settings: the program's `flow_noalign_lemon.txt` with `scale` for its `scaleFact`
/// and its two settings paths made absolute, so the step runs from any folder.
pub(crate) fn main_config(program: &Path, scale: f64) -> Result<String, QuadError> {
    let path = program.join(MAIN_CONFIG);
    let text = fs::read_to_string(&path).map_err(|e| io(&path, &e))?;
    let config = program.join("config").display().to_string();
    let mut out = String::with_capacity(text.len() + 256);
    for line in text.lines() {
        match line.split_whitespace().next() {
            Some("scaleFact") => writeln!(out, "scaleFact {scale}"),
            Some(key) if key.ends_with("_config_filename") => {
                let fixed = line.replace("\"config/", &format!("\"{config}/"));
                writeln!(out, "{fixed}")
            }
            _ => writeln!(out, "{line}"),
        }
        .unwrap_or_default();
    }
    Ok(out)
}
