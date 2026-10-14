//! Roles over the socket: the human is the window, in-process; agents wait for it, observers
//! only look, and nothing on the socket can become the human.

use serde_json::{Value, json};

use crate::socket::{Client, stopped, with_window};

/// Runs a command as the human at the window, which must succeed.
fn as_window(window: &gs_engine::server::Window, command: &str, args: Value) {
    let request = gs_engine::Request::Run(gs_protocol::RunParams::new(command, args));
    let response = window
        .execute(request)
        .unwrap_or_else(|| panic!("the engine stopped"));
    assert!(
        matches!(response.outcome, gs_protocol::Outcome::Ok(_)),
        "the window's {command} works: {response:?}"
    );
}

/// The human is the window, in-process: it pauses agents and marks gestures; agents wait in
/// every way in (single commands, batches, undo), and observers only look.
#[test]
fn the_window_pauses_agents_and_observers_only_look() {
    let (server, socket, _leftovers) = with_window("roles");
    let window = server.window();
    let (mut agent, mut observer) = (
        Client::hello(&socket, "bot", "agent"),
        Client::hello(&socket, "eye", "observer"),
    );
    assert_eq!(
        observer.error_code("primitive.cube", json!({})),
        json!("E_PRECONDITION"),
        "observers cannot edit"
    );
    assert!(
        observer.run("mesh.stats", json!({})).get("ok").is_some(),
        "but can look"
    );
    assert_eq!(
        agent.error_code("session.control", json!({ "paused": true })),
        json!("E_PRECONDITION"),
        "only the human pauses"
    );
    as_window(&window, "session.control", json!({ "paused": true }));
    let paused = agent.run("primitive.cube", json!({}));
    assert_eq!(paused["error"]["code"], json!("E_PAUSED"), "agents wait");
    assert_eq!(paused["error"]["retryable"], json!(true), "and may retry");
    let batch = agent.rpc(
        "cmd.batch",
        json!({ "commands": [{ "command": "primitive.cube" }] }),
    )["result"]
        .clone();
    assert_eq!(
        batch["error"]["code"],
        json!("E_PAUSED"),
        "batches wait too"
    );
    assert_eq!(
        agent.error_code("history.undo", json!({})),
        json!("E_PAUSED"),
        "and undo"
    );
    assert!(
        agent.run("mesh.stats", json!({})).get("ok").is_some(),
        "queries still work"
    );
    as_window(&window, "primitive.cube", json!({}));
    as_window(
        &window,
        "session.control",
        json!({ "paused": false, "gesture": true }),
    );
    let busy = agent.run("primitive.sphere", json!({}));
    assert_eq!(
        (
            busy["error"]["code"].clone(),
            busy["error"]["retryable"].clone()
        ),
        (json!("E_BUSY"), json!(true)),
        "mid-stroke"
    );
    as_window(&window, "session.control", json!({ "gesture": false }));
    assert!(
        agent.run("primitive.sphere", json!({})).get("ok").is_some(),
        "free again"
    );
    window.close();
    stopped(server);
    assert!(!socket.exists(), "the socket is gone");
}

/// M2: nothing on the socket can become the human, change its role, or close the window's
/// session.
#[test]
fn the_socket_never_hands_out_the_human_role() {
    let (server, socket, _leftovers) = with_window("no-human");
    let (mut agent, mut observer) = (
        Client::hello(&socket, "bot", "agent"),
        Client::hello(&socket, "eye", "observer"),
    );
    let refused =
        Client::connect(&socket).rpc("engine.hello", json!({ "client": "x", "role": "human" }));
    assert_eq!(
        refused["error"]["code"],
        json!(-32001),
        "the socket never grants human"
    );
    let refused = agent.rpc("engine.hello", json!({ "client": "bot", "role": "human" }));
    assert_eq!(
        refused["error"]["code"],
        json!(-32001),
        "not on a second hello either"
    );
    let refused = observer.rpc("engine.hello", json!({ "client": "eye", "role": "agent" }));
    assert_eq!(
        refused["error"]["code"],
        json!(-32001),
        "a role cannot change"
    );
    let message = refused["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("already an observer"),
        "says which: {message}"
    );
    assert_eq!(
        observer.error_code("primitive.sphere", json!({})),
        json!("E_PRECONDITION"),
        "still an observer"
    );
    let again = agent.rpc("engine.hello", json!({ "client": "bot", "role": "agent" }));
    assert!(
        again.get("result").is_some(),
        "the same role again is fine: {again}"
    );
    for client in [&mut agent, &mut observer] {
        let refused = client.rpc("engine.shutdown", json!({}));
        assert_eq!(
            refused["error"]["code"],
            json!(-32001),
            "only the window closes it"
        );
    }
    assert!(
        agent.run("mesh.stats", json!({})).get("ok").is_some(),
        "still running"
    );
    server.window().close();
    stopped(server);
}
