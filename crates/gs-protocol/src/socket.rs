//! The session socket's two types, the same on every system: a Unix domain socket, so a session
//! is `<name>.sock` in the session folder everywhere, `gs` finds sessions by listing that
//! folder, and the wire is unchanged (JSON-RPC, one message per line).
//!
//! Linux and macOS use the standard library's types. Windows has had `AF_UNIX` sockets since
//! Windows 10 version 1803; `uds_windows` (MIT) reaches them through types with the standard
//! library's names and methods (`connect`, `bind`, `incoming`, `accept`, `try_clone`, the
//! timeouts, `shutdown`), so every user writes the same code. Named pipes (`interprocess`) would
//! have needed names of their own outside the session folder, another way to list sessions, and
//! a stream with no `try_clone`.
//!
//! What differs between systems: a socket's path must fit in 107 bytes on Linux and Windows and
//! 103 on macOS; a server makes its session folder private with mode `0700` on Unix, while on
//! Windows the folder is private because `%LOCALAPPDATA%` is. A socket file a crash left behind
//! is removed before the next bind everywhere.

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
pub use uds_windows::{UnixListener, UnixStream};

use crate::paths::System;

/// The longest socket path on `system`, in bytes: its `sun_path` less the closing NUL.
pub(crate) fn longest_path(system: System) -> usize {
    if system == System::MacOs { 103 } else { 107 }
}
