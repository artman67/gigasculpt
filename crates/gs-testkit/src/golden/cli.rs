//! The `golden` tool: the `gs golden` commands of ARCHITECTURE 10.3, run as
//! `cargo run -p gs-testkit --bin golden -- <command>`.

use std::path::PathBuf;

use gs_render::{AdapterKind, Renderer};

use super::queue::{Goldens, Status, names};
use super::rules::Request;

/// Shown for `help` and for a command line the tool does not understand.
pub const USAGE: &str = "\
golden [--root DIR] [--work DIR] COMMAND

  check              render every scene on lavapipe and compare with its reference;
                     pictures that do not pass go to the pending queue
  review             the pending queue against the references (and a sheet,
                     WORK/review.png), and approvals awaiting the maintainer
  decide FILE        the automated approval rules for the catalog item in FILE
  approve NAME..     move pending pictures over their references (--all: every one)
  reject NAME..      drop pending pictures (--all: every one)
  revert NAME..      put back the reference an approval replaced
  keep NAME..        confirm approvals (--all: every one awaiting a look)
  digest [OUT]       one sheet of every changed reference, old next to new
                     (default WORK/digest.png)

ROOT defaults to the repository's goldens/, WORK to target/golden/.";

/// Runs one command line (without the program name). The text is the report; an error means
/// the command failed or, for `check`, that some picture did not pass.
pub fn run(args: &[String]) -> Result<String, String> {
    let mut goldens = Goldens::repo();
    let mut rest = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let mut dir = || words.next().map(PathBuf::from).ok_or(USAGE.to_string());
        match word.as_str() {
            "--root" => goldens.root = dir()?,
            "--work" => goldens.work = dir()?,
            _ => rest.push(word.as_str()),
        }
    }
    command(&goldens, &rest)
}

/// The names a command acts on: the ones given, or with `--all` every picture in `all`.
fn chosen(given: &[&str], all: Result<Vec<String>, String>) -> Result<Vec<String>, String> {
    match given {
        [] => Err(USAGE.into()),
        ["--all"] => all,
        _ => Ok(given.iter().map(|s| (*s).to_string()).collect()),
    }
}

fn done(verb: &str, names: &[String]) -> String {
    format!("{verb} {}: {}", names.len(), names.join(", "))
}

fn command(g: &Goldens, words: &[&str]) -> Result<String, String> {
    match words {
        ["check"] => check(g),
        ["review"] => g.review(),
        ["decide", file] => {
            let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
            let lines: Vec<String> = (g.decide(&Request::parse(&text)?)?)
                .iter()
                .map(|d| format!("{:?} {}: {}", d.ruling, d.name, d.reason))
                .collect();
            Ok(lines.join("\n"))
        }
        ["approve", given @ ..] => {
            let picked = chosen(given, names(&g.pending()))?;
            g.approve(&picked).map(|()| done("approved", &picked))
        }
        ["reject", given @ ..] => {
            let picked = chosen(given, names(&g.pending()))?;
            g.reject(&picked).map(|()| done("rejected", &picked))
        }
        ["revert", given @ ..] if given != ["--all"] => {
            let picked = chosen(given, Ok(Vec::new()))?;
            g.revert(&picked).map(|()| done("reverted", &picked))
        }
        ["keep", given @ ..] => {
            let picked = chosen(given, names(&g.previous()))?;
            g.keep(&picked).map(|()| done("kept", &picked))
        }
        ["digest", out @ ..] if out.len() <= 1 => {
            let path = out
                .first()
                .map_or_else(|| g.work.join("digest.png"), PathBuf::from);
            match g.digest(&path)? {
                0 => Ok("no changed references".into()),
                count => Ok(format!("{count} changed references: {}", path.display())),
            }
        }
        ["help"] => Ok(USAGE.into()),
        _ => Err(USAGE.into()),
    }
}

/// `check`: one line per picture that did not pass.
fn check(g: &Goldens) -> Result<String, String> {
    let r = Renderer::new(AdapterKind::Lavapipe).map_err(|e| e.to_string())?;
    let outcomes = g.check(&r)?;
    let failed: Vec<String> = (outcomes.iter())
        .filter_map(|o| match &o.status {
            Status::Matches(_) => None,
            Status::Changed(v) => Some(format!("  {} {}", o.name, v.summary())),
            Status::New => Some(format!("  {} is new", o.name)),
        })
        .collect();
    let summary = format!(
        "{} pictures, {} match their references",
        outcomes.len(),
        outcomes.len() - failed.len()
    );
    if failed.is_empty() {
        Ok(summary)
    } else {
        let queued = format!("queued in {}", g.pending().display());
        Err(format!("{summary}\n{}\n{queued}", failed.join("\n")))
    }
}
