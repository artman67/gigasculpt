//! What the cli and mcp test programs share (each includes this file).

use std::fs;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU32, Ordering};

/// A new private runtime folder for one case, `PREFIX-PID-N`: under `$XDG_RUNTIME_DIR` (else the
/// temporary folder), whose short path keeps session sockets under the 108-byte limit wherever
/// the tests run from. One per case, because cases may run at once in one process.
pub fn runtime_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("{prefix}-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("cannot make {}: {e}", dir.display()));
    dir
}

/// A `gs` run's exit code. A `gs` that died by a stray SIGKILL ((G-kill) in docs/STATUS.md)
/// fails the test in words the gate knows, so the gate runs it once more, as it does a test
/// killed outright.
pub fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    assert_ne!(
        status.signal(),
        Some(9),
        "gs: a child process died by a stray SIGKILL"
    );
    status.code().unwrap_or(-1)
}
