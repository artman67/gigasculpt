//! JSON-RPC dispatch: one request line in, at most one response line out.
//!
//! Parameters refuse unknown fields, with the field's JSON pointer and a did-you-mean in the
//! error, so a misspelled `dry_run` never runs an edit for real.

use std::sync::Arc;
use std::time::Duration;

use gs_protocol::{
    CommandError, ErrorCode, HelloParams, JobParams, JobReport, PROTOCOL_VERSION, Request, Role,
    RpcError, RpcRequest, RpcResponse, ShutdownParams, StartParams, method, rpc_code,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::errors::{error, explain_decode};
use crate::job::{Caller, JobCell, Start, Work};
use crate::{Engine, Origin};

/// One socket client, as the engine thread knows it.
pub(super) struct Client {
    origin: Origin,
    /// True once `engine.hello` set the client's role, which then cannot change.
    greeted: bool,
}

impl Client {
    /// Connection `conn` before its hello: an agent called `client`.
    pub fn new(conn: u64) -> Self {
        Self {
            origin: Origin {
                client_id: conn,
                client: "client".into(),
                role: Role::Agent,
            },
            greeted: false,
        }
    }
}

/// What the engine thread made of one request line.
pub(super) struct Handled {
    pub answer: Answer,
    /// A job to start on its own thread.
    pub work: Option<Box<Work>>,
    /// The engine should stop.
    pub stop: bool,
}

/// The answer to a request line.
pub(super) enum Answer {
    /// The reply line (none for a notification).
    Now(Option<String>),
    /// A reply that first waits for a job, on the connection's thread.
    Wait(Waiting),
}

/// A reply that waits for a job.
pub(super) struct Waiting {
    /// The request's id; `None` for a notification, which gets no reply.
    id: Option<Value>,
    cell: Arc<JobCell>,
    /// How long to wait; `None`: until the job ends.
    limit: Option<Duration>,
    /// Reply with the job's report (`job.*`), or with its response alone (`cmd.*`).
    report: bool,
}

impl Waiting {
    /// Waits, then gives the reply line.
    pub fn reply(self) -> Option<String> {
        let report = self.cell.wait(self.limit);
        let result = if self.report {
            json!(report)
        } else {
            json!(report.response)
        };
        self.id.map(|id| respond(id, Ok(result)))
    }
}

/// What one method gives back: a result now, or a job to wait for.
enum Reply {
    Value(Value),
    Job {
        cell: Arc<JobCell>,
        limit: Option<Duration>,
        report: bool,
        work: Option<Box<Work>>,
    },
}

/// Answers one request line.
pub(super) fn handle(
    engine: &mut Engine,
    client: &mut Client,
    line: &str,
    stop_key: Option<&str>,
) -> Handled {
    let request = match parse_request(line) {
        Ok(request) => request,
        Err(err) => {
            let answer = Answer::Now(Some(respond(Value::Null, Err(err))));
            return Handled {
                answer,
                work: None,
                stop: false,
            };
        }
    };
    let params = request.params;
    let reply = match request.method.as_str() {
        method::HELLO => hello(engine, client, params).map(Reply::Value),
        method::CATALOG => {
            no_params(params, method::CATALOG).map(|()| Reply::Value(json!(engine.catalog())))
        }
        method::RUN => decode(params, method::RUN)
            .map(|p| start(engine, &client.origin, Request::Run(p), None)),
        method::BATCH => decode(params, method::BATCH)
            .map(|p| start(engine, &client.origin, Request::Batch(p), None)),
        method::JOB_START => decode(params, method::JOB_START).and_then(|p: StartParams| {
            let wait = Some(p.wait_ms);
            let request = p.request();
            request
                .map(|r| start(engine, &client.origin, r, wait))
                .map_err(|e| rpc_error(rpc_code::INVALID_PARAMS, format!("job.start: {e}")))
        }),
        method::JOB_STATUS => {
            decode(params, method::JOB_STATUS).and_then(|p| status(engine, &client.origin, &p))
        }
        method::SHUTDOWN => shutdown(&client.origin, params, stop_key).map(Reply::Value),
        other => Err(rpc_error(
            rpc_code::METHOD_NOT_FOUND,
            format!(
                "unknown method '{other}'; the methods are {}",
                [
                    method::HELLO,
                    method::CATALOG,
                    method::RUN,
                    method::BATCH,
                    method::JOB_START,
                    method::JOB_STATUS,
                    method::SHUTDOWN
                ]
                .join(", ")
            ),
        )),
    };
    let stop = request.method == method::SHUTDOWN && reply.is_ok();
    let (answer, work) = match reply {
        Ok(Reply::Job {
            cell,
            limit,
            report,
            work,
        }) => {
            let id = request.id;
            let waiting = Waiting {
                id,
                cell,
                limit,
                report,
            };
            (Answer::Wait(waiting), work)
        }
        Ok(Reply::Value(value)) => (
            Answer::Now(request.id.map(|id| respond(id, Ok(value)))),
            None,
        ),
        Err(err) => (
            Answer::Now(request.id.map(|id| respond(id, Err(err)))),
            None,
        ),
    };
    Handled { answer, work, stop }
}

/// Runs a call. With `wait` (`job.start`), the answer is a job report after waiting at most
/// that long; without (`cmd.run`, `cmd.batch`), it is the call's response, once it has ended.
fn start(engine: &mut Engine, origin: &Origin, request: Request, wait: Option<u64>) -> Reply {
    let name = request.name().to_string();
    match engine.start(origin, request, Caller::Socket) {
        Start::Now(response) if wait.is_some() => Reply::Value(json!(JobReport {
            id: None,
            command: name,
            progress: None,
            message: None,
            working: false,
            response: Some(response),
        })),
        Start::Now(response) => Reply::Value(json!(response)),
        Start::Job(work) => Reply::Job {
            cell: work.cell(),
            limit: wait.map(Duration::from_millis),
            report: wait.is_some(),
            work: Some(work),
        },
    }
}

/// `job.status`: the job's report after waiting at most `wait_ms`; cancelled first if asked
/// and the client may.
/// An unknown id is refused with the `E_NOT_FOUND` command error as the error's data.
fn status(engine: &mut Engine, origin: &Origin, params: &JobParams) -> Result<Reply, RpcError> {
    match engine.job(params.id, params.cancel, origin) {
        Ok(cell) => Ok(Reply::Job {
            cell,
            limit: Some(Duration::from_millis(params.wait_ms)),
            report: true,
            work: None,
        }),
        Err(err) => Err(RpcError {
            data: serde_json::to_value(&err).ok(),
            ..rpc_error(rpc_code::INVALID_PARAMS, err.message.clone())
        }),
    }
}

fn parse_request(line: &str) -> Result<RpcRequest, RpcError> {
    let value: Value = serde_json::from_str(line)
        .map_err(|e| rpc_error(rpc_code::PARSE_ERROR, format!("not valid JSON: {e}")))?;
    let request: RpcRequest = decode_as(value, rpc_code::INVALID_REQUEST, "request")?;
    if request.jsonrpc != "2.0" {
        let message = "jsonrpc must be \"2.0\"".to_string();
        return Err(rpc_error(rpc_code::INVALID_REQUEST, message));
    }
    Ok(request)
}

/// Names the client and its role, and starts counting other clients' changes for it. The
/// socket never grants the human role, and a connection keeps the role it started with.
fn hello(engine: &mut Engine, client: &mut Client, raw: Value) -> Result<Value, RpcError> {
    let hello: HelloParams = decode(raw, method::HELLO)?;
    if let Some(protocol) = hello.protocol
        && protocol != PROTOCOL_VERSION
    {
        let message = format!("this engine speaks protocol {PROTOCOL_VERSION}, not {protocol}");
        return Err(rpc_error(rpc_code::INVALID_PARAMS, message));
    }
    if hello.role == Role::Human {
        return Err(refused(
            "the human role belongs to the window inside the engine; connect as agent or observer",
        ));
    }
    if client.greeted && hello.role != client.origin.role {
        return Err(refused(&format!(
            "this connection is already an {}; a connection keeps the role it started with",
            role_name(client.origin.role)
        )));
    }
    client.origin.client = hello.client;
    client.origin.role = hello.role;
    client.greeted = true;
    Ok(json!(engine.hello(&client.origin)))
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Human => "human",
        Role::Agent => "agent",
        Role::Observer => "observer",
    }
}

/// Stops the session if this client may: the human always, an agent only with the session's
/// stop key (proof that it spawned the session), an observer never.
fn shutdown(origin: &Origin, raw: Value, key: Option<&str>) -> Result<Value, RpcError> {
    let params: ShutdownParams = decode(raw, method::SHUTDOWN)?;
    let allowed = match origin.role {
        Role::Human => Ok(()),
        Role::Observer => Err("an observer cannot stop the session"),
        Role::Agent => match key {
            None => Err("this session has no stop key, so only whoever started it can stop it"),
            Some(key) if params.key.as_deref() == Some(key) => Ok(()),
            Some(_) => Err("only the client that spawned this session can stop it"),
        },
    };
    allowed.map_err(refused)?;
    Ok(json!({ "stopping": true }))
}

/// Methods without parameters take none, or an empty object.
fn no_params(raw: Value, what: &str) -> Result<(), RpcError> {
    match raw {
        Value::Null => Ok(()),
        Value::Object(map) if map.is_empty() => Ok(()),
        _ => Err(rpc_error(
            rpc_code::INVALID_PARAMS,
            format!("{what} takes no parameters"),
        )),
    }
}

/// A method's parameters. Missing parameters count as `{}`.
fn decode<T: DeserializeOwned>(raw: Value, what: &str) -> Result<T, RpcError> {
    let raw = if raw.is_null() { json!({}) } else { raw };
    decode_as(raw, rpc_code::INVALID_PARAMS, what)
}

/// Decodes a wire value; on failure, an error with `code` naming the bad field's JSON pointer
/// and, for an unknown field, the closest known names.
fn decode_as<T: DeserializeOwned>(raw: Value, code: i64, what: &str) -> Result<T, RpcError> {
    serde_path_to_error::deserialize(raw).map_err(|e| {
        let found = explain_decode(&e);
        let at = if found.path.is_empty() {
            String::new()
        } else {
            format!(" at {}", found.path)
        };
        let guess = found
            .suggestions
            .first()
            .map(|best| format!("; did you mean '{best}'?"))
            .unwrap_or_default();
        let message = format!("{what}{at}: {}{guess}", found.message);
        RpcError {
            code,
            message,
            data: Some(json!({ "path": found.path, "suggestions": found.suggestions })),
        }
    })
}

fn rpc_error(code: i64, message: String) -> RpcError {
    RpcError {
        code,
        message,
        data: None,
    }
}

fn refused(message: &str) -> RpcError {
    rpc_error(rpc_code::REFUSED, message.to_string())
}

/// The answer to a line that is not UTF-8.
pub(super) fn not_utf8() -> String {
    let err = rpc_error(
        rpc_code::PARSE_ERROR,
        "the request is not valid UTF-8".into(),
    );
    respond(Value::Null, Err(err))
}

/// The answer to a line over the limit, sent just before the connection closes. Its `data` is
/// the `E_LIMIT` command error, so clients get the stable code as for any refused command.
pub(super) fn too_large(max_line: usize) -> String {
    let limit = CommandError {
        hint: Some("send big meshes in smaller parts".into()),
        ..error(
            ErrorCode::Limit,
            format!("the request is longer than {max_line} bytes, so the connection is closed"),
        )
    };
    let err = RpcError {
        data: serde_json::to_value(&limit).ok(),
        ..rpc_error(rpc_code::TOO_LARGE, limit.message.clone())
    };
    respond(Value::Null, Err(err))
}

fn respond(id: Value, result: Result<Value, RpcError>) -> String {
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(err) => (None, Some(err)),
    };
    let response = RpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result,
        error,
    };
    serde_json::to_string(&response).unwrap_or_default()
}
