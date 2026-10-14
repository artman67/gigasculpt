//! Quad Remesh, stage A (decision 0038): a mesh of four-sided faces whose edges follow the
//! form, made by quadwild-bimdf, a separate GPL-3 program built from a pinned source
//! (`third_party/quadwild/`). Unlike the rest of gs-geo this runs a program, in a job folder;
//! everything around it is pure.
//!
//! 1. [`prepare`](prep): triangles, welded and repaired, tiny shells dropped, decimated above
//!    200k triangles, open rims resampled, sharp edges from creases and group borders.
//! 2. Step 1, `quadwild`: the program's own remesh, its field and patches.
//! 3. The size: `scaleFact = sqrt(area / target) / mean edge` over step 1's mesh.
//! 4. Step 2, `quad_from_patches`, and one rerun with a corrected size when the count misses
//!    the target by more than 5%; the closer result is kept.
//! 5. The quads judged ([`check`]) against the triangles' shells, loops and Euler
//!    characteristic, and with a symmetry plane, their rim snapped onto it.
//!
//! How far the quads lie from the old surface, and carrying columns across, are the caller's:
//! they need the projection search, which lives in gs-core.

mod check;
mod files;
mod prep;
mod run;

use std::ffi::{OsStr, OsString};
use std::fmt;

use gs_protocol::PlainMesh;

pub use check::Shape;
pub use prep::MOST_TRIANGLES;
pub use run::{Job, LIMIT_TICKS, Program, bwrap_in, fix, places};

use files::Obj;
use run::{Runner, io};

/// The engine build GigaSculpt expects: `third_party/quadwild/sources.lock`'s pin, also
/// written into the program folder's stamp. A test holds the two equal.
pub const PIN: &str = "e722c7e-1";
/// How far the quad count may miss the target before step 2 runs again.
const MISS: f64 = 0.05;
/// Rim points of a symmetric half this close to the plane, in mean quad edges, are put on it.
const SNAP: f32 = 0.25;
/// Step 2's result, beside step 1's.
const QUADS: &str = "mesh_rem_p0_0_quadrangulation_smooth.obj";

/// What to make.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Quads wanted. The program has no count of its own; the result lands near it.
    pub faces: usize,
    /// Group borders (the `group` face column) become borders between the program's patches.
    pub keep_groups: bool,
    /// Edges, as point pairs of the input, that become borders between patches too.
    pub creases: Vec<[u32; 2]>,
    /// The input is a half cut at the plane through the origin square to this axis: the
    /// quads' rim points near it are put exactly on it.
    pub plane: Option<usize>,
}

/// What preparing the input took.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Preparation {
    /// Triangles given to the program.
    pub triangles: u64,
    /// Triangles before decimation, when there were more than [`MOST_TRIANGLES`].
    pub decimated_from: Option<u64>,
    /// Points merged by the weld.
    pub merged: u64,
    /// Triangles dropped with shells too small to remesh.
    pub shells_dropped: u64,
    /// Rim points removed by the resampling.
    pub rim_points_removed: u64,
    /// Edges in the sharp-edge file.
    pub sharp_edges: u64,
}

/// The quads and how they were made.
#[derive(Clone, Debug)]
pub struct Quads {
    /// Quads only, in the input's space, with no columns.
    pub mesh: PlainMesh,
    /// How many times step 2 ran: 1, or 2 with the corrected size.
    pub passes: u32,
    /// The mean quad edge.
    pub edge: f32,
    /// What preparing the input took.
    pub preparation: Preparation,
}

/// Why no quads came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuadError {
    /// Cancel was asked for.
    Cancelled,
    /// The two steps ran past [`LIMIT_TICKS`].
    TimedOut,
    /// Nothing the program can work on is left after preparing the input.
    Input(String),
    /// A step left no usable file: which step, what is wrong, and the last lines it printed.
    Step {
        /// 1 or 2.
        step: u8,
        /// What was missing or unreadable.
        what: String,
        /// The program's last output.
        tail: String,
    },
    /// A step was ended by a signal, SIGKILL only on its second run: which step, the signal and
    /// the last lines it printed.
    Killed {
        /// 1 or 2.
        step: u8,
        /// The signal's number.
        signal: i32,
        /// The program's last output.
        tail: String,
    },
    /// The quads failed a check.
    Check(String),
    /// The job folder could not be written or read, or a step not started.
    Io(String),
}

impl fmt::Display for QuadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "cancelled"),
            Self::TimedOut => write!(f, "the program ran past its time limit"),
            Self::Input(why) => write!(f, "nothing to remesh: {why}"),
            Self::Step { step, what, tail } => {
                write!(
                    f,
                    "step {step} failed: {what}; the program's last output: {tail}"
                )
            }
            Self::Killed { step, signal, tail } => {
                let name = run::signal_name(*signal).map(|n| format!(" ({n})"));
                let twice = if *signal == run::SIGKILL {
                    " twice"
                } else {
                    ""
                };
                write!(
                    f,
                    "step {step} failed: the program was killed by signal {signal}{}{twice}; the \
                     program's last output: {tail}",
                    name.unwrap_or_default()
                )
            }
            Self::Check(why) => write!(f, "the quads failed a check: {why}"),
            Self::Io(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for QuadError {}

/// `mesh` remeshed into quads as `settings` say, by `program`, in `job`; `cancelled` is asked
/// every 20 ms while a step runs.
pub fn remesh(
    mesh: &PlainMesh,
    settings: &Settings,
    program: &Program,
    job: &Job,
    cancelled: &dyn Fn() -> bool,
) -> Result<Quads, QuadError> {
    if settings.faces == 0 {
        return Err(QuadError::Input("no quads asked for".into()));
    }
    let (tris, sharp, preparation) = prep::prepare(mesh, settings);
    if tris.faces.len() < 4 {
        return Err(QuadError::Input(
            "fewer than 4 triangles are left after cleaning".into(),
        ));
    }
    let before = check::shape(
        tris.positions.len(),
        tris.faces.iter().map(<[u32; 3]>::as_slice),
    )
    .map_err(QuadError::Input)?;
    let dir = job.dir();
    let (input, prep) = (dir.join("mesh.obj"), dir.join("prep.txt"));
    files::write_obj(&input, &tris.positions, &tris.faces)?;
    std::fs::write(&prep, files::PREP).map_err(|e| io(&prep, &e))?;
    let mut args: Vec<OsString> = vec![input.into(), "2".into(), prep.into()];
    if !sharp.is_empty() {
        let path = dir.join("mesh.sharp");
        files::write_sharp(&path, &sharp)?;
        args.push(path.into());
    }
    let mut runner = Runner {
        program,
        job: dir,
        cancelled,
        ticks: 0,
    };
    runner.step(1, "quadwild", &refs(&args), "step1.log")?;
    let rem = dir.join("mesh_rem_p0.obj");
    let step1 = |what: String, runner: &Runner| QuadError::Step {
        step: 1,
        what,
        tail: runner.tail("step1.log"),
    };
    if !dir.join("mesh_rem_p0.patch").is_file() {
        return Err(step1("it left no mesh_rem_p0.patch".into(), &runner));
    }
    let remeshed = files::read_obj(&rem).map_err(|e| step1(e, &runner))?;
    let scale = size(&remeshed, settings.faces)
        .ok_or_else(|| step1("it left an empty mesh".into(), &runner))?;
    let (quads, passes) = sized(&mut runner, scale, settings.faces)?;
    check::judge(quads.points.len(), &quads.faces, before).map_err(QuadError::Check)?;
    let mut mesh = plain(&quads);
    let edge = mean_edge(&mesh);
    if let Some(axis) = settings.plane {
        snap(&mut mesh, axis, SNAP * edge);
    }
    Ok(Quads {
        mesh,
        passes,
        edge,
        preparation,
    })
}

/// `args` as the borrowed form `Command` takes.
fn refs(args: &[OsString]) -> Vec<&OsStr> {
    args.iter().map(OsString::as_os_str).collect()
}

/// `scaleFact` for about `faces` quads over step 1's mesh: `sqrt(area / faces) / mean edge`,
/// every face edge counted (shared ones twice), as `quad_from_patches` counts them.
fn size(mesh: &Obj, faces: usize) -> Option<f64> {
    let (mut area, mut length, mut edges) = (0.0, 0.0, 0_usize);
    let norm = |v: [f64; 3]| crate::math::dot(v, v).sqrt();
    for face in &mesh.faces {
        let p = |i: usize| mesh.points[face[i % face.len()] as usize];
        for j in 0..face.len() {
            length += norm(crate::math::sub(p(j + 1), p(j)));
            edges += 1;
        }
        for j in 1..face.len().saturating_sub(1) {
            let (a, b) = (
                crate::math::sub(p(j), p(0)),
                crate::math::sub(p(j + 1), p(0)),
            );
            area += norm(crate::math::cross(a, b)) / 2.0;
        }
    }
    let scale = (area / faces as f64).sqrt() / (length / edges as f64);
    (scale.is_finite() && scale > 0.0).then_some(scale)
}

/// Step 2 at `scale`, and once more at a corrected scale when the count misses `target` by
/// more than [`MISS`]; the result closer to the target, and how many times step 2 ran. A
/// second run that leaves nothing usable, or is killed, keeps the first.
fn sized(runner: &mut Runner, scale: f64, target: usize) -> Result<(Obj, u32), QuadError> {
    let first = quads_at(runner, scale, "step2.log")?;
    let miss = |n: usize| (n as f64 / target as f64 - 1.0).abs();
    let n = first.faces.len();
    if miss(n) <= MISS {
        return Ok((first, 1));
    }
    let again = quads_at(
        runner,
        scale * (n as f64 / target as f64).sqrt(),
        "step2b.log",
    );
    match again {
        Ok(second) if miss(second.faces.len()) < miss(n) => Ok((second, 2)),
        Ok(_) | Err(QuadError::Step { .. } | QuadError::Killed { .. }) => Ok((first, 2)),
        Err(e) => Err(e),
    }
}

/// Step 2 at `scale`, its output to `log`, and the quads it left.
fn quads_at(runner: &mut Runner, scale: f64, log: &str) -> Result<Obj, QuadError> {
    let dir = runner.job;
    let (config, out) = (dir.join("main.txt"), dir.join(QUADS));
    let text = files::main_config(runner.program.dir(), scale)?;
    std::fs::write(&config, text).map_err(|e| io(&config, &e))?;
    if out.exists() {
        std::fs::remove_file(&out).map_err(|e| io(&out, &e))?;
    }
    let (rem, stats) = (dir.join("mesh_rem_p0.obj"), dir.join("stats.json"));
    let args: [&OsStr; 4] = [
        rem.as_os_str(),
        "0".as_ref(),
        config.as_os_str(),
        stats.as_os_str(),
    ];
    runner.step(2, "quad_from_patches", &args, log)?;
    files::read_obj(&out).map_err(|what| QuadError::Step {
        step: 2,
        what,
        tail: runner.tail(log),
    })
}

/// `quads` as a `PlainMesh` of the points they use, in order of first use.
fn plain(quads: &Obj) -> PlainMesh {
    let mut renumber = vec![u32::MAX; quads.points.len()];
    let mut positions = Vec::new();
    let faces = (quads.faces.iter())
        .map(|face| {
            std::array::from_fn(|i| {
                let r = &mut renumber[face[i] as usize];
                if *r == u32::MAX {
                    *r = positions.len() as u32;
                    positions.push(quads.points[face[i] as usize].map(|c| c as f32));
                }
                *r
            })
        })
        .collect();
    PlainMesh {
        positions,
        faces,
        attributes: std::collections::BTreeMap::new(),
    }
}

/// The shells, open boundary loops and Euler characteristic of `mesh`, or the first edge on
/// more than two faces.
pub fn shape(mesh: &PlainMesh) -> Result<Shape, String> {
    check::shape(
        mesh.positions.len(),
        mesh.faces.iter().map(crate::weld::corners),
    )
}

/// The mean edge of `mesh`'s faces, every face edge counted.
pub fn mean_edge(mesh: &PlainMesh) -> f32 {
    let mut total = 0.0_f64;
    let mut count = 0_usize;
    for face in &mesh.faces {
        let c = crate::weld::corners(face);
        for j in 0..c.len() {
            let (a, b) = (c[j] as usize, c[(j + 1) % c.len()] as usize);
            total += f64::from(crate::math::dist2(mesh.positions[a], mesh.positions[b]).sqrt());
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (total / count as f64) as f32
    }
}

/// Puts `mesh`'s open-rim points within `reach` of the plane square to `axis` onto it.
fn snap(mesh: &mut PlainMesh, axis: usize, reach: f32) {
    let mut edges: Vec<[u32; 2]> = (mesh.faces.iter())
        .flat_map(|f| (0..4).map(move |j| [f[j].min(f[(j + 1) % 4]), f[j].max(f[(j + 1) % 4])]))
        .collect();
    gs_par::sort(&mut edges);
    for run in edges.chunk_by(|a, b| a == b).filter(|r| r.len() == 1) {
        for v in run[0] {
            let p = &mut mesh.positions[v as usize];
            if p[axis].abs() <= reach {
                p[axis] = 0.0;
            }
        }
    }
}
