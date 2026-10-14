//! The session server: a Unix socket speaking JSON-RPC 2.0, one message per line, plus the
//! in-process door the window uses.
//!
//! Plain threads, no async runtime: one engine thread owns the engine and takes tasks from a
//! single queue in arrival order. The log is in commit order; a job takes its sequence numbers
//! when it is admitted and is logged when it lands, so a sequence number is a label, not a
//! place in the log (review 0002 m5).
//! Each connection gets a thread that forwards its lines and writes back the replies, so one
//! client's requests are answered in the order sent. A connection's thread closes it when the
//! client leaves, so nothing is held per connection after it ends.
//!
//! Calls with a long command run as jobs on threads of their own (see `job`); a reply that
//! waits for a job waits on its connection's thread, so the engine thread never blocks. The
//! window's saves and opens are jobs too: it gets the job's id at once (decision 0016). The
//! engine thread wakes at least once a second to check the autosave triggers.
//!
//! The socket only ever grants the `agent` and `observer` roles. The human is the window, in
//! the same process: it sends its requests through a `Window` handle.

mod conn;
mod rpc;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::net::Shutdown;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gs_protocol::{CmdResponse, Request, Role, UnixListener, UnixStream};

pub use crate::job::JobWatch;
use crate::job::{self, Caller, Start};
use crate::{Engine, EngineConfig, Origin};

/// How often the engine thread checks the autosave triggers when nothing else wakes it.
const TICK: Duration = Duration::from_secs(1);

/// How long a write to a client may block before the connection is dropped.
const WRITE_PATIENCE: Duration = Duration::from_secs(30);

/// How to run a session server.
#[derive(Clone, Debug)]
pub struct ServeOptions {
    /// The socket to listen on.
    pub socket: PathBuf,
    /// The key an agent's `engine.shutdown` must carry. `None`: no agent can stop the session.
    pub stop_key: Option<String>,
    /// The longest request line accepted, in bytes. A longer line gets an error and the
    /// connection is closed.
    pub max_line_bytes: usize,
}

/// Work for the engine thread.
enum Task {
    /// A request line from socket connection `conn`; the answer goes back on `reply`.
    Line {
        conn: u64,
        line: String,
        reply: Sender<rpc::Answer>,
    },
    /// Connection `conn` closed.
    Gone { conn: u64 },
    /// A request from the window.
    Window {
        request: Request,
        reply: Sender<CmdResponse>,
    },
    /// The window asks for one of its jobs, cancelled first with `cancel`, as `job.status` does.
    Job {
        id: u64,
        cancel: bool,
        reply: Sender<Option<JobWatch>>,
    },
    /// A job's thread handed its work back.
    Finished(Box<job::Done>),
    /// A Live Boolean run handed its work back.
    Live,
    /// The window closes the session.
    Stop,
}

/// One open connection: a handle to end it at shutdown, and its thread once started.
struct Open {
    stream: UnixStream,
    thread: Option<JoinHandle<()>>,
}

/// What the threads of one server share.
struct Shared {
    stop: AtomicBool,
    socket: PathBuf,
    /// Every open connection. Each connection's thread removes its own entry when it ends, so
    /// nothing is held for a connection that closed.
    live: Mutex<BTreeMap<u64, Open>>,
}

impl Shared {
    fn live(&self) -> MutexGuard<'_, BTreeMap<u64, Open>> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Stops accepting, and ends every open connection once its pending reply is written.
    fn stopping(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        UnixStream::connect(&self.socket).ok();
        for open in self.live().values() {
            open.stream.shutdown(Shutdown::Read).ok();
        }
    }
}

/// A running session server.
pub struct Server {
    tasks: Sender<Task>,
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
    /// Held while the server runs, so a second server for the same socket refuses to start.
    _lock: File,
}

/// The human at the window: sends requests into the engine's queue from inside the process.
#[derive(Clone)]
pub struct Window {
    tasks: Sender<Task>,
}

impl Window {
    /// Runs a request as the human. `None` if the engine has stopped.
    pub fn execute(&self, request: Request) -> Option<CmdResponse> {
        let (reply, answer) = mpsc::channel();
        self.tasks.send(Task::Window { request, reply }).ok()?;
        answer.recv().ok()
    }

    /// Job `id`, which a long call of the window's answered with, to watch for the status
    /// line's bar; cancelled first with `cancel`, as `job.status` does (decision 0010 as
    /// amended 2026-10-13). `None` if there is no such job or the engine has stopped.
    pub fn job(&self, id: u64, cancel: bool) -> Option<JobWatch> {
        let (reply, answer) = mpsc::channel();
        self.tasks.send(Task::Job { id, cancel, reply }).ok()?;
        answer.recv().ok().flatten()
    }

    /// Closes the session, as closing the window does.
    pub fn close(&self) {
        self.tasks.send(Task::Stop).ok();
    }
}

impl Server {
    /// Starts serving `options.socket`. Refuses if another server holds the socket; replaces a
    /// stale socket file left by a crash.
    pub fn start(config: EngineConfig, options: &ServeOptions) -> io::Result<Self> {
        Self::start_with(config, options, |_| {})
    }

    /// Like [`Server::start`], and lets the caller set up the engine (the window attaches
    /// itself) before it answers anyone.
    pub fn start_with(
        config: EngineConfig,
        options: &ServeOptions,
        prepare: impl FnOnce(&mut Engine),
    ) -> io::Result<Self> {
        let socket = options.socket.clone();
        if let Some(dir) = socket.parent() {
            private_folder(dir)?;
        }
        let lock = File::create(socket.with_extension("lock"))?;
        if lock.try_lock().is_err() {
            let message = format!(
                "session '{}' is already running at {}",
                config.session,
                socket.display()
            );
            return Err(io::Error::new(io::ErrorKind::AddrInUse, message));
        }
        if fs::symlink_metadata(&socket).is_ok() {
            fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        let mut engine = Engine::new(config)?;
        prepare(&mut engine);
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            socket,
            live: Mutex::new(BTreeMap::new()),
        });
        let (tasks, queue) = mpsc::channel();
        let engine_thread = {
            let (shared, tasks) = (Arc::clone(&shared), tasks.clone());
            let key = options.stop_key.clone();
            thread::spawn(move || run_engine(engine, &queue, &tasks, &shared, key.as_deref()))
        };
        let accept_thread = {
            let (shared, tasks) = (Arc::clone(&shared), tasks.clone());
            let max_line = options.max_line_bytes;
            thread::spawn(move || accept(&listener, &tasks, &shared, max_line))
        };
        Ok(Self {
            tasks,
            shared,
            threads: vec![engine_thread, accept_thread],
            _lock: lock,
        })
    }

    /// The window's door into this session.
    pub fn window(&self) -> Window {
        Window {
            tasks: self.tasks.clone(),
        }
    }

    /// Waits until the session stops, lets open connections write their last reply, and
    /// removes the socket.
    pub fn wait(self) -> io::Result<()> {
        drop(self.tasks);
        for thread in self.threads {
            thread.join().ok();
        }
        // A connection accepted just before the stop may have missed `stopping`.
        self.shared.stopping();
        // Each connection's thread ends once it has written its last reply.
        let threads: Vec<JoinHandle<()>> = self
            .shared
            .live()
            .values_mut()
            .filter_map(|open| open.thread.take())
            .collect();
        for thread in threads {
            thread.join().ok();
        }
        fs::remove_file(&self.shared.socket)
    }
}

/// Makes the folder `dir` and its parents. On Unix only their owner may use the folders it makes
/// (mode `0700`); on Windows the session folder is private because `%LOCALAPPDATA%` is.
fn private_folder(dir: &Path) -> io::Result<()> {
    let mut folder = fs::DirBuilder::new();
    folder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
    folder.create(dir)
}

/// Runs a session until a client or the window stops it.
pub fn serve(config: EngineConfig, options: &ServeOptions) -> io::Result<()> {
    Server::start(config, options)?.wait()
}

/// The engine thread: answers every queued task in order until shutdown. Jobs run on threads
/// of their own and come back through the queue; a commit the session policy holds back is
/// tried again after every task. When the session stops, every job still running ends.
fn run_engine(
    mut engine: Engine,
    queue: &Receiver<Task>,
    tasks: &Sender<Task>,
    shared: &Shared,
    key: Option<&str>,
) {
    let window = Origin {
        client_id: 0,
        client: "window".into(),
        role: Role::Human,
    };
    let mut clients: BTreeMap<u64, rpc::Client> = BTreeMap::new();
    let mut held: Vec<job::Done> = Vec::new();
    let wake = tasks.clone();
    engine.live.wake_with(move || drop(wake.send(Task::Live)));
    loop {
        let task = match queue.recv_timeout(TICK) {
            Ok(task) => task,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(work) = engine.autosave_due() {
                    spawn(&mut engine, *work, tasks);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        match task {
            Task::Line { conn, line, reply } => {
                let client = clients
                    .entry(conn)
                    .or_insert_with(|| rpc::Client::new(conn));
                let handled = rpc::handle(&mut engine, client, &line, key);
                if let Some(work) = handled.work {
                    spawn(&mut engine, *work, tasks);
                }
                reply.send(handled.answer).ok();
                if handled.stop {
                    break;
                }
            }
            Task::Gone { conn } => {
                clients.remove(&conn);
                engine.goodbye(conn);
            }
            Task::Window { request, reply } => match engine.start(&window, request, Caller::Window)
            {
                Start::Now(response) => {
                    reply.send(response).ok();
                }
                Start::Job(work) => {
                    reply.send(engine.started(&work)).ok();
                    spawn(&mut engine, *work, tasks);
                }
            },
            Task::Job { id, cancel, reply } => {
                let cell = engine.job(id, cancel, &window).ok();
                reply.send(cell.map(JobWatch::new)).ok();
            }
            Task::Finished(done) => held.push(*done),
            Task::Live => engine.land_live(),
            Task::Stop => break,
        }
        held = held.into_iter().filter_map(|d| engine.finish(d)).collect();
        if let Some(work) = engine.autosave_due() {
            spawn(&mut engine, *work, tasks);
        }
    }
    engine.drop_jobs();
    engine.close();
    shared.stopping();
}

/// Starts a job's thread, which hands the work back through the queue.
fn spawn(engine: &mut Engine, work: job::Work, tasks: &Sender<Task>) {
    let (cell, tasks) = (work.cell(), tasks.clone());
    let started = thread::Builder::new()
        .name(format!("job {}", cell.id))
        .spawn(move || tasks.send(Task::Finished(Box::new(work.run()))).ok());
    if started.is_err() {
        engine.drop_job(&cell, "its thread could not start");
    }
}

/// The accept loop: one thread per connection until the stop flag is set.
fn accept(listener: &UnixListener, tasks: &Sender<Task>, shared: &Arc<Shared>, max_line: usize) {
    for (conn, stream) in (1..).zip(listener.incoming()) {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        match stream {
            Ok(stream) => open(conn, stream, tasks, shared, max_line),
            // Out of file descriptors, most likely: give the open connections a moment.
            Err(_) => thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Starts connection `conn`'s thread. If that fails, the connection is dropped, so the client
/// sees it close instead of waiting forever.
fn open(
    conn: u64,
    stream: UnixStream,
    tasks: &Sender<Task>,
    shared: &Arc<Shared>,
    max_line: usize,
) {
    stream.set_write_timeout(Some(WRITE_PATIENCE)).ok();
    let (Ok(writer), Ok(closer)) = (stream.try_clone(), stream.try_clone()) else {
        return;
    };
    let open = Open {
        stream: closer,
        thread: None,
    };
    shared.live().insert(conn, open);
    let (tasks, done) = (tasks.clone(), Arc::clone(shared));
    let started = thread::Builder::new().spawn(move || {
        conn::serve(conn, stream, writer, &tasks, max_line);
        done.live().remove(&conn);
    });
    let mut live = shared.live();
    match (started, live.get_mut(&conn)) {
        (Ok(thread), Some(open)) => open.thread = Some(thread),
        // The thread already ended and removed its entry: nothing to wait for.
        (Ok(_), None) => {}
        (Err(_), _) => drop(live.remove(&conn)),
    }
}
