//! What the engine's tests add to the test kit's `Session`: sessions driven by an agent (the
//! way most clients connect, so the session policy applies), the human at the window acting on
//! the same engine in-process (the socket never grants that role), a session with a window,
//! replaying a command log the way `gs run` does, cameras and the document as views and
//! reference tests read them, and small meshes several topics share.

use std::collections::BTreeMap;
use std::path::Path;

use gs_engine::{EngineConfig, ExactCamera, Origin, Request};
use gs_protocol::{
    AttrDomain, AttrValues, Attribute, CmdResponse, LogHeader, LogRecord, PlainMesh, Role,
    RunParams, ScriptLine,
};
use gs_render::Camera;
use gs_testkit::{Session, success};
use gs_ui::{Host, Keymap, Layout, LayoutFile, Shell, Studio};
use serde_json::{Value, json};

/// A session with seed 0, driven by an agent.
pub fn agent() -> Session {
    Session::with(EngineConfig::default(), Role::Agent)
}

/// A world-space Draw stroke of size 0.4 over the part "sphere" near `at`: four samples 0.05
/// apart along x.
pub fn stroke(at: [f32; 3]) -> Value {
    let samples: Vec<Value> = (0..4)
        .map(|i| json!({ "position": [at[0] + 0.05 * i as f32, at[1], at[2]] }))
        .collect();
    json!({ "brush": "Draw", "part": "sphere", "samples": samples, "size": 0.4 })
}

/// `level.describe` of the only part.
pub fn levels(s: &mut Session) -> Value {
    s.query("level.describe", json!({}))
}

/// One triangle with a vertex mask of 0.5 and a face group of 7.
pub fn tagged_triangle() -> PlainMesh {
    let mut mesh = PlainMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        faces: vec![[0, 1, 2, u32::MAX]],
        attributes: BTreeMap::new(),
    };
    let mask = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(vec![0.5; 3]),
    };
    let group = Attribute {
        domain: AttrDomain::Face,
        values: AttrValues::U32(vec![7]),
    };
    mesh.attributes.insert("mask".into(), mask);
    mesh.attributes.insert("group".into(), group);
    mesh
}

/// A session driven by the human at the window, for the commands only the human may run.
pub fn human() -> Session {
    Session::with(EngineConfig::default(), Role::Human)
}

/// The window's door and log replay, on any test session.
pub trait EngineSession {
    /// Runs a command as the human at the window, on this session's engine.
    fn as_human(&mut self, command: &str, args: Value) -> CmdResponse;
    /// Replays a command log the way `gs run` does: each line as its request (a batch line as
    /// one batch), checking the scene hash after every line that carries one.
    fn replay_log(&mut self, log: &Path);
}

impl EngineSession for Session {
    fn as_human(&mut self, command: &str, args: Value) -> CmdResponse {
        let window = Origin {
            client_id: 0,
            client: "window".into(),
            role: Role::Human,
        };
        let params = RunParams::new(command, args);
        self.engine.execute(&window, Request::Run(params))
    }

    fn replay_log(&mut self, log: &Path) {
        let (_, records) = log_records(log);
        for (number, record) in (2..).zip(records) {
            let request = record
                .request()
                .unwrap_or_else(|e| panic!("log line {number}: {e}"));
            success(
                self.request(request),
                &format!("log line {number}: {record:?}"),
            );
            if let Some(hash) = &record.hash {
                assert_eq!(
                    &self.hash(),
                    hash,
                    "the replay diverged at log line {number}: {record:?}"
                );
            }
        }
    }
}

/// The header and every record of a command log.
pub fn log_records(log: &Path) -> (LogHeader, Vec<LogRecord>) {
    let text =
        std::fs::read_to_string(log).unwrap_or_else(|e| panic!("no log at {}: {e}", log.display()));
    let mut lines = text
        .lines()
        .map(|line| ScriptLine::parse(line).unwrap_or_else(|e| panic!("bad log line {line}: {e}")));
    let Some(ScriptLine::Header(header)) = lines.next() else {
        panic!("the log does not start with a header:\n{text}");
    };
    let records = lines
        .map(|line| match line {
            ScriptLine::Record(record) => record,
            ScriptLine::Header(_) => panic!("a second header:\n{text}"),
        })
        .collect();
    (header, records)
}

/// The camera `view.get` reports for `view`, exactly as JSON text: equal text is equal bits.
pub fn exact(s: &mut Session, view: &str) -> String {
    s.query("view.get", json!({ "view": view }))["camera"].to_string()
}

/// The camera `view.get` reports for `view`.
pub fn camera(s: &mut Session, view: &str) -> Camera {
    let v = s.query("view.get", json!({ "view": view }))["camera"].clone();
    let exact: ExactCamera = serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"));
    exact.camera("").unwrap_or_else(|e| panic!("{}", e.message))
}

/// The largest difference between two lists of points, coordinate by coordinate.
pub fn worst(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    assert_eq!(a.len(), b.len(), "the same points");
    (a.iter().zip(b))
        .flat_map(|(p, q)| (0..3).map(move |k| (p[k] - q[k]).abs()))
        .fold(0.0, f32::max)
}

/// Points as bits, so that -0.0 and 0.0 differ.
pub fn bits(points: &[[f32; 3]]) -> Vec<[u32; 3]> {
    points.iter().map(|p| p.map(f32::to_bits)).collect()
}

/// The document's fingerprint: its hash, its revision, its undo steps.
pub fn document(s: &mut Session) -> (String, u64, Value) {
    let rev = s.call("mesh.hash", json!({})).rev;
    (s.hash(), rev, s.history())
}

/// A session with a window (the real window host, without a renderer), driven by an agent.
pub fn windowed() -> Session {
    let mut s = agent();
    let layout = LayoutFile::in_memory(Layout::default());
    let shell = Shell::new(s.engine.catalog().clone(), Keymap::default(), layout);
    let host = Host::new(Studio::new(shell, None, [1000, 700], 1.0));
    s.engine.attach_window(Box::new(host));
    s
}
