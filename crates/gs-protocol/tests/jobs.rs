//! `job.start` parameters: what they turn into, read from JSON as a client sends them.

use gs_protocol::{Request, StartParams, method};
use serde_json::{Value, json};

fn request(params: Value) -> Result<Request, String> {
    let params: StartParams =
        serde_json::from_value(params).unwrap_or_else(|e| panic!("not start params: {e}"));
    params.request()
}

#[test]
fn a_start_is_exactly_one_command_or_one_batch() {
    let run = json!({ "command": "primitive.sphere", "args": { "subdivisions": 2 } });
    let one = request(json!({ "run": run, "wait_ms": 5 }));
    let Ok(Request::Run(params)) = &one else {
        panic!("one command: {one:?}");
    };
    assert_eq!(params.args, json!({ "subdivisions": 2 }), "its arguments");
    assert_eq!(
        one.as_ref().map(Request::name),
        Ok("primitive.sphere"),
        "named by it"
    );

    let step = json!({ "command": "primitive.cube" });
    let batch = request(json!({ "batch": { "commands": [step, step] } }));
    let Ok(Request::Batch(params)) = &batch else {
        panic!("a batch: {batch:?}");
    };
    assert_eq!(params.commands.len(), 2, "both steps");
    assert_eq!(
        batch.as_ref().map(Request::name),
        Ok(method::BATCH),
        "a batch is cmd.batch"
    );

    let both = json!({ "run": { "command": "a" }, "batch": { "commands": [] } });
    for wrong in [json!({}), both] {
        let refused = request(wrong.clone());
        assert!(refused.is_err_and(|e| e.contains("exactly one")), "{wrong}");
    }
}
