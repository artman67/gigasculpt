//! One JSON-RPC connection to a session engine. Connecting and every reply have a time limit,
//! so a stuck engine gives exit code 5 instead of a hang.

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gs_protocol::{
    Catalog, HelloParams, PROTOCOL_VERSION, Request, Role, RpcRequest, RpcResponse, RunParams,
    UnixStream, method, rpc_code,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::calls::{self, Ended};
use crate::failure::Failure;
use crate::output::{self, Surface};

pub struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    patience: Duration,
}

impl Connection {
    /// Connects to a socket and introduces this client, waiting at most `patience` for each
    /// step.
    pub fn open(socket: &Path, role: Role, patience: Duration) -> Result<Self, Failure> {
        let writer = connect(socket, patience)?;
        let unreachable =
            |e: io::Error| Failure::Unreachable(format!("cannot reach {}: {e}", socket.display()));
        writer
            .set_read_timeout(Some(patience))
            .map_err(unreachable)?;
        let reader = BufReader::new(writer.try_clone().map_err(unreachable)?);
        let mut connection = Self {
            reader,
            writer,
            patience,
        };
        let hello = HelloParams {
            client: client_name(),
            role,
            protocol: Some(PROTOCOL_VERSION),
        };
        connection.call::<Value>(method::HELLO, json!(hello))?;
        Ok(connection)
    }

    /// Sends one request and waits for its response. Requests go one at a time, so every
    /// request can use the same id.
    pub fn call<T: DeserializeOwned>(&mut self, method: &str, params: Value) -> Result<T, Failure> {
        let request = RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        };
        let mut line =
            serde_json::to_string(&request).map_err(|e| Failure::Protocol(e.to_string()))?;
        line.push('\n');
        let lost = |e: io::Error| Failure::Unreachable(format!("lost the session: {e}"));
        // A session that refuses a line (one over its limit) answers and closes while the rest
        // is still being written: read its answer before calling the session lost.
        let written = self.writer.write_all(line.as_bytes());
        let mut reply = String::new();
        match (self.reader.read_line(&mut reply), written) {
            (Ok(n), _) if n > 0 => {}
            (Ok(_), Ok(())) => {
                return Err(Failure::Unreachable(
                    "the session closed the connection".into(),
                ));
            }
            (Err(e), Ok(()))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(Failure::Timeout(format!(
                    "the session did not answer {method} within {} ms; it may be stuck or the \
                     command slow (raise --timeout-ms)",
                    self.patience.as_millis()
                )));
            }
            (_, Err(e)) | (Err(e), Ok(())) => return Err(lost(e)),
        }
        let response: RpcResponse =
            serde_json::from_str(&reply).map_err(|e| Failure::Protocol(e.to_string()))?;
        if let Some(err) = response.error {
            // A refusal that carries a command error (a line over the limit) keeps its code.
            if let Some(refused) = err
                .data
                .clone()
                .and_then(|d| serde_json::from_value(d).ok())
            {
                return Err(Failure::Command(Box::new(refused)));
            }
            let text = format!("{} (code {})", err.message, err.code);
            return Err(match err.code {
                rpc_code::REFUSED | rpc_code::TOO_LARGE => Failure::Refused(text),
                _ => Failure::Protocol(text),
            });
        }
        serde_json::from_value(response.result.unwrap_or_default())
            .map_err(|e| Failure::Protocol(e.to_string()))
    }

    /// The command catalog.
    pub fn catalog(&mut self) -> Result<Catalog, Failure> {
        self.call(method::CATALOG, json!({}))
    }

    /// Runs one command and returns the response exactly as the engine sent it.
    pub fn run(&mut self, command: &str, args: Value) -> Result<Value, Failure> {
        self.send(one(command, args))
    }

    /// Runs one command or a batch, waiting for it as long as this connection's patience, and
    /// returns the response as sent. A job still running then is a timeout that names it.
    pub fn send(&mut self, request: Request) -> Result<Value, Failure> {
        let patience = self.patience;
        match calls::run(self, request, patience, &mut |_| {})? {
            Ended::Done(response) => Ok(response),
            Ended::Running(report) => {
                let lines = output::running(&report, Surface::Cli);
                Err(Failure::Timeout(lines.join("; ")))
            }
        }
    }
}

/// A request to run one command.
pub fn one(command: &str, args: Value) -> Request {
    Request::Run(RunParams::new(command, args))
}

/// This client's name: `$GS_CLIENT`, else `gs`. The history attributes steps to it, so two
/// agents with their own names each undo only their own steps.
fn client_name() -> String {
    let named = std::env::var("GS_CLIENT").ok();
    named
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "gs".into())
}

/// Connects to `socket`, giving up after `patience` (a full listen queue makes `connect` wait).
fn connect(socket: &Path, patience: Duration) -> Result<UnixStream, Failure> {
    let (done, result) = mpsc::channel();
    let path: PathBuf = socket.to_path_buf();
    thread::spawn(move || done.send(UnixStream::connect(path)).ok());
    match result.recv_timeout(patience) {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(e)) => Err(Failure::Unreachable(format!(
            "cannot reach {}: {e}",
            socket.display()
        ))),
        Err(_) => Err(Failure::Timeout(format!(
            "could not connect to {} within {} ms",
            socket.display(),
            patience.as_millis()
        ))),
    }
}
