//! Shared value types, wire messages, catalog format, log format and error codes.
//! The only crate the thin `gs` client links.
//!
//! This crate is mostly data: types, their serde form and their JSON Schema, plus the few rules
//! both sides must share (where session sockets live, what a session may be called, how a log
//! line turns back into a request). Behavior that uses these types lives with the code that needs
//! it (the engine or the client), so each piece of logic has one home and is tested where it runs.
//!
//! `PlainMesh` is the interchange mesh, not the engine's internal storage; see its docs.

mod catalog;
mod error;
mod mesh;
mod paths;
mod pen;
mod repro;
mod script;
mod socket;
mod values;
mod wire;

pub use catalog::{Catalog, CommandInfo, Cost, ExampleInfo, Kind, Step};
pub use error::{CommandError, ErrorCode, Violation};
pub use mesh::{
    AttrDomain, AttrValues, Attribute, COLOR, GROUP, HIDE, MASK, NO_VERTEX, PlainMesh, arity,
    corners, face_triangles, quad_diagonal, quad_triangles,
};
pub use paths::{
    Env, System, check_session_name, free_path, session_dir, session_dir_in, socket_path,
    state_dir, state_dir_in,
};
pub use pen::{PenSample, PenTool};
pub use repro::{
    BUNDLE_FORMAT, Bundle, BundleInfo, HANG_SECONDS, PANICKED, Predicate, Source, Start,
    bundle_file, panic_message,
};
pub use script::{LOG_FORMAT, LogHeader, LogRecord, ScriptLine};
pub use socket::{UnixListener, UnixStream};
pub use values::{Length, PartRef, Vec3};
pub use wire::{
    BatchParams, CmdResponse, Effect, HelloParams, HelloResult, JobParams, JobReport, Outcome,
    PROTOCOL_VERSION, Request, Role, RpcError, RpcRequest, RpcResponse, RunParams, ShutdownParams,
    StartParams, Success, method, rpc_code,
};
