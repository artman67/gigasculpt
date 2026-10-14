//! Long commands from the window run as jobs (decision 0010 as amended 2026-10-13): a Quad
//! Remesh leaves the window's loop answering, the status line's bar shows its name and Cancel
//! and clears once it lands, Cancel leaves the scene and History as they were, and while it runs
//! an edit is refused with its name and the view still turns. A real session server with the
//! window's host attached, driven through the window loop's own command path
//! (`app::commands`), the remesh run by the real program.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use gs_engine::EngineConfig;
use gs_engine::server::{JobWatch, ServeOptions, Server, Window};
use gs_protocol::{Outcome, Request, RunParams, Step};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Shell, Studio, app};
use serde_json::{Value, json};

/// A sphere whose Quad Remesh takes seconds, so each test's steps all land while it runs.
const BIG: u32 = 96;

/// A session server with the window's host attached, as the window starts one, without a
/// renderer. Stops the session (which cancels its jobs) and removes its socket when dropped.
/// `asking` drives saves and opens through it too.
pub(crate) struct Rig {
    server: Option<Server>,
    pub(crate) host: Host,
    window: Window,
    socket: PathBuf,
}

impl Rig {
    pub(crate) fn start(name: &str) -> Self {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(|| "/tmp".into(), PathBuf::from);
        let socket = base.join(format!("gs-busy-{}-{name}.sock", std::process::id()));
        let config = EngineConfig {
            session: name.into(),
            ..EngineConfig::default()
        };
        let options = ServeOptions {
            socket: socket.clone(),
            stop_key: None,
            max_line_bytes: 1 << 20,
        };
        let mut host = None;
        let server = Server::start_with(config, &options, |engine| {
            let layout = LayoutFile::in_memory(Layout::default());
            let shell = Shell::new(engine.catalog().clone(), Keymap::default(), layout);
            let window = Host::new(Studio::new(shell, None, [1000, 700], 1.0));
            engine.attach_window(Box::new(window.clone()));
            host = Some(window);
        })
        .unwrap_or_else(|e| panic!("serve: {e}"));
        let host = host.unwrap_or_else(|| panic!("the window was attached before serving"));
        let window = server.window();
        Self {
            server: Some(server),
            host,
            window,
            socket,
        }
    }

    /// Runs `command` as the human at the window and gives its result or its refusal's message.
    fn run(&self, command: &str, args: Value) -> Result<Value, String> {
        let request = Request::Run(RunParams::new(command, args));
        let response = (self.window.execute(request))
            .unwrap_or_else(|| panic!("the engine stopped before {command}"));
        match response.outcome {
            Outcome::Ok(done) => Ok(done.result),
            Outcome::Error(e) => Err(e.message),
        }
    }

    pub(crate) fn ok(&self, command: &str, args: Value) -> Value {
        (self.run(command, args)).unwrap_or_else(|e| panic!("{command}: {e}"))
    }

    /// Asks for `command` as a control does, then runs one pass of the window's command path.
    pub(crate) fn press(&self, command: &str, args: Value) {
        let step = Step {
            command: command.into(),
            args,
        };
        self.host.lock().shell.queue(step);
        self.pass();
    }

    /// One pass of the window loop's command path.
    fn pass(&self) {
        app::commands(&self.host, &self.window).unwrap_or_else(|e| panic!("{e}"));
    }

    /// The window's job, while the status line's bar shows one.
    fn job(&self) -> Option<JobWatch> {
        let studio = self.host.lock();
        studio.shell.status.busy.as_ref().map(|b| b.job.clone())
    }

    fn running(&self) -> JobWatch {
        self.job()
            .unwrap_or_else(|| panic!("the window's call runs as a job"))
    }

    /// The status line as one frame draws it, left to right: each widget's label and what it
    /// runs.
    fn status_line(&self) -> Vec<(String, Option<Step>)> {
        let mut studio = self.host.lock();
        studio.interface();
        let panels = studio.shell.panels();
        let line = panels.iter().find(|p| p.id == "status");
        let line = line.unwrap_or_else(|| panic!("the status line is drawn"));
        let mut widgets: Vec<_> = line.widgets.iter().collect();
        widgets.sort_by(|a, b| a.rect[0].total_cmp(&b.rect[0]));
        let widgets = widgets.into_iter();
        widgets
            .map(|w| (w.label.clone(), w.command.clone()))
            .collect()
    }

    /// The window's last problem, as its status line says it.
    pub(crate) fn problem(&self) -> String {
        self.host
            .lock()
            .shell
            .status
            .problem
            .clone()
            .unwrap_or_default()
    }

    /// A short pen stroke across the middle of the 3D view, as the window sends one.
    fn stroke(&self) {
        let rect = self.ok("ui.describe", json!({}))["viewport"]["rect"].clone();
        let at = |i: usize| rect[i].as_f64().unwrap_or_default();
        let [x, y] = [f64::midpoint(at(0), at(2)), f64::midpoint(at(1), at(3))];
        for (dx, contact) in [(0.0, true), (10.0, true), (20.0, true), (20.0, false)] {
            let sample = json!({ "x": x + dx, "y": y, "contact": contact, "pressure": 0.5 });
            self.ok("pen.input", json!({ "samples": [sample] }));
        }
    }

    /// Runs passes of the window's command path until the bar clears; at most two minutes.
    pub(crate) fn until_done(&self) {
        let done = (0..6000).any(|_| {
            self.pass();
            let running = self.job().is_some();
            if running {
                thread::sleep(Duration::from_millis(20));
            }
            !running
        });
        assert!(done, "the job ended within two minutes");
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.window.close();
        if let Some(server) = self.server.take() {
            server.wait().ok();
        }
        std::fs::remove_file(&self.socket).ok();
        std::fs::remove_file(self.socket.with_extension("lock")).ok();
    }
}

/// A big sphere and its Quad Remesh, asked for as the window's Remesh menu asks.
fn remeshing(name: &str) -> (Rig, JobWatch) {
    let rig = Rig::start(name);
    rig.ok("primitive.sphere", json!({ "subdivisions": BIG }));
    rig.press("mesh.quad_remesh", json!({ "faces": 2000 }));
    let job = rig.running();
    (rig, job)
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test times passes of the window's loop against a frame"
)]
fn a_quad_remesh_from_the_window_leaves_its_loop_answering_within_a_frame() {
    let (rig, job) = remeshing("busy-frame");
    assert!(
        !job.ended(),
        "the pass that asked for it came back while it runs"
    );
    // The best of five, so one pass the machine paused for says nothing.
    let quickest = (0..5)
        .map(|_| {
            let begun = Instant::now();
            rig.pass();
            begun.elapsed()
        })
        .min()
        .unwrap_or_default();
    assert!(!job.ended(), "the remesh still runs");
    let frame = Duration::from_micros(16_667);
    assert!(quickest < frame, "a pass while it runs took {quickest:?}");
}

#[test]
fn the_bar_shows_the_jobs_name_and_cancel_and_clears_once_it_lands() {
    let rig = Rig::start("busy-bar");
    rig.ok("primitive.sphere", json!({ "subdivisions": 16 }));
    rig.press("mesh.quad_remesh", json!({ "faces": 400 }));
    let job = rig.running();
    let line = rig.status_line();
    assert_eq!(
        line[0].0, "Quad remesh",
        "the bar starts the line: {line:?}"
    );
    let cancel = line.iter().find(|(label, _)| label == "Cancel");
    let cancel = cancel.and_then(|(_, step)| step.clone());
    let cancel = cancel.unwrap_or_else(|| panic!("the bar has Cancel: {line:?}"));
    assert_eq!(
        (cancel.command.as_str(), &cancel.args),
        ("job.status", &json!({ "id": job.id(), "cancel": true })),
        "Cancel cancels the job as job.status does"
    );
    rig.until_done();
    let line = rig.status_line();
    let labels: Vec<&str> = line.iter().map(|(label, _)| label.as_str()).collect();
    assert!(
        !labels.contains(&"Cancel") && !labels.contains(&"Quad remesh"),
        "the bar cleared: {labels:?}"
    );
    let faces = rig.ok("mesh.stats", json!({}))["parts"][0]["faces"].as_u64();
    let faces = faces.unwrap_or_default();
    assert!((360..=440).contains(&faces), "it landed: {faces} faces");
}

#[test]
fn cancel_leaves_the_scene_its_hash_and_history_as_they_were() {
    let rig = Rig::start("busy-cancel");
    rig.ok("primitive.sphere", json!({ "subdivisions": BIG }));
    let hash = rig.ok("mesh.hash", json!({}))["hash"].clone();
    let history = rig.ok("history.list", json!({ "limit": 1000 }));
    rig.press("mesh.quad_remesh", json!({ "faces": 2000 }));
    let job = rig.running();
    let line = rig.status_line();
    let cancel = line.iter().find(|(label, _)| label == "Cancel");
    let cancel = cancel.and_then(|(_, step)| step.clone());
    let cancel = cancel.unwrap_or_else(|| panic!("the bar has Cancel: {line:?}"));
    rig.press(&cancel.command, cancel.args);
    assert!(job.ended(), "Cancel ends it at once");
    rig.pass();
    assert!(rig.job().is_none(), "and the bar clears");
    // Its thread stops when the remesh next looks; wait for that, not for a time.
    let stopped = (0..6000).any(|_| {
        let working = job.working();
        if working {
            thread::sleep(Duration::from_millis(20));
        }
        !working
    });
    assert!(stopped, "the remesh stopped within two minutes");
    rig.pass();
    assert_eq!(rig.ok("mesh.hash", json!({}))["hash"], hash, "the scene");
    assert_eq!(
        rig.ok("history.list", json!({ "limit": 1000 })),
        history,
        "History gains no line"
    );
}

#[test]
fn while_it_runs_an_edit_is_refused_with_its_name_and_the_view_still_turns() {
    let (rig, job) = remeshing("busy-hold");
    let camera = || rig.ok("view.get", json!({ "view": "main" }))["camera"].clone();
    let before = camera();
    rig.press("primitive.cube", json!({}));
    let problem = rig.problem();
    assert!(
        problem.starts_with("Quad remesh is running"),
        "the edit is refused with the job's name: {problem}"
    );
    let undo = rig.run("history.undo", json!({})).map(|_| ());
    assert!(
        undo.as_ref()
            .is_err_and(|m| m.starts_with("Quad remesh is running")),
        "undo too: {undo:?}"
    );
    rig.press("boolean.check", json!({}));
    let problem = rig.problem();
    assert!(
        problem.starts_with("Quad remesh is running"),
        "and a second long command: {problem}"
    );
    let turn = json!({ "view": "main", "right_deg": 30, "up_deg": 20 });
    rig.ok("view.orbit", turn);
    assert_ne!(camera(), before, "the view turns");
    assert!(!job.ended(), "all while the remesh runs");
}

/// A pen stroke on the model while the remesh runs sculpts nothing, and the status line says
/// why in a person's words, with where its Cancel is, as an edit's refusal does. The line
/// clears once the remesh lands, the person's own step: its reason is gone.
#[test]
fn a_stroke_while_it_runs_says_why_in_a_persons_words_until_it_lands() {
    let (rig, job) = remeshing("busy-stroke");
    let hash = rig.ok("mesh.hash", json!({}))["hash"].clone();
    rig.stroke();
    assert!(!job.ended(), "the stroke came while the remesh ran");
    assert_eq!(rig.ok("mesh.hash", json!({}))["hash"], hash, "no sculpt");
    let said = rig.problem();
    assert!(said.starts_with("Quad remesh is running"), "{said}");
    assert!(said.contains("Its Cancel is beside its bar"), "{said}");
    assert!(!said.contains("E_"), "{said}");
    rig.press("primitive.cube", json!({}));
    let edit = rig.problem();
    assert_eq!(edit, said, "a refused edit reads the same");
    rig.stroke();
    assert!(!job.ended(), "a second stroke came while it ran");
    assert_eq!(rig.problem(), said, "and says it again");
    rig.until_done();
    assert_eq!(rig.problem(), "", "the remesh landing clears it");
}
