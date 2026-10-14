//! The automated approval rules of ARCHITECTURE 10.3. When a change makes test scenes look
//! different, the new pictures wait in the pending queue and a separate session decides on
//! them against the catalog item being built:
//!
//! - **Approve** only if the item says what visual change to expect, the change shows up in
//!   every scene the item affects, and the pictures pass the mesh checks.
//! - **Reject** if a scene unrelated to the item changed (a regression: the builder gets the
//!   difference map), if the mesh checks fail, if the item does not say what to expect, or if
//!   the expected change is missing from a scene it should affect.
//! - **Escalate** to the maintainer only when the change is to how something looks or feels
//!   to an artist (a brush's shape, shading, a MatCap, the interface).
//!
//! These are the mechanical rules. Whether a difference matches the expected change is judged
//! by the deciding session from the review sheet; it rejects whatever does not.

use serde::Deserialize;

use super::queue::{Goldens, Manifest, names, read};

/// The catalog item a change belongs to, as the builder states it in a small TOML file:
///
/// ```toml
/// item = "geometry.voxel-remesh"
/// expect = "the voxel-cube scene gets rounder edges"
/// scenes = ["voxel-cube"]
/// artistic = false
/// ```
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The catalog item id.
    pub item: String,
    /// The visual change to expect, in words. Empty means none was stated.
    #[serde(default)]
    pub expect: String,
    /// The scenes the item affects.
    #[serde(default)]
    pub scenes: Vec<String>,
    /// The change is to how something looks or feels to an artist.
    #[serde(default)]
    pub artistic: bool,
}

impl Request {
    pub fn parse(text: &str) -> Result<Request, String> {
        toml::from_str(text).map_err(|e| format!("approval request: {e}"))
    }
}

/// What happens to a pending picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ruling {
    Approve,
    Reject,
    Escalate,
}

/// The ruling on one pending picture, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub name: String,
    pub ruling: Ruling,
    pub reason: String,
}

/// Rules on every pending picture (`name` and manifest) for `request`.
pub fn decide(request: &Request, pending: &[(String, Manifest)]) -> Vec<Decision> {
    let missing: Vec<&str> = (request.scenes.iter())
        .filter(|s| !pending.iter().any(|(_, m)| &m.scene == *s))
        .map(String::as_str)
        .collect();
    let rule = |m: &Manifest| -> (Ruling, String) {
        if !request.scenes.contains(&m.scene) {
            let why = format!(
                "scene {} is not one {} affects: a regression",
                m.scene, request.item
            );
            return (Ruling::Reject, why);
        }
        if !m.defects.is_empty() {
            return (
                Ruling::Reject,
                format!("mesh checks fail: {}", m.defects.join("; ")),
            );
        }
        if request.expect.trim().is_empty() {
            let why = format!("{} does not say what visual change to expect", request.item);
            return (Ruling::Reject, why);
        }
        if !missing.is_empty() {
            let why = format!("the expected change is missing from {}", missing.join(", "));
            return (Ruling::Reject, why);
        }
        if request.artistic {
            let why = "changes how it looks or feels to an artist: the maintainer decides";
            return (Ruling::Escalate, why.into());
        }
        (
            Ruling::Approve,
            format!("the change {} expects", request.item),
        )
    };
    pending
        .iter()
        .map(|(name, m)| {
            let (ruling, reason) = rule(m);
            Decision {
                name: name.clone(),
                ruling,
                reason,
            }
        })
        .collect()
}

impl Goldens {
    /// [`decide`] on the whole pending queue.
    pub fn decide(&self, request: &Request) -> Result<Vec<Decision>, String> {
        let mut pending = Vec::new();
        for name in names(&self.pending())? {
            if let Some(p) = read(&self.pending(), &name)? {
                pending.push((name, p.manifest));
            }
        }
        Ok(decide(request, &pending))
    }
}
