//! A step that dies or leaves nothing (decision 0038, amended 2026-10-13): a stand-in for the
//! program, two shell scripts in a program folder inside the job folder, kills itself or exits
//! without its file on purpose. The kills run plainly and inside bubblewrap when it is
//! installed, where a death by a signal reaches GigaSculpt as bubblewrap's exit code.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use gs_geo::quad::{Job, PIN, Program, QuadError, Quads, bwrap_in, remesh};
use gs_testkit::fixtures;

use super::{job, to};

/// A cube's six quads as `printf` writes them: what both stand-in steps leave.
const CUBE: &str = "v -1 -1 -1\\nv 1 -1 -1\\nv 1 1 -1\\nv -1 1 -1\\nv -1 -1 1\\nv 1 -1 1\\n\
    v 1 1 1\\nv -1 1 1\\nf 1 4 3 2\\nf 5 6 7 8\\nf 1 2 6 5\\nf 2 3 7 6\\nf 3 4 8 7\\nf 4 1 5 8\\n";
/// Killed by SIGKILL on its first run only, leaving `killed` in the job folder.
const KILLED_ONCE: &str = "if [ ! -e killed ]; then : > killed; kill -9 $$; fi";

/// Writes `text` to `path`, runnable, making its folder.
fn put(path: &Path, text: &str) {
    let made = (path.parent().map_or(Ok(()), fs::create_dir_all))
        .and_then(|()| fs::write(path, text))
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(0o755)));
    made.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// A job folder holding a stand-in program, run inside `bwrap` when given. Each step notes its
/// start in `runs`, prints "stand-in" and its name, runs `step1` or `step2` (shell), then leaves
/// its files: step 1 the cube as its remeshed mesh, and patches; step 2 the cube's quads.
fn stand_in(name: &str, step1: &str, step2: &str, bwrap: Option<PathBuf>) -> (Job, Program) {
    let job = job(&format!("stand-in-{name}"));
    let folder = job.dir().join("program");
    put(&folder.join("stamp"), PIN);
    put(
        &folder.join("config/main_config/flow_noalign_lemon.txt"),
        "scaleFact 1\n",
    );
    let write = |exe: &str, body: &str, out: &str| {
        let script = format!("#!/bin/sh\necho {exe} >> runs\necho stand-in {exe}\n{body}\n");
        put(
            &folder.join(exe),
            &(script + &format!("printf '{CUBE}' > {out}\n")),
        );
    };
    write(
        "quadwild",
        &format!("{step1}\n: > mesh_rem_p0.patch"),
        "mesh_rem_p0.obj",
    );
    write(
        "quad_from_patches",
        step2,
        "mesh_rem_p0_0_quadrangulation_smooth.obj",
    );
    let program = Program::find(&[folder], bwrap).unwrap_or_else(|why| panic!("stand-in: {why}"));
    (job, program)
}

/// A sphere remeshed by the stand-in into the cube's six quads, and the steps that started.
fn run(
    job: &Job,
    program: &Program,
    cancelled: &dyn Fn() -> bool,
) -> (Result<Quads, QuadError>, String) {
    let out = remesh(&fixtures::icosphere(1), &to(6), program, job, cancelled);
    let runs = fs::read_to_string(job.dir().join("runs")).unwrap_or_default();
    (out, runs.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Plainly, and inside bubblewrap when it is installed.
fn sandboxes() -> Vec<Option<PathBuf>> {
    let bwrap = bwrap_in(std::env::var_os("PATH"));
    std::iter::once(None).chain(bwrap.map(Some)).collect()
}

/// The refusal's words, if it refused.
fn refusal(out: Result<Quads, QuadError>) -> Option<String> {
    out.err().map(|e| e.to_string())
}

#[test]
fn a_step_killed_by_a_signal_is_refused_naming_the_signal() {
    for (i, bwrap) in sandboxes().into_iter().enumerate() {
        let (job, program) = stand_in(&format!("term-{i}"), "kill -15 $$", "", bwrap);
        let (out, runs) = run(&job, &program, &|| false);
        let sandboxed = program.sandboxed();
        assert_eq!(
            refusal(out).as_deref(),
            Some(
                "step 1 failed: the program was killed by signal 15 (SIGTERM); the program's \
                 last output: stand-in quadwild"
            ),
            "sandboxed: {sandboxed}"
        );
        assert_eq!(
            runs, "quadwild",
            "only SIGKILL runs again (sandboxed: {sandboxed})"
        );
    }
}

#[test]
fn a_step_that_leaves_no_file_names_the_file() {
    let (job, program) = stand_in("no-file", "", "exit 0", None);
    let (out, _) = run(&job, &program, &|| false);
    assert_eq!(
        refusal(out).as_deref(),
        Some(
            "step 2 failed: it left no mesh_rem_p0_0_quadrangulation_smooth.obj; the program's \
             last output: stand-in quad_from_patches"
        )
    );
}

#[test]
fn a_step_killed_by_sigkill_runs_once_more_and_twice_is_refused() {
    for (i, bwrap) in sandboxes().into_iter().enumerate() {
        let sandboxed = bwrap.is_some();
        let (job, program) = stand_in(&format!("once-{i}"), "", KILLED_ONCE, bwrap.clone());
        let (out, runs) = run(&job, &program, &|| false);
        let quads = out.unwrap_or_else(|e| panic!("killed once (sandboxed: {sandboxed}): {e}"));
        assert_eq!(quads.mesh.faces.len(), 6, "the rerun's quads");
        assert_eq!(runs, "quadwild quad_from_patches quad_from_patches");
        let (job, program) = stand_in(&format!("twice-{i}"), "", "kill -9 $$", bwrap);
        let (out, runs) = run(&job, &program, &|| false);
        assert_eq!(
            refusal(out).as_deref(),
            Some(
                "step 2 failed: the program was killed by signal 9 (SIGKILL) twice; the \
                 program's last output: stand-in quad_from_patches"
            ),
            "sandboxed: {sandboxed}"
        );
        assert_eq!(runs, "quadwild quad_from_patches quad_from_patches");
    }
}

#[test]
fn a_killed_step_does_not_run_again_after_cancel() {
    let (job, program) = stand_in("cancel", KILLED_ONCE, "", None);
    let killed = job.dir().join("killed");
    let (out, runs) = run(&job, &program, &|| killed.exists());
    assert_eq!(
        out.err(),
        Some(QuadError::Cancelled),
        "cancel came with the kill"
    );
    assert_eq!(runs, "quadwild", "no rerun after cancel");
    // A rerun empties the step's log before it starts, however soon it is stopped.
    let log = fs::read_to_string(job.dir().join("step1.log")).unwrap_or_default();
    assert_eq!(
        log.trim(),
        "stand-in quadwild",
        "the killed run's log, never started again"
    );
}

#[test]
fn a_killed_step_does_not_run_again_at_the_time_limit() {
    let (job, program) = stand_in("limit", KILLED_ONCE, "", None);
    // No ticks: the time is up at the first check, whether the kill comes before it or not. With
    // one tick a kill seen before the first tick is run again, as the rule says.
    let (out, runs) = run(&job, &program.limit(0), &|| false);
    assert_eq!(
        out.err(),
        Some(QuadError::TimedOut),
        "the limit came with the kill"
    );
    assert!(
        matches!(runs.as_str(), "" | "quadwild"),
        "no rerun at the limit: {runs}"
    );
}
