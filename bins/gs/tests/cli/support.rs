//! A private runtime folder per case, and what a `gs` run printed.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::common::{exit_code, gs_command, runtime_dir, stop_key_in};
use serde_json::Value;

/// A private runtime folder for one case. Stops its sessions and removes itself when dropped.
pub struct Env {
    pub dir: PathBuf,
    /// The stop key `gs spawn` printed for each session.
    pub keys: RefCell<BTreeMap<String, String>>,
}

/// What a `gs` run printed and returned.
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim())
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.stdout))
    }

    pub fn lines(&self) -> Vec<&str> {
        self.stdout.lines().collect()
    }
}

impl Env {
    pub fn new() -> Self {
        Self {
            dir: runtime_dir("gs-cli"),
            keys: RefCell::new(BTreeMap::new()),
        }
    }

    /// `gs` with `args`, in this case's folders, with this test program as its engine.
    pub fn command(&self, args: &[&str]) -> Command {
        gs_command(&self.dir, args)
    }

    pub fn gs_with(&self, args: &[&str], env: &[(&str, &str)]) -> Out {
        let mut command = self.command(args);
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command
            .output()
            .unwrap_or_else(|e| panic!("cannot run gs: {e}"));
        Out {
            code: exit_code(output.status),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    pub fn gs(&self, args: &[&str]) -> Out {
        self.gs_with(args, &[])
    }

    /// Runs `gs` and insists it succeeds.
    pub fn ok(&self, args: &[&str]) -> Out {
        let out = self.gs(args);
        assert_eq!(
            out.code, 0,
            "gs {args:?} failed:\n{}{}",
            out.stdout, out.stderr
        );
        out
    }

    pub fn spawn(&self, name: &str) {
        self.spawn_with(name, &[]);
    }

    /// Spawns a session and keeps the stop key it prints. A busy machine can take well over
    /// the default 10 s to start an engine; `gs spawn` fails at once if the engine dies, so the
    /// long limit only stretches a real hang.
    pub fn spawn_with(&self, name: &str, flags: &[&str]) {
        let out = self.ok(&[&["spawn", name, "--wait-ms", "180000"], flags].concat());
        assert!(
            out.stdout.contains("ready"),
            "spawn says when it is ready: {}",
            out.stdout
        );
        self.keep_key(name, &out.stdout);
    }

    /// Keeps the stop key in what `gs spawn NAME` printed, so the session is stopped.
    pub fn keep_key(&self, name: &str, printed: &str) {
        let key =
            stop_key_in(printed).unwrap_or_else(|| panic!("spawn prints a stop key: {printed}"));
        self.keys.borrow_mut().insert(name.into(), key.into());
    }

    /// Stops a session with its stop key.
    pub fn stop(&self, name: &str) -> Out {
        let key = self.keys.borrow().get(name).cloned().unwrap_or_default();
        // A limit, so an engine that does not stop fails the case instead of stalling it; a
        // minute, because a busy machine can take more than a few seconds to stop one.
        self.gs(&["-s", name, "--timeout-ms", "60000", "stop", "--key", &key])
    }

    pub fn sessions(&self) -> Vec<String> {
        let found = fs::read_dir(self.dir.join("gigasculpt"))
            .into_iter()
            .flatten()
            .flatten();
        let sockets = found
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sock"));
        sockets
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect()
    }

    pub fn file(&self, name: &str, contents: &str) -> String {
        let path = self.dir.join(name);
        fs::write(&path, contents)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        path.display().to_string()
    }

    /// The command log a session writes, as `session.info` names it.
    pub fn log_of(&self, session: &str) -> String {
        let out = self.ok(&["-s", session, "query", "session.info", "--json"]);
        let path = out.json()["ok"]["result"]["log_path"].clone();
        path.as_str().unwrap_or_default().to_string()
    }

    pub fn hash(&self, session: &str) -> String {
        let out = self.ok(&["-s", session, "query", "mesh.hash", "--json"]);
        out.json()["ok"]["result"]["hash"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for session in self.sessions() {
            self.stop(&session);
        }
        // An engine `gs stop` could not reach (a broken build under mutation testing) would
        // otherwise run forever. Engines inherit this case's private runtime folder, so find
        // them by it.
        let marker = format!("XDG_RUNTIME_DIR={}", self.dir.display());
        for process in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let environ = fs::read(process.path().join("environ")).unwrap_or_default();
            if environ
                .split(|&b| b == 0)
                .any(|var| var == marker.as_bytes())
            {
                Command::new("kill")
                    .arg("-9")
                    .arg(process.file_name())
                    .status()
                    .ok();
            }
        }
        fs::remove_dir_all(&self.dir).ok();
    }
}
