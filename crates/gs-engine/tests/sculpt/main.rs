//! Sculpting through `brush.stroke`, the way agents and the window's pen reach it: the brushes'
//! claims hold through the engine (nothing outside the brush moves, masked vertices stay, a
//! mirrored stroke leaves a symmetric part symmetric), every stroke is one undo step that the
//! command log replays bit for bit at any thread count, a refining stroke (Live Detail) keeps
//! the part a valid surface, and a stroke that changes nothing says why. Registry properties
//! and differential checks only (ARCHITECTURE 10.1).

mod alphas;
mod brush_files;
mod brushes;
mod claims;
mod curve_by_pen;
mod curve_files;
mod curve_points;
mod depth;
mod detail;
mod effects;
mod errors;
mod feather;
mod feel;
mod history;
mod mirror_split;
mod misses;
mod off_model;
mod paint;
mod palette;
mod pen_feel;
mod pick;
mod pull_out;
mod refine;
mod screen;
mod settings;
mod slots;
mod steps;
mod stroke_types;
mod symmetry;
mod symmetry_set;
mod tilt;
mod wrap;

use std::path::PathBuf;

use gs_engine::EngineConfig;
use gs_protocol::{PlainMesh, Role};
use gs_testkit::Session;
use serde_json::{Value, json};

/// An agent's session holding a quad sphere of radius 1 named `sphere`, logging to `log`.
pub fn sphere_session(log: Option<PathBuf>) -> Session {
    let config = EngineConfig {
        log_path: log,
        ..EngineConfig::default()
    };
    let mut s = Session::with(config, Role::Agent);
    s.ok("primitive.sphere", json!({ "subdivisions": 24 }));
    s
}

/// World samples along the straight line from `a` to `b`, `n` of them.
pub fn line(a: [f32; 3], b: [f32; 3], n: usize) -> Vec<[f32; 3]> {
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
        })
        .collect()
}

/// `brush.stroke` arguments for `brush` along `points`, plus `extra` arguments.
pub fn stroke(brush: &str, points: &[[f32; 3]], extra: &Value) -> Value {
    let samples: Vec<Value> = points.iter().map(|p| json!({ "position": p })).collect();
    with(json!({ "brush": brush, "samples": samples }), extra)
}

/// `args` with every argument of `extra` added.
pub fn with(mut args: Value, extra: &Value) -> Value {
    if let (Value::Object(map), Value::Object(more)) = (&mut args, extra) {
        map.extend(more.clone());
    }
    args
}

/// A log path in the build's scratch folder, one per test name. The engine keeps an older log
/// under another name when it starts a new one, so a leftover never mixes in.
pub fn log_path(name: &str) -> PathBuf {
    let file = format!("sculpt-{name}-{}.jsonl", std::process::id());
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(file)
}

/// The `brush.stroke` lines of a log, in order.
pub fn stroke_lines(log: &std::path::Path) -> Vec<gs_protocol::LogRecord> {
    use gs_protocol::ScriptLine;
    let text = std::fs::read_to_string(log).unwrap_or_else(|e| panic!("no log: {e}"));
    (text.lines())
        .filter_map(|l| match ScriptLine::parse(l) {
            Ok(ScriptLine::Record(r)) if r.command == "brush.stroke" => Some(r),
            _ => None,
        })
        .collect()
}

/// Each vertex's position and vertex attribute values as bits, for two versions of a part, so
/// equal means identical. A column only one version has reads as zeros in the other, except
/// color, which reads as white (its empty value: a first paint stroke adds the column).
pub fn vertex_bits(a: &PlainMesh, b: &PlainMesh) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
    use gs_protocol::{AttrDomain, AttrValues};
    let names: std::collections::BTreeSet<&String> = (a.attributes.iter().chain(&b.attributes))
        .filter(|(_, c)| c.domain == AttrDomain::Vertex)
        .map(|(n, _)| n)
        .collect();
    let bits = |m: &PlainMesh| -> Vec<Vec<u32>> {
        (0..m.positions.len())
            .map(|v| {
                let mut out: Vec<u32> = m.positions[v].iter().map(|x| x.to_bits()).collect();
                for name in &names {
                    out.push(match m.attributes.get(*name).map(|c| &c.values) {
                        Some(AttrValues::F32(x)) => x[v].to_bits(),
                        Some(AttrValues::U32(x)) => x[v],
                        None if name.as_str() == "color" => u32::MAX,
                        None => 0,
                    });
                }
                out
            })
            .collect()
    };
    (bits(a), bits(b))
}
