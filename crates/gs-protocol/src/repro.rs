//! Bug bundles (decision 0031): a `NAME.gsrepro/` folder holding what makes a bug happen again.
//!
//! - `bundle.json`: [`BundleInfo`], the header and the predicate that says what "broken" means;
//! - `start.gss`: the scene the commands start from, unless that is the empty scene;
//! - `commands.jsonl`: the resolved command lines, as the command log wrote them (`LogRecord`,
//!   one per line, no header), so `gs run` can replay them as a script;
//! - `files/`: a copy of each file a line reads (`file.import`, `reference.open`, ...), the
//!   line's path rewritten to the copy, relative to the bundle folder, which a replay runs in;
//! - `screenshot.png`: what the window showed, when there was a window;
//! - `log-tail.json`: the last lines of `log.tail`, for a reader.
//!
//! The engine writes bundles; the `gs` client replays, minimizes and checks them.

use std::fs;
use std::io;
use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ErrorCode;

/// The bundle format version this crate writes and reads.
pub const BUNDLE_FORMAT: u32 = 1;

/// The files of a bundle folder.
pub mod bundle_file {
    /// The header and predicate.
    pub const INFO: &str = "bundle.json";
    /// The start scene, when there is one.
    pub const START: &str = "start.gss";
    /// The command lines.
    pub const COMMANDS: &str = "commands.jsonl";
    /// The folder of copies of the files the lines read.
    pub const FILES: &str = "files";
    /// The window's picture.
    pub const SCREENSHOT: &str = "screenshot.png";
    /// The last lines of `log.tail`.
    pub const TAIL: &str = "log-tail.json";
}

/// `bundle.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleInfo {
    /// Bundle format version, 1.
    pub gigasculpt_repro: u32,
    /// Engine version that wrote it.
    pub engine_version: String,
    /// Hash of the command catalog of that engine.
    pub catalog_hash: String,
    /// Session seed. Logged lines pin their own random seeds; the seed matters only for the
    /// calls a bundle adds that were never logged.
    pub seed: u64,
    /// Where the commands start.
    pub start: Start,
    /// What "broken" means.
    pub predicate: Predicate,
    /// What wrote it.
    pub source: Source,
    /// How many command lines it holds.
    pub lines: u64,
    /// Anything a reader should know, in words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// Where a bundle's commands start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Start {
    /// A fresh session's empty scene.
    Empty,
    /// The bundle's `start.gss`, whose content hash (BLAKE3, as `file.open` pins it) is `hash`.
    File {
        /// The file's content hash.
        hash: String,
    },
    /// No start could be found; the bundle cannot be replayed.
    Missing {
        /// Why, in words.
        why: String,
    },
}

/// What wrote a bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A command panicked.
    Panic,
    /// `gs run` replayed a verify log and a line's scene hash differed.
    Verify,
    /// `repro.save`.
    Save,
}

/// What "broken" means for a bundle: true while the bug happens. Checked by replaying the
/// bundle in a fresh session, stopping at the first line that fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Predicate {
    /// A line fails with `E_INTERNAL` because its command panicked, with this text in the
    /// message.
    Panic {
        /// Text the panic message contains.
        contains: String,
    },
    /// A line fails with this error code, and this text in the message if given.
    Error {
        /// The error code.
        code: ErrorCode,
        /// Text the message contains. Empty: any message.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        contains: String,
    },
    /// The scene hash (`mesh.hash`) after line `line` (counting from 1) is not `expected`.
    HashMismatch {
        /// The line, counting from 1.
        #[schemars(range(min = 1))]
        line: u64,
        /// The hash the line should give.
        expected: String,
    },
    /// After the last line, `mesh.diff` from `a` to `b` moves some vertex more than `over`.
    MeshDiff {
        /// `part:NAME` or `checkpoint:NAME/PART`, as `mesh.diff` takes them.
        a: String,
        /// The other side, the same way.
        b: String,
        /// The largest displacement that is still fine, in scene units.
        #[schemars(range(min = 0.0))]
        over: f64,
    },
    /// A line does not answer within `seconds`.
    Hang {
        /// How long a line may take.
        #[serde(default = "hang_seconds")]
        #[schemars(range(min = 1))]
        seconds: u64,
    },
}

/// How the message of the `E_INTERNAL` error a command that panicked gives starts (after
/// `step N: ` in a batch).
pub const PANICKED: &str = "the command panicked: ";

/// The panic message in a command error, if its command panicked.
pub fn panic_message(code: ErrorCode, message: &str) -> Option<&str> {
    let panicked = message.split_once(PANICKED).map(|(_, m)| m);
    panicked.filter(|_| code == ErrorCode::Internal)
}

/// The default time a line may take before it counts as a hang.
pub const HANG_SECONDS: u64 = 20;

fn hang_seconds() -> u64 {
    HANG_SECONDS
}

impl Predicate {
    /// The predicate in words, for messages.
    pub fn describe(&self) -> String {
        match self {
            Self::Panic { contains } => format!("a command panics with \"{contains}\""),
            Self::Error { code, contains } if contains.is_empty() => {
                format!("a command fails with {}", code_name(*code))
            }
            Self::Error { code, contains } => {
                format!("a command fails with {} \"{contains}\"", code_name(*code))
            }
            Self::HashMismatch { line, expected } => {
                format!("the scene hash after line {line} is not {expected}")
            }
            Self::MeshDiff { a, b, over } => format!("{a} and {b} differ by more than {over}"),
            Self::Hang { seconds } => format!("a command takes longer than {seconds} s"),
        }
    }
}

/// An error code's wire name, for example `E_INTERNAL`.
fn code_name(code: ErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(ToString::to_string))
        .unwrap_or_default()
}

/// A bundle's header and command lines, the part every reader and writer shares.
#[derive(Clone, Debug)]
pub struct Bundle {
    /// `bundle.json`.
    pub info: BundleInfo,
    /// `commands.jsonl`, one `LogRecord` as JSON per line.
    pub lines: Vec<String>,
}

impl Bundle {
    /// Reads `bundle.json` and `commands.jsonl` from a bundle folder.
    pub fn read(folder: &Path) -> Result<Self, String> {
        let file = |name: &str| {
            let path = folder.join(name);
            fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))
        };
        let info: BundleInfo = serde_json::from_str(&file(bundle_file::INFO)?)
            .map_err(|e| format!("{} is not a bug bundle header: {e}", bundle_file::INFO))?;
        if info.gigasculpt_repro > BUNDLE_FORMAT {
            return Err(format!(
                "the bundle has format {}, newer than this program's {BUNDLE_FORMAT}",
                info.gigasculpt_repro
            ));
        }
        let text = file(bundle_file::COMMANDS)?;
        let lines = text.lines().filter(|l| !l.trim().is_empty());
        let lines = lines.map(ToString::to_string).collect();
        Ok(Self { info, lines })
    }

    /// Writes `bundle.json`, with `lines` counted, and `commands.jsonl` into `folder`, which
    /// must exist.
    pub fn write(&self, folder: &Path) -> io::Result<()> {
        let mut text = String::new();
        for line in &self.lines {
            text.push_str(line);
            text.push('\n');
        }
        fs::write(folder.join(bundle_file::COMMANDS), text)?;
        let info = BundleInfo {
            lines: self.lines.len() as u64,
            ..self.info.clone()
        };
        let mut text = serde_json::to_string_pretty(&info).map_err(io::Error::other)?;
        text.push('\n');
        fs::write(folder.join(bundle_file::INFO), text)
    }
}
