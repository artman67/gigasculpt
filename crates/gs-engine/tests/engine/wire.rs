//! The wire protocol over the socket: batches and expected revisions, misspelled and
//! malformed requests, lines over the limit, and a request split by a pause.

use std::io::{Read, Write};
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::socket::{Client, KEY, Server};

#[test]
fn batches_and_expected_revisions_work_over_the_socket() {
    let server = Server::start("batch", &[]);
    let mut a = server.client("a", "agent");
    let steps = json!([{ "command": "primitive.cube" }, { "command": "mesh.translate", "args": { "direction": [1, 0, 0], "distance": 1 } }]);
    let done = a.rpc("cmd.batch", json!({ "commands": steps }))["result"].clone();
    assert_eq!(
        (done["seq"].clone(), done["rev"].clone()),
        (json!(1), json!(1)),
        "one revision for the batch"
    );
    assert_eq!(done["ok"]["history_id"], json!(1), "one step");
    let stale =
        a.rpc("cmd.run", json!({ "command": "history.undo", "if_rev": 0 }))["result"].clone();
    assert_eq!(
        stale["error"]["code"],
        json!("E_STALE"),
        "the document moved on"
    );
    let fresh =
        a.rpc("cmd.run", json!({ "command": "history.undo", "if_rev": 1 }))["result"].clone();
    assert_eq!(fresh["rev"], json!(2), "undo at the expected revision");
    let dry = a.rpc(
        "cmd.run",
        json!({ "command": "primitive.cube", "dry_run": true }),
    )["result"]
        .clone();
    assert_eq!(
        dry["ok"]["result"],
        json!({ "dry_run": true }),
        "dry runs over the wire"
    );
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}

/// M1: a misspelled field is an error naming it, with a did-you-mean, never a silent default.
#[test]
fn misspelled_wire_fields_are_errors() {
    let server = Server::start("typos", &[]);
    let mut c = server.client("a", "agent");
    let cases = [
        (
            "cmd.run",
            json!({ "command": "primitive.sphere", "dryRun": true }),
            "/dryRun",
            "dry_run",
        ),
        (
            "cmd.run",
            json!({ "command": "mesh.stats", "ifRev": 9 }),
            "/ifRev",
            "if_rev",
        ),
        (
            "cmd.run",
            json!({ "command": "primitive.cube", "arg": { "name": "box" } }),
            "/arg",
            "args",
        ),
        (
            "cmd.batch",
            json!({ "commands": [{ "command": "primitive.cube", "arguments": {} }] }),
            "/commands/0/arguments",
            "args",
        ),
        (
            "engine.hello",
            json!({ "client": "a", "rol": "agent" }),
            "/rol",
            "role",
        ),
        (
            "engine.hello",
            json!({ "client": "a", "role": "agnet" }),
            "/role",
            "agent",
        ),
        ("engine.shutdown", json!({ "kee": KEY }), "/kee", "key"),
    ];
    for (method, params, path, guess) in cases {
        let reply = c.rpc(method, params.clone());
        let error = &reply["error"];
        assert_eq!(error["code"], json!(-32602), "{method} {params}: {reply}");
        assert_eq!(error["data"]["path"], json!(path), "{method} {params}");
        assert_eq!(
            error["data"]["suggestions"][0],
            json!(guess),
            "{method} {params}: {reply}"
        );
        let message = error["message"].as_str().unwrap_or_default();
        assert!(
            message.contains(&format!("did you mean '{guess}'")),
            "{message}"
        );
    }
    let reply = c.raw(r#"{"jsonrpc": "2.0", "id": 9, "method": "mesh.stats", "parms": {}}"#);
    assert_eq!(
        reply["error"]["code"],
        json!(-32600),
        "a misspelled request field"
    );
    let reply = c.rpc("catalog.get", json!({ "if_none_match": "x" }));
    assert_eq!(
        reply["error"]["code"],
        json!(-32602),
        "catalog.get takes nothing"
    );
    assert_eq!(
        c.run("mesh.stats", json!({}))["ok"]["result"]["parts"],
        json!([]),
        "nothing ran: no sphere, no cube"
    );
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}

#[test]
fn malformed_requests_get_protocol_errors() {
    let server = Server::start("malformed", &[]);
    let mut c = Client::connect(&server.socket);
    let error = |reply: &Value| (reply["error"]["code"].clone(), reply["id"].clone());
    assert_eq!(
        error(&c.raw("not json")),
        (json!(-32700), Value::Null),
        "parse error"
    );
    assert_eq!(
        error(&c.raw(r#"{"id": 3, "method": "catalog.get"}"#)),
        (json!(-32600), Value::Null),
        "no version"
    );
    assert_eq!(
        error(&c.raw(r#"{"jsonrpc": "1.0", "id": 4, "method": "catalog.get"}"#)),
        (json!(-32600), Value::Null),
        "old version"
    );
    let unknown = c.raw(r#"{"jsonrpc": "2.0", "id": 5, "method": "nope"}"#);
    assert_eq!(error(&unknown), (json!(-32601), json!(5)), "unknown method");
    let message = unknown["error"]["message"].as_str().unwrap_or_default();
    for method in [
        "engine.hello",
        "catalog.get",
        "cmd.run",
        "cmd.batch",
        "engine.shutdown",
    ] {
        assert!(
            message.contains(method),
            "the error lists {method}: {message}"
        );
    }
    let reply =
        c.raw(r#"{"jsonrpc": "2.0", "id": 6, "method": "cmd.run", "params": {"command": 5}}"#);
    assert_eq!(error(&reply), (json!(-32602), json!(6)), "bad params");
    let reply = c.raw(r#"{"jsonrpc": "2.0", "id": 7, "method": "engine.hello", "params": {"client": "x", "protocol": 99}}"#);
    assert_eq!(
        error(&reply),
        (json!(-32602), json!(7)),
        "unknown protocol version"
    );
    assert!(reply["result"].is_null(), "an error has no result");
    let reply = c.raw("\n\n{\"jsonrpc\": \"2.0\", \"id\": 8, \"method\": \"catalog.get\"}");
    assert_eq!(
        (reply["id"].clone(), reply["error"].clone()),
        (json!(8), Value::Null),
        "blank lines are skipped"
    );
    let reply = c.raw(r#"{"jsonrpc": "2.0", "id": null, "method": "catalog.get"}"#);
    assert_eq!(reply["id"], Value::Null, "an explicit null id is answered");
    // m6: a notification (no id) runs but gets no reply; the next answer is for the next line.
    c.send(br#"{"jsonrpc": "2.0", "method": "cmd.run", "params": {"command": "primitive.cube"}}"#);
    let after = c.rpc("cmd.run", json!({ "command": "mesh.stats" }));
    assert_eq!(
        after["result"]["ok"]["result"]["parts"][0]["part"],
        json!("cube"),
        "the notification ran, unanswered"
    );
    // M3: a line that is not UTF-8 gets a parse error, and the connection keeps working.
    c.send(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\xfe\"}");
    assert_eq!(error(&c.read()), (json!(-32700), Value::Null), "not UTF-8");
    assert!(
        c.rpc("catalog.get", json!({})).get("result").is_some(),
        "still served"
    );
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}

/// m7: a request line over the limit gets an error and the connection is closed; the engine
/// keeps serving everyone else.
#[test]
fn a_line_over_the_limit_closes_the_connection() {
    let server = Server::start("limit", &["--max-line-mib", "1"]);
    let mut c = Client::connect(&server.socket);
    let mut line = br#"{"jsonrpc": "2.0", "id": 1, "method": "cmd.run", "params": {"command": "mesh.hash", "args": {"part": ""#.to_vec();
    line.extend(std::iter::repeat_n(b'x', 2 << 20));
    line.extend(br#""}}}"#);
    c.writer.write_all(&line).ok();
    c.writer.write_all(b"\n").ok();
    let reply = c.read();
    assert_eq!(
        reply["error"]["code"],
        json!(-32002),
        "too large: {}",
        reply["error"]
    );
    let refused = &reply["error"]["data"];
    assert_eq!(
        (refused["code"].clone(), refused["hint"].is_string()),
        (json!("E_LIMIT"), true),
        "the E_LIMIT error itself, with a fix: {refused}"
    );
    let mut rest = Vec::new();
    c.reader.read_to_end(&mut rest).ok();
    assert!(rest.is_empty(), "then the connection closes");
    let mut other = server.client("b", "agent");
    assert!(
        other.run("mesh.stats", json!({})).get("ok").is_some(),
        "others are served"
    );
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}

/// A request that comes in two pieces, with a pause longer than a connection waits before it
/// checks whether the session is stopping, is answered whole: what came before the check is kept.
#[test]
fn a_request_split_by_a_pause_is_answered_whole() {
    let server = Server::start("pause", &[]);
    let mut c = server.client("a", "agent");
    let request =
        br#"{"jsonrpc": "2.0", "id": 7, "method": "cmd.run", "params": {"command": "mesh.stats"}}"#;
    let (first, rest) = request.split_at(30);
    c.writer.write_all(first).ok();
    std::thread::sleep(std::time::Duration::from_secs(1));
    c.send(rest);
    let reply = c.read();
    assert_eq!(reply["id"], json!(7), "the request's own answer: {reply}");
    assert!(reply["result"]["ok"].is_object(), "answered whole: {reply}");
    assert_eq!(server.stop(), ExitCode::SUCCESS, "stops");
}
