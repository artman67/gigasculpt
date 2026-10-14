//! `exchange.read`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use gs_core::transform::Similarity;
use gs_core::{Levels, MeshError, MeshStore, NodeConfig, Numbering, Settings};
use gs_protocol::{CommandError, ErrorCode, Kind, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::incoming::{self, Incoming};
use super::place::{self, Slots};
use super::renumber;
use super::{Form, folder, level_top, manifest, same_topology, top};
use crate::document::{Order, Props, VertexOrder};
use crate::errors::{self, error};
use crate::level::{uv_corners, whole_part};
use crate::{Command, Ctx, Example, Unavailable, register_command};

/// Bring back what the Blender add-on wrote into an exchange folder (decision 0032), as one
/// undo step. Each object goes back into the part it was sent from: when it has that part's
/// points and faces, only the changed positions, mask, face groups, hidden faces and colors are
/// written, so the part keeps its detail levels and every other column (a column the add-on
/// left out reads as its empty value: zeros, and white for colors). Colors come from the
/// object's active color attribute: a point byte color with alpha 255 as it is, anything else
/// averaged to points in linear light, laid over white and rounded to the nearest byte. An
/// object with other points or faces, one never sent, or a second
/// copy of one becomes a new part beside it, with a note. An object in the Levels form (a
/// Multires modifier) comes back as its top in Blender's numbering: in place, points only, when
/// the part has as many levels and the top has its faces (matched from the base points, which
/// both programs keep first), else its levels are rebuilt as `level.reconstruct` does and it
/// becomes a new part with them.
/// Polygons of more than four corners are split into triangles as `file.import` splits them.
///
/// `hash` pins the folder: one hash of the manifest and every file it lists. Left out, it is
/// filled in with the folder's hash as it is now, so a replay reads exactly these files or
/// fails by name.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExchangeRead {
    /// The folder, in full: the add-on's `from-blender` folder.
    #[schemars(length(min = 1))]
    pub folder: String,
    /// The folder's content hash (BLAKE3, hex). Left out: the hash of the folder as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

/// What a read did, object by object.
#[derive(Serialize, JsonSchema)]
pub struct Read {
    pub parts: Vec<Came>,
}

/// One Blender object brought back.
#[derive(Serialize, JsonSchema)]
pub struct Came {
    /// The part it went into.
    pub part: String,
    /// The Blender object it came from.
    pub object: String,
    pub how: How,
    /// Polygons of more than four corners split into triangles.
    pub split: u64,
    /// Why it became a new part, when it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What became of one object.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum How {
    /// Its part already had every value it brought.
    Unchanged,
    /// Its values went into its part, which keeps everything else.
    InPlace,
    /// It became a new part.
    New,
}

impl Command for ExchangeRead {
    const NAME: &'static str = "exchange.read";
    const KIND: Kind = Kind::Edit;
    type Output = Read;

    fn examples() -> Vec<Example<Self>> {
        let call = Self {
            folder: "/nonexistent/exchange/from-blender".into(),
            hash: None,
        };
        vec![
            Example::new(
                "read what Blender sent (refused here: the add-on writes the folder first)",
                call,
            )
            .fails_with(ErrorCode::NotFound),
        ]
    }

    fn pin(&mut self, _cx: &Ctx) -> Result<(), CommandError> {
        let Some(hash) = manifest::bundle_hash(Path::new(&self.folder)) else {
            return Ok(());
        };
        match &self.hash {
            Some(expected) if *expected != hash => Err(changed(&self.folder, &hash, expected)),
            Some(_) => Ok(()),
            None => {
                self.hash = Some(hash);
                Ok(())
            }
        }
    }

    fn run(self, cx: &mut Ctx) -> Result<Read, CommandError> {
        let dir = folder(&self.folder)?;
        let bundle = manifest::load(&dir)?;
        if let Some(expected) = self.hash.as_ref().filter(|e| **e != bundle.hash) {
            return Err(changed(&self.folder, &bundle.hash, expected));
        }
        // Every object is decoded before anything changes, so a bad one refuses the call whole.
        let all: Vec<Incoming> = (bundle.manifest.parts.iter())
            .map(|e| incoming::decode(&bundle.files, e))
            .collect::<Result<_, _>>()?;
        let mut taken = BTreeSet::new();
        let mut parts = Vec::with_capacity(all.len());
        for came in all {
            parts.push(land(cx, came, &mut taken)?);
        }
        if parts.iter().all(|p| p.how == How::Unchanged) {
            cx.noop_reason("every object came back as it was sent");
        }
        Ok(Read { parts })
    }
}
register_command!(ExchangeRead, "Read from Blender");

fn changed(folder: &str, hash: &str, expected: &str) -> CommandError {
    errors::unavailable(
        Unavailable::new(format!(
            "the exchange folder {folder} changed: its content hash is {hash}, the call \
             expected {expected}"
        ))
        .fix("send again from Blender, or leave hash out to read the folder as it is now"),
    )
}

/// Puts one object into the scene: into the part it was sent from when it still has that
/// part's points and faces, else as a new part.
fn land(cx: &mut Ctx, came: Incoming, taken: &mut BTreeSet<u64>) -> Result<Came, CommandError> {
    let first = came.id.filter(|&id| taken.insert(id));
    let Some(name) = first.and_then(|id| cx.doc.resolve(&format!("#{id}"))) else {
        let note = came.id.map(|id| {
            if first.is_none() {
                format!(
                    "another object already came back as part #{id}, so this copy is a new part"
                )
            } else {
                format!("no part has id {id} now, so it is a new part")
            }
        });
        return new_part(cx, came, note);
    };
    if came.form == Form::Levels && came.levels > 1 {
        return levels_back(cx, &name, came);
    }
    let (shown, _) = top(cx.doc.get_part(&name)?)?;
    if !same_topology(&shown, &came.mesh) {
        let note = differs(&name, &shown, &came.mesh);
        return new_part(cx, came, Some(note));
    }
    in_place(cx, &name, &came, &came.mesh, came.form == Form::Mesh)
}

/// An object in the Levels form. When the part has as many levels and the top has its faces in
/// Blender's numbering, the points go into the part's top (points only). Otherwise its levels
/// are rebuilt from the top, and it goes into the part when that gives the part's faces, else
/// it becomes a new part with those levels.
fn levels_back(cx: &mut Ctx, name: &str, came: Incoming) -> Result<Came, CommandError> {
    let (shown, had) = top(cx.doc.get_part(name)?)?;
    let base = (had.as_ref())
        .filter(|l| l.count() == came.levels as usize)
        .and_then(|l| u32::try_from(l.level_numbering(0).vertex_count()).ok());
    if let Some(order) = base.and_then(|b| renumber::renumbering(&came.mesh, &shown, b)) {
        let mut positions = vec![[0.0; 3]; order.len()];
        for (p, &q) in came.mesh.positions.iter().zip(&order) {
            positions[q as usize] = *p;
        }
        let faces = shown.faces.clone();
        let mesh = PlainMesh {
            positions,
            faces,
            ..PlainMesh::default()
        };
        return in_place(cx, name, &came, &mesh, false);
    }
    let (mesh, levels) = level_top(&rebuild(&came)?)?;
    if same_topology(&shown, &mesh) {
        return in_place(cx, name, &came, &mesh, false);
    }
    let note = format!(
        "{}; its levels came back without creases",
        differs(name, &shown, &mesh)
    );
    let new = cx.doc.free_name(&came.name);
    cx.doc.insert_levels(&new, levels, &[&came.mesh])?;
    let props = cx.doc.get_part(&new)?.props();
    cx.doc.set_props(
        &new,
        Props {
            transform: came.transform,
            ..props
        },
    )?;
    Ok(Came {
        part: new,
        object: came.name,
        how: How::New,
        split: came.split,
        note: Some(note),
    })
}

/// The level stack whose top `came` is: `came.levels - 1` levels rebuilt below it.
fn rebuild(came: &Incoming) -> Result<Levels, CommandError> {
    let refused = |e: MeshError| {
        error(
            ErrorCode::Precondition,
            format!(
                "the object '{}' from Blender has {} levels, but they could not be rebuilt \
                 from its top: {e}",
                came.name, came.levels
            ),
        )
    };
    let settings = Settings {
        creases: Vec::new(),
        pin_corners: false,
        corners: uv_corners(),
    };
    let (store, placed) =
        MeshStore::import(came.mesh.clone(), NodeConfig::default()).map_err(refused)?;
    let numbering = Arc::new(Numbering::from_placement(&store, placed));
    let mut levels = Levels::new(store, numbering, &settings).map_err(refused)?;
    for _ in 1..came.levels {
        levels = levels.reconstruct().map_err(refused)?;
    }
    Ok(levels)
}

/// Writes `mesh`, which has part `name`'s points and faces, into the part's top (with
/// `columns`, its mask, groups, hidden faces and colors too), and `came`'s place.
fn in_place(
    cx: &mut Ctx,
    name: &str,
    came: &Incoming,
    mesh: &PlainMesh,
    columns: bool,
) -> Result<Came, CommandError> {
    let part = cx.doc.get_part(name)?;
    let numbering = match (part.levels(), part.order()) {
        (Some(l), _) => Some(Arc::clone(l.level_numbering(l.top_level()))),
        (None, Order::Given(given)) => Some(given),
        (None, Order::Spatial) => None,
    };
    let props = part.props();
    let moved = !same_place(&props.transform, &came.transform);
    if moved {
        let transform = came.transform;
        cx.doc.set_props(name, Props { transform, ..props })?;
    }
    let mut wrote = 0;
    whole_part(cx, name, |store| {
        let slots = Slots::of(store, numbering.as_deref());
        let nodes = place::write(store, &slots, mesh, columns);
        wrote = nodes.len();
        nodes
    })?;
    Ok(Came {
        part: name.into(),
        object: came.name.clone(),
        how: if moved || wrote > 0 {
            How::InPlace
        } else {
            How::Unchanged
        },
        split: came.split,
        note: None,
    })
}

/// A new part from `came`, named from its object, with its place.
fn new_part(cx: &mut Ctx, came: Incoming, note: Option<String>) -> Result<Came, CommandError> {
    let name = cx.doc.free_name(&came.name);
    let props = Props {
        transform: came.transform,
        ..Props::default()
    };
    cx.doc
        .put_mesh(&name, came.mesh, VertexOrder::AsGiven, props)?;
    Ok(Came {
        part: name,
        object: came.name,
        how: How::New,
        split: came.split,
        note,
    })
}

/// The note for an object whose points or faces are not part `name`'s.
fn differs(name: &str, had: &PlainMesh, came: &PlainMesh) -> String {
    let shape = |m: &PlainMesh| format!("{} points and {} faces", m.positions.len(), m.faces.len());
    format!(
        "its points and faces are not part '{name}''s ({} came back, the part has {}), so it \
         is a new part and '{name}' is unchanged",
        shape(came),
        shape(had)
    )
}

/// Whether two places are the same, bit for bit.
fn same_place(a: &Similarity, b: &Similarity) -> bool {
    let bits = |t: &Similarity| {
        let (p, q) = (t.translation.to_array(), t.rotation.to_array());
        (p.map(f32::to_bits), q.map(f32::to_bits), t.scale.to_bits())
    };
    bits(a) == bits(b)
}
