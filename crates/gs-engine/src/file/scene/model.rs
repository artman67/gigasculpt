//! The document as a scene file, and back (decision 0016): each Part's store, numbering and
//! properties go to gs-io as they are, and a Part with detail levels its top level's store and
//! numbering with its levels as gs-core stores them (section 15); the scene section holds the
//! scene id, the background and the material palette, the session section the session seed,
//! the next command number, where the save sits in the log, what Isolate chose and the active
//! part and material.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use gs_core::{Levels, MeshStore, Placement, StoredLevels};
use gs_io::scene::{Kept, NumberingIn, NumberingOut, PartIn, PartOut, SceneError, SceneIn};
use gs_protocol::{CommandError, ErrorCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document::{Document, Isolation, Order, Part, Props, unfit};
use crate::errors::error;
use crate::view::Source;
use crate::view::saved::chunk;

/// The program and version that writes files.
pub(crate) fn writer() -> String {
    format!("gigasculpt {}", env!("CARGO_PKG_VERSION"))
}

/// What a session restores from a file but the scene hash does not cover.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SessionSection {
    /// The session seed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// The next command number, which random numbers derive from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counter: Option<u64>,
    /// For an autosave: the scene's own file, if it had one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// For an autosave: why it was written (`max_duration`, `rest`, `recovered` or `quick`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autosave: Option<String>,
    /// Where the save sits in the command log, for recovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<LogPosition>,
    /// The open Reference Views' setups and the active one (decision 0024 section 8), read
    /// apart so one this version cannot read never costs the rest of the section.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub references: Option<Value>,
    /// What Isolate chose, while it is on (decision 0029, section 9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isolate: Option<Isolation>,
    /// The active part's stable id (decision 0033). Left out, or naming no part, the first part
    /// is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<u64>,
    /// A scene file's active material's id (decision 0043, section 7). Left out, or naming no
    /// material, MatCap Gray is active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<u64>,
}

/// A place in a command log.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct LogPosition {
    /// The log file.
    pub path: String,
    /// Which run of the session's log (counting from 1; each open starts one).
    pub run: u64,
    /// Lines of the run before this point, the header not counted.
    pub records: u64,
    /// The engine and catalog that wrote it: a replay is bit-safe only on the same ones.
    pub engine: String,
    pub catalog: String,
}

/// The document beyond its Parts.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SceneSection {
    /// Made at the first save, so autosaves of two scenes never mix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// How many part ids the document has handed out (decision 0015), so ids are never reused
    /// after reopening.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued: Option<u64>,
    /// The background (`view.background`), kept as written and read apart
    /// (`Background::saved`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Value>,
    /// A scene file's material palette (decision 0043, section 7): every material in full and
    /// the id counter, kept as written and read apart (`material::scene::opened`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette: Option<Value>,
}

/// What a save takes from a Part beside its store: its numbering's arrays (`Order::given`)
/// and, for a Part with detail levels, its levels as stored. A Part with levels writes its top
/// level's store numbered as the top, whatever level is active (decision 0016, section 15). A
/// frozen Part's set-aside levels are written the same way, as a Part of their own (decision
/// 0030, section 7).
pub(crate) struct Held<'a> {
    pub name: &'a str,
    pub store: &'a MeshStore,
    pub props: Props,
    pub given: Option<Placement>,
    pub levels: Option<StoredLevels>,
    pub frozen: Option<Box<Held<'a>>>,
}

/// What a save writes of `part`.
pub(crate) fn held<'a>(name: &'a str, part: &'a Part) -> Held<'a> {
    let props = part.props();
    let frozen = part.frozen().map(|l| Box::new(stack(name, l, props)));
    match part.levels() {
        Some(l) => Held {
            frozen,
            ..stack(name, l, props)
        },
        None => Held {
            name,
            store: part.store(),
            props,
            given: part.order().given(),
            levels: None,
            frozen,
        },
    }
}

/// What a save keeps of `part` as a parts reference's copy (decision 0024, section 8): what it
/// shows, without frozen levels, which a kept Part never holds (decision 0030, section 7;
/// review 0007, S3: the writer refuses them rather than dropping them).
pub(crate) fn kept<'a>(name: &'a str, part: &'a Part) -> Held<'a> {
    Held {
        frozen: None,
        ..held(name, part)
    }
}

/// What a save writes of a stack of levels: its top, numbered as the top, and the rest.
fn stack<'a>(name: &'a str, l: &'a Levels, props: Props) -> Held<'a> {
    Held {
        name,
        store: l.top(),
        props,
        given: Some(l.level_numbering(l.top_level()).placement()),
        levels: Some(l.stored()),
        frozen: None,
    }
}

/// One Part as gs-io writes it.
pub(crate) fn part_out<'a>(h: &'a Held<'a>) -> PartOut<'a> {
    let numbering = match &h.given {
        Some(g) => NumberingOut::Given {
            vertices: &g.vertices,
            faces: &g.faces,
            first_corner: &g.first_corner,
        },
        None => NumberingOut::Spatial,
    };
    PartOut {
        name: h.name,
        props: serde_json::to_value(h.props).unwrap_or_default(),
        store: h.store,
        numbering,
        levels: h.levels.as_ref(),
        frozen: h.frozen.as_deref().map(|f| Box::new(part_out(f))),
    }
}

/// A Part that could not be opened, and why.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Refused {
    pub part: String,
    pub reason: String,
}

/// A file turned into a document.
pub(crate) struct Loaded {
    pub doc: Document,
    pub names: Vec<String>,
    pub refused: Vec<Refused>,
    pub scene: SceneSection,
    pub session: SessionSection,
    /// The Parts kept beside the scene (a parts reference's frozen copies), by chunk name.
    pub kept: BTreeMap<String, Result<Part, String>>,
}

/// Reads the scene or Part file at `path`, and of the Parts it keeps beside the scene only the
/// frozen copies its saved references name (decision 0024, section 8).
pub(crate) fn read(path: &Path) -> Result<SceneIn, SceneError> {
    gs_io::scene::read(path, Kept::Named(&kept_names))
}

/// The names of the frozen copies a session section's saved parts references use.
fn kept_names(session: &Value) -> Vec<String> {
    let open = session
        .pointer("/references/open")
        .and_then(Value::as_array);
    let names = open.into_iter().flatten().filter_map(|entry| {
        let view = entry.get("view")?.as_str()?;
        match Source::deserialize(entry.get("source")?).ok()? {
            Source::Parts { parts } => Some(parts.into_iter().map(move |p| chunk(view, &p))),
            _ => None,
        }
    });
    names.flatten().collect()
}

/// Builds the document a file holds: every Part checked into a store, its numbering and
/// properties; a Part that cannot be is refused by name.
pub(crate) fn load(file: SceneIn) -> Loaded {
    let mut names = BTreeSet::new();
    let mut parts = Vec::new();
    let mut refused = Vec::new();
    for part in file.parts {
        let made = part
            .map_err(|e| (e.part.clone(), e.to_string()))
            .and_then(|p| {
                let name = p.name.clone();
                if names.contains(&name) {
                    return Err((name, "another part has the same name".into()));
                }
                if let Some(why) = unfit(&name) {
                    return Err((name, why.reason));
                }
                part_in(p)
                    .map(|part| (name.clone(), part))
                    .map_err(|e| (name, e))
            });
        match made {
            Ok((name, part)) => {
                names.insert(name.clone());
                parts.push((name, part));
            }
            Err((part, reason)) => refused.push(Refused { part, reason }),
        }
    }
    let order = parts.iter().map(|p| p.0.clone()).collect();
    let scene: SceneSection = serde_json::from_value(file.scene).unwrap_or_default();
    let session: SessionSection = serde_json::from_value(file.session).unwrap_or_default();
    let mut doc = Document::of(parts, scene.issued.unwrap_or(0));
    doc.set_isolation(session.isolate.clone().unwrap_or_default());
    doc.set_active(session.active);
    doc.settle();
    Loaded {
        doc,
        names: order,
        refused,
        scene,
        session,
        kept: (file.references.into_iter())
            .map(|(name, part)| (name, part.map_err(|e| e.to_string()).and_then(part_in)))
            .collect(),
    }
}

/// A Part read, with its frozen levels (decision 0030, section 7) when it has them: a stack of
/// more than one level, kept beside a Part without levels of its own, as `level.freeze` leaves
/// them.
fn part_in(mut p: PartIn) -> Result<Part, String> {
    let Some(record) = p.frozen.take() else {
        return plain_or_leveled(p);
    };
    let record = plain_or_leveled(*record).map_err(|e| format!("its frozen levels: {e}"))?;
    match (record.levels(), plain_or_leveled(p)?) {
        (Some(l), part) if part.levels().is_none() => Ok(part.with_frozen(Some(l.share()))),
        (None, _) => Err("its frozen levels hold one level".into()),
        (Some(_), _) => Err("it has detail levels and frozen ones".into()),
    }
}

fn plain_or_leveled(p: PartIn) -> Result<Part, String> {
    let store = MeshStore::from_exact(p.config, p.attrs, p.nodes).map_err(|e| e.to_string())?;
    let props: Props = match p.props {
        Value::Null => Props::default(),
        other => serde_json::from_value(other).map_err(|e| format!("its properties: {e}"))?,
    };
    if !props.transform.is_valid() {
        return Err("its transform is not a move, unit turn and positive scale".into());
    }
    match (p.numbering, p.levels) {
        (
            NumberingIn::Given {
                vertices,
                faces,
                first_corner,
            },
            Some(stored),
        ) => {
            let placed = Placement {
                vertices,
                faces,
                first_corner,
            };
            let levels = Levels::from_file(store, placed, stored).map_err(|e| e.to_string())?;
            Ok(Part::leveled(levels, props))
        }
        (NumberingIn::Spatial, Some(_)) => Err("a part with detail levels has no numbering".into()),
        (NumberingIn::Spatial, None) => Ok(Part::loaded(store, Order::Spatial, props)),
        (
            NumberingIn::Given {
                vertices,
                faces,
                first_corner,
            },
            None,
        ) => {
            let order = Order::from_given(&store, vertices, faces, first_corner)?;
            Ok(Part::loaded(store, order, props))
        }
    }
}

/// A file that cannot be read as a scene file, as a command error.
pub(crate) fn unreadable(path: &Path, e: &SceneError) -> CommandError {
    let shown = path.display();
    let backup = gs_io::scene::backup_path(path);
    let (code, hint) = match e {
        SceneError::NotScene => (
            ErrorCode::Precondition,
            "file.import reads OBJ, PLY, STL, ZTL and ZPR files".to_string(),
        ),
        SceneError::TooNew { .. } => (
            ErrorCode::Precondition,
            "open it with a newer GigaSculpt".to_string(),
        ),
        SceneError::Incomplete { .. } | SceneError::Damaged { .. } => (
            ErrorCode::Damaged,
            format!(
                "open its backup, {}, or an autosave (file.recovery lists crashed sessions)",
                backup.display()
            ),
        ),
        SceneError::Io { .. } => (
            ErrorCode::Precondition,
            "check the file and its folder".into(),
        ),
    };
    CommandError {
        hint: Some(hint),
        ..error(code, format!("{shown}: {e}"))
    }
}
