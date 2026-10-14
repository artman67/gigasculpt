//! Real kills (decision 0016, sections 4, 11 and 14): a child process killed while it saves
//! leaves the old file whole and openable; a child session killed after an autosave and twenty
//! more commands is recovered, after a simulated reboot, to the same scene hash. The child is
//! this test program run again with one test selected and an environment variable naming its
//! part, its state and runtime folders in the test's folder. Slow, so tier 2.

pub mod tier2 {
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    use gs_engine::{EngineConfig, StateFolder};
    use gs_protocol::Role;
    use gs_testkit::Session;
    use serde_json::json;

    use crate::{at, folder, save, session, stroke};

    /// The variable that makes this program a child, and what for.
    const CHILD: &str = "GS_SCENE_CHILD";

    /// A child process, killed when dropped, so a failing test never leaves one running.
    struct Kid(Child);

    impl Drop for Kid {
        fn drop(&mut self) {
            self.0.kill().ok();
            self.0.wait().ok();
        }
    }

    /// This program again, running only `test` as a child with `dir`, its stdout piped, and
    /// `dir/state` and `dir/runtime` as its state and runtime folders.
    fn child(test: &str, dir: &Path) -> Kid {
        let me = std::env::current_exe().unwrap();
        let c = Command::new(me)
            .args([test, "--exact", "--nocapture", "--test-threads=1"])
            .env(CHILD, dir)
            .env("XDG_STATE_HOME", dir.join("state"))
            .env("XDG_RUNTIME_DIR", dir.join("runtime"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Kid(c)
    }

    /// Writes one line to the parent.
    fn tell(line: &str) {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").unwrap();
        out.flush().unwrap();
    }

    /// What the child writes, line by line.
    fn said(c: &mut Kid) -> impl Iterator<Item = String> + use<> {
        BufReader::new(c.0.stdout.take().unwrap())
            .lines()
            .map_while(Result::ok)
    }

    /// What follows `word` in the next line the child writes that holds it. (The test runner
    /// may have started the line with the test's name.)
    fn heard(lines: &mut impl Iterator<Item = String>, word: &str) -> String {
        let found = lines.find_map(|l| l.split_once(word).map(|(_, rest)| rest.to_string()));
        found.unwrap_or_else(|| panic!("the child never said {word}"))
    }

    /// The child's part: saves a big scene over `scene.gss` again and again, until killed (at
    /// most 200 times, should nobody kill it).
    fn saving(dir: &Path) {
        let path = at(dir, "scene.gss");
        let mut s = session(1, None, None);
        s.ok("primitive.sphere", json!({ "subdivisions": 180 }));
        tell(&format!("@hash {}", s.hash()));
        for _ in 0..200 {
            tell("@saving");
            s.ok("file.save", json!({ "path": path, "overwrite": true }));
        }
    }

    #[test]
    fn a_save_killed_part_way_leaves_the_old_file_openable() {
        if let Some(dir) = std::env::var_os(CHILD) {
            return saving(Path::new(&dir));
        }
        let dir = folder("killed-save");
        let path = at(&dir, "scene.gss");
        let mut s = session(1, None, None);
        s.ok("primitive.sphere", json!({ "subdivisions": 4 }));
        save(&mut s, json!({ "path": path }));
        let (old, old_bytes) = (s.hash(), std::fs::read(&path).unwrap());
        for delay_ms in [0, 10, 40, 90, 160, 250] {
            std::fs::write(&path, &old_bytes).unwrap();
            let test = "crash::tier2::a_save_killed_part_way_leaves_the_old_file_openable";
            let mut c = child(test, &dir);
            let mut lines = said(&mut c);
            let new = heard(&mut lines, "@hash ");
            heard(&mut lines, "@saving");
            std::thread::sleep(Duration::from_millis(delay_ms));
            drop(c);
            let mut r = session(1, None, None);
            r.ok(
                "file.open",
                json!({ "path": path, "discard_changes": true }),
            );
            let now = r.hash();
            assert!(
                now == old || now == new,
                "killed after {delay_ms} ms: a whole file"
            );
        }
    }

    /// The child's part: a session in the standard places (`gigasculpt serve` and the window
    /// find them the same way), with autosave on, an autosave, twenty more commands, then its
    /// scene hash; it waits to be killed (for at most two minutes).
    fn working() {
        let config = EngineConfig {
            seed: 5,
            autosave: StateFolder::standard(),
            ..EngineConfig::default()
        };
        let mut s = Session::with(config.with_log(None).unwrap(), Role::Human);
        s.ok("primitive.sphere", json!({ "subdivisions": 12 }));
        s.ok("file.quick_save", json!({}));
        for i in 0..20u8 {
            let x = f32::from(i) * 0.03 - 0.3;
            let (command, args) = match i % 4 {
                0 => (
                    "brush.stroke",
                    stroke("Draw", [x, -0.2, 1.0], [x, 0.3, 1.0]),
                ),
                1 => ("mesh.jitter", json!({ "part": "sphere", "amount": "0.5%" })),
                2 => ("history.undo", json!({})),
                _ => (
                    "mesh.translate",
                    json!({ "part": "sphere", "direction": [0, 1, 0], "distance": x }),
                ),
            };
            s.ok(command, args);
        }
        tell(&format!("@hash {}", s.hash()));
        std::thread::sleep(Duration::from_secs(120));
    }

    /// A power loss: the session is killed, and the reboot empties the runtime folder. A new
    /// session of the same name, its log folder pruned to nothing it may delete, is offered the
    /// crashed session's work and replays it from the log the state folder kept.
    #[test]
    fn a_killed_session_recovers_to_its_last_command() {
        if std::env::var_os(CHILD).is_some() {
            return working();
        }
        let dir = folder("killed-session");
        let test = "crash::tier2::a_killed_session_recovers_to_its_last_command";
        let mut c = child(test, &dir);
        let pid = c.0.id();
        let last = heard(&mut said(&mut c), "@hash ");
        drop(c);
        let env = |name: &str| (name == "XDG_STATE_HOME").then(|| dir.join("state").into());
        let state = gs_protocol::state_dir_in(gs_protocol::System::HERE, &env).unwrap();
        let runtime = dir.join("runtime");
        std::fs::remove_dir_all(&runtime).ok();
        std::fs::create_dir_all(&runtime).unwrap();
        let mut config = EngineConfig {
            autosave: Some(StateFolder::in_folder(state.clone())),
            ..EngineConfig::default()
        }
        .with_standard_log(state.clone());
        if let Some(standard) = config.standard_log.as_mut() {
            (standard.files, standard.bytes) = (0, 0);
        }
        let mut r = Session::with(config, Role::Human);
        let crashed = state.join("logs").join(format!("main-{pid}.jsonl"));
        assert!(crashed.exists(), "the crashed run's log is kept");
        let offered = r.query("file.recovery", json!({}));
        let candidate = &offered["candidates"][0];
        assert_eq!(candidate["lines_after"], json!(20), "{offered}");
        assert_eq!(candidate["replayable"], json!(true), "{offered}");
        let id = candidate["id"].clone();
        let recovered = r.ok("file.recover", json!({ "session": id })).result;
        assert_eq!(recovered["replayed"], json!(20), "{recovered}");
        assert_eq!(r.hash(), last, "the scene as it was at the last command");
        let after = r.query("file.recovery", json!({}));
        assert_eq!(
            after["candidates"],
            json!([]),
            "a recovered session is offered no more"
        );
    }
}
