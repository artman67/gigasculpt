//! One connection's thread: reads request lines, passes them to the engine thread, writes the
//! replies back. Every line gets an answer or closes the connection, so a client never waits on
//! a line the engine dropped.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, Sender};

use gs_protocol::UnixStream;

use super::{Task, rpc};

/// What reading one line found.
enum Line {
    /// The client closed its end.
    End,
    /// The line is longer than the limit.
    TooLong,
    /// A line (without its line break) is in the buffer.
    Read,
}

/// Serves connection `conn` until the client leaves, the line limit is broken, or the engine
/// stops. Always tells the engine the connection is gone.
pub(super) fn serve(
    conn: u64,
    stream: UnixStream,
    mut writer: UnixStream,
    tasks: &Sender<Task>,
    max_line: usize,
) {
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        let reply = match read_line(&mut reader, max_line, &mut buf) {
            Ok(Line::End) | Err(_) => break,
            Ok(Line::TooLong) => {
                send(&mut writer, &rpc::too_large(max_line)).ok();
                break;
            }
            Ok(Line::Read) => match std::str::from_utf8(&buf) {
                Err(_) => Some(rpc::not_utf8()),
                Ok(text) if text.trim().is_empty() => continue,
                Ok(text) => match ask(tasks, conn, text.to_string()) {
                    Ok(reply) => reply,
                    Err(Stopped) => break,
                },
            },
        };
        if let Some(reply) = reply
            && send(&mut writer, &reply).is_err()
        {
            break;
        }
    }
    tasks.send(Task::Gone { conn }).ok();
}

/// The engine has stopped.
struct Stopped;

/// Queues one line for the engine and waits for its answer (none for a notification), which
/// may first wait for a job here.
fn ask(tasks: &Sender<Task>, conn: u64, line: String) -> Result<Option<String>, Stopped> {
    let (reply, answer) = mpsc::channel();
    tasks
        .send(Task::Line { conn, line, reply })
        .map_err(|_| Stopped)?;
    match answer.recv().map_err(|_| Stopped)? {
        rpc::Answer::Now(reply) => Ok(reply),
        rpc::Answer::Wait(waiting) => Ok(waiting.reply()),
    }
}

fn send(writer: &mut UnixStream, reply: &str) -> io::Result<()> {
    writer.write_all(format!("{reply}\n").as_bytes())
}

/// Reads one line of at most `max` bytes into `buf`, without its line break.
fn read_line(reader: &mut impl BufRead, max: usize, buf: &mut Vec<u8>) -> io::Result<Line> {
    buf.clear();
    let limit = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    if Read::take(&mut *reader, limit).read_until(b'\n', buf)? == 0 {
        return Ok(Line::End);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
    } else if buf.len() as u64 == limit {
        return Ok(Line::TooLong);
    }
    Ok(Line::Read)
}
