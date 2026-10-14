//! Scene files through the engine (decision 0016): `file.save`, `file.open`, `file.inspect`,
//! `file.new`, `file.revert`, `file.save_next`, the recent files, autosave and recovery. Kinds
//! from ARCHITECTURE 10.1: round trips through commands (a saved and reopened scene hashes the
//! same and the next commands give the same bits), determinism (two sessions saving the same
//! scene write the same bytes), oracle tests (damaged, unsaved, too-new and misnamed cases are
//! refused with the right error), a golden file per format version, real kills of a child
//! process during a save and during a session, and the Reference Views a scene file keeps.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test helpers fail loudly; clippy.toml allows unwrap and expect in tests"
)]

mod autosave;
mod background;
mod boolean_props;
mod crash;
mod files;
mod frozen;
mod golden;
mod levels;
mod mid_gesture;
mod paint_visible;
mod palette;
mod palette_session;
mod references;
mod roundtrip;
mod uv_hints;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use gs_engine::{EngineConfig, StateFolder};
use gs_protocol::Role;
use gs_testkit::Session;
use serde_json::{Value, json};

/// A fresh folder for one test's files, under the test runner's temporary folder.
pub fn folder(name: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("scene-{name}-{}-{n}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `dir/name` as text.
pub fn at(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// A human's session (so nothing waits on a policy), seed `seed`, logging to `log`, with
/// autosave in `state` when given.
pub fn session(seed: u64, log: Option<PathBuf>, state: Option<&Path>) -> Session {
    let config = EngineConfig {
        seed,
        log_path: log,
        autosave: state.map(|s| StateFolder::in_folder(s.to_path_buf())),
        ..EngineConfig::default()
    };
    Session::with(config, Role::Human)
}

/// Two quads and a triangle with face groups, as OBJ text.
pub const OBJ: &str = "v 0 0 0\nv 1 0 0.25\nv 1 1 0\nv 0 1 0\nv 2.5 0 0.1\nv 2 1 0.3\nv 3 0.5 0.3\n\
g left\nf 1 2 3 4\ng right\nf 2 5 6 3\nf 5 7 6\n";

/// A stroke of `brush` across the front of the unit sphere.
pub fn stroke(brush: &str, from: [f32; 3], to: [f32; 3]) -> Value {
    let samples: Vec<Value> = (0..6)
        .map(|i| {
            let t = i as f32 / 5.0;
            let p: [f32; 3] = std::array::from_fn(|k| from[k] + (to[k] - from[k]) * t);
            json!({ "position": p })
        })
        .collect();
    json!({ "part": "sphere", "brush": brush, "size": "15%", "samples": samples })
}

/// A scene with what a file must keep: a sculpted sphere in store order with a mask on some
/// nodes and Voxel Remesh's kept cell size, and an imported part in its file's order with face
/// groups and an export scale.
pub fn build(s: &mut Session, dir: &Path) {
    s.ok("primitive.sphere", json!({ "subdivisions": 10 }));
    s.ok(
        "mesh.voxel_remesh",
        json!({ "part": "sphere", "resolution": 24 }),
    );
    s.ok(
        "brush.stroke",
        stroke("Draw", [-0.4, 0.1, 1.0], [0.4, 0.3, 1.0]),
    );
    s.ok(
        "brush.stroke",
        stroke("Mask Pen", [0.0, -0.5, 1.0], [0.3, -0.3, 1.0]),
    );
    let obj = at(dir, "painted.obj");
    std::fs::write(&obj, OBJ).unwrap();
    s.ok(
        "file.import",
        json!({ "path": obj, "scale": 0.5, "name": "painted" }),
    );
}

/// What happens next in the scene: a stroke and a random jitter whose seed the engine draws
/// from the session seed and the command counter.
pub fn next(s: &mut Session) -> String {
    s.ok(
        "brush.stroke",
        stroke("Draw", [0.2, 0.4, 1.0], [-0.2, 0.0, 1.0]),
    );
    s.ok("mesh.jitter", json!({ "part": "painted", "amount": "2%" }));
    s.hash()
}

/// `file.save`'s answer, or the test fails.
pub fn save(s: &mut Session, args: Value) -> Value {
    s.ok("file.save", args).result
}
