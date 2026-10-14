//! `mesh.diff`, and its heatmap (catalog item `core.mesh-diff-heatmap`).

use std::sync::Arc;

use gs_core::{AttrDef, AttrVec, NewValues, NodeId};
use gs_protocol::{CommandError, ErrorCode, Kind, PlainMesh};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Bounds;
use crate::display::{DisplaySettings, MaskTint};
use crate::document::{DocTxn, Document, Part};
use crate::errors::args_invalid;
use crate::math::{dot, sub};
use crate::view::{ImageOutput, Labelled, NamedView, RenderRequest, Shading, ViewSpec};
use crate::{Command, Ctx, Example, register_command};

/// Compare two meshes with the same vertices and faces, vertex by vertex: a part now against
/// itself in a checkpoint, for example, to see what an edit did.
///
/// `a` and `b` each name a part as it is now (`part:NAME`) or as a history checkpoint saved it
/// (`checkpoint:NAME/PART`, split at the first `/`, which is right because checkpoint names
/// hold none and part names may). Each vertex's displacement runs from its
/// place in `a` to its place in `b`. Reports the largest, mean and RMS displacement, how many
/// vertices moved more than `epsilon` and the box around them (their places in `a` and in
/// `b`), and the displacement along `a`'s vertex normals (positive is outward). Two meshes whose
/// vertex counts or faces differ are not compared: the result gives both counts and says so.
/// With `heatmap`, also draws `b` alone with each point colored by that displacement.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshDiff {
    /// The first mesh: `part:NAME` or `checkpoint:NAME/PART`.
    #[schemars(length(min = 1))]
    pub a: String,
    /// The second mesh, the same way.
    #[schemars(length(min = 1))]
    pub b: String,
    /// A vertex counts as moved when it moved further than this, in scene units.
    #[serde(default = "default_epsilon")]
    #[schemars(range(min = 0.0))]
    pub epsilon: f64,
    /// Also draw `b` alone offscreen, each point colored by how far it moved along `a`'s normal:
    /// white for none, red outward, blue inward, full at `scale`. Needs a renderer, as
    /// `view.snapshot` does. Left out: no picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<Heatmap>,
}

fn default_epsilon() -> f64 {
    1e-6
}

/// How the heatmap is drawn. It is drawn in MatCap White under the default display, whatever
/// the session's, so its colors mean the same in every session.
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Heatmap {
    /// The camera, as `view.snapshot` takes it, framing `b` alone. Left out: front.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewSpec>,
    /// Width and height in pixels, 16 to 4096. Left out: 512.
    #[serde(default = "default_size")]
    #[schemars(range(min = 16, max = 4096))]
    pub size: u32,
    /// The displacement along the normal drawn fully red or blue, in scene units. Left out:
    /// the largest one, so the strongest change is full color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub scale: Option<f64>,
    /// Write the PNG to this file instead of returning it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1))]
    pub path: Option<String>,
}

fn default_size() -> u32 {
    512
}

/// The look the heatmap draws in: MatCap White, so a point that did not move draws white and
/// the red and blue show at full strength.
const HEAT_LOOK: &str = "white";

/// How two meshes differ. Lengths are in scene units.
#[derive(Serialize, JsonSchema)]
pub struct MeshDiffOutput {
    pub a: Side,
    pub b: Side,
    /// True when both have the same vertex count and the same faces, so they were compared.
    pub same_topology: bool,
    /// Why they were not compared, when they were not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How far the vertices moved from `a` to `b`. Null when the topology differs.
    pub displacement: Option<Displacement>,
    /// The heatmap, when one was asked for and the topology is the same.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heatmap: Option<HeatmapOutput>,
}

/// The heatmap's picture and the scale it was drawn at.
#[derive(Serialize, JsonSchema)]
pub struct HeatmapOutput {
    /// The displacement along the normal drawn fully red or blue, in scene units: the one
    /// given, or else the largest. 0 when no point moved along its normal: all white.
    pub scale: f64,
    #[serde(flatten)]
    pub image: ImageOutput,
}

/// One of the compared meshes.
#[derive(Serialize, JsonSchema)]
pub struct Side {
    /// The part's name.
    pub part: String,
    /// The checkpoint it was read from, if not the scene now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<String>,
    /// Number of vertices.
    pub vertices: u64,
    /// Number of faces.
    pub faces: u64,
}

/// Displacement statistics over every vertex.
#[derive(Serialize, JsonSchema)]
pub struct Displacement {
    /// The largest distance one vertex moved.
    pub max: f64,
    /// The mean distance over every vertex.
    pub mean: f64,
    /// The root mean square distance over every vertex.
    pub rms: f64,
    /// How many vertices moved further than `epsilon`.
    pub moved: u64,
    /// The box around the moved vertices, at their places in `a` and in `b`. Null when none
    /// moved.
    #[serde(rename = "box")]
    pub bounds: Option<Bounds>,
    /// The displacement along `a`'s vertex normals.
    pub along_normal: AlongNormal,
}

/// Signed displacement along a vertex normal: positive outward, negative inward.
#[derive(Serialize, JsonSchema)]
pub struct AlongNormal {
    /// The mean over every vertex.
    pub mean: f64,
    /// The most inward.
    pub min: f64,
    /// The most outward.
    pub max: f64,
}

/// A mesh read for comparison, with its vertex normals when they are wanted, and the part it
/// is (a share, for the heatmap).
struct Read {
    side: Side,
    mesh: Arc<PlainMesh>,
    normals: Vec<[f32; 3]>,
    part: Part,
}

/// Reads the mesh `reference` names. `field` is the argument's JSON pointer.
fn read(cx: &Ctx, reference: &str, field: &str, normals: bool) -> Result<Read, CommandError> {
    let from = |doc: &DocTxn, part: &str, checkpoint: Option<String>| {
        let mesh = doc.part(part)?;
        let side = Side {
            part: doc.resolve(part).unwrap_or_else(|| part.to_string()),
            checkpoint,
            vertices: mesh.positions.len() as u64,
            faces: mesh.faces.len() as u64,
        };
        let normals = if normals { doc.normals(part)? } else { vec![] };
        Ok(Read {
            side,
            mesh,
            normals,
            part: doc.get_part(part)?.share(),
        })
    };
    if let Some(part) = reference.strip_prefix("part:") {
        return from(&cx.doc, part, None);
    }
    if let Some((name, part)) =
        (reference.strip_prefix("checkpoint:")).and_then(|r| r.split_once('/'))
    {
        let saved = cx.history.checkpoint_named(name)?;
        // A read-only document of the checkpoint's shared parts, so its parts resolve and give
        // their normals exactly as the scene's do.
        let shared = saved.parts.iter().map(|(n, p)| (n.clone(), p.share()));
        let mut doc = Document::of(shared, 0);
        return from(&DocTxn::new(&mut doc), part, Some(name.to_string()));
    }
    Err(args_invalid(
        field,
        format!("'{reference}' names no mesh; use part:NAME or checkpoint:NAME/PART"),
    ))
}

impl Command for MeshDiff {
    const NAME: &'static str = "mesh.diff";
    const KIND: Kind = Kind::Query;
    type Output = MeshDiffOutput;

    fn examples() -> Vec<Example<Self>> {
        let since = |summary, heatmap| {
            Example::new(
                summary,
                Self {
                    a: "checkpoint:base/sphere".into(),
                    b: "part:sphere".into(),
                    epsilon: default_epsilon(),
                    heatmap,
                },
            )
            .after("primitive.sphere", json!({}))
            .after("history.checkpoint", json!({ "name": "base" }))
            .after(
                "mesh.translate",
                json!({ "direction": [1, 0, 0], "distance": 0.1 }),
            )
        };
        let heatmap = Heatmap {
            view: Some(ViewSpec::Named(NamedView::Right)),
            size: 256,
            scale: None,
            path: None,
        };
        vec![
            since("what a move did to the sphere since a checkpoint", None),
            since(
                "and a heatmap of it from the right: red where it moved out, blue in (a \
                 session without a renderer refuses)",
                Some(heatmap),
            )
            .fails_with(ErrorCode::Precondition),
        ]
    }

    fn run(self, cx: &mut Ctx) -> Result<MeshDiffOutput, CommandError> {
        let a = read(cx, &self.a, "/a", true)?;
        let b = read(cx, &self.b, "/b", false)?;
        let same_topology =
            a.mesh.faces == b.mesh.faces && a.mesh.positions.len() == b.mesh.positions.len();
        let reason = (!same_topology).then(|| {
            format!(
                "different topology: a has {} vertices and {} faces, b has {} and {}{}",
                a.side.vertices,
                a.side.faces,
                b.side.vertices,
                b.side.faces,
                if a.side.vertices == b.side.vertices && a.side.faces == b.side.faces {
                    " (the faces use different vertices)"
                } else {
                    ""
                },
            )
        });
        let displacement = same_topology.then(|| displacement(&a, &b, self.epsilon));
        let heatmap = match self.heatmap {
            Some(asked) if same_topology => Some(heatmap(cx, asked, &a, &b)?),
            _ => None,
        };
        Ok(MeshDiffOutput {
            a: a.side,
            b: b.side,
            same_topology,
            reason,
            displacement,
            heatmap,
        })
    }
}
register_command!(MeshDiff, "Compare meshes");

/// How far each vertex moved along `a`'s normal, positive outward.
fn along_normal(a: &Read, b: &Read) -> Vec<f64> {
    let vertices = a.mesh.positions.iter().zip(&b.mesh.positions);
    (vertices.zip(&a.normals))
        .map(|((pa, pb), n)| dot(sub(pb.map(f64::from), pa.map(f64::from)), n.map(f64::from)))
        .collect()
}

/// The heatmap's color of displacement `d` at `scale`: white at 0, toward red outward and blue
/// inward, full at `scale` and past it. An `Rgba8` word: sRGB bytes, red in the low byte.
fn heat(d: f64, scale: f64) -> u32 {
    let t = if scale > 0.0 {
        (d / scale).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let fade = (255.0 * (1.0 - t.abs())).round() as u32;
    let [r, g, b] = if t >= 0.0 {
        [255, fade, fade]
    } else {
        [fade, fade, 255]
    };
    r | g << 8 | b << 16 | 0xff << 24
}

/// `b` drawn alone with its points colored by `along_normal`, as `asked` says.
fn heatmap(cx: &Ctx, asked: Heatmap, a: &Read, b: &Read) -> Result<HeatmapOutput, CommandError> {
    let along = along_normal(a, b);
    let largest = along.iter().fold(0.0f64, |m, d| m.max(d.abs()));
    let scale = asked.scale.unwrap_or(largest);
    // The colors go into a share of b's store, so the scene never changes.
    let (mut store, numbering) = (b.part.store().share(), b.part.numbering());
    let nodes: Vec<NodeId> = (0..store.node_count() as NodeId).collect();
    let words = |n: NodeId| {
        let slots = 0..store.node(n).pos.len() as u32;
        let word = |l| along.get(numbering.index(n, l) as usize);
        let words = slots.map(|l| word(l).map_or(u32::MAX, |&d| heat(d, scale)));
        NewValues::Values(AttrVec::U32(words.collect()))
    };
    let fresh = nodes.iter().map(|&n| words(n)).collect();
    store.put_column(AttrDef::color(), &nodes, fresh);
    // Framed on b alone.
    let alone = Document::of([(b.side.part.clone(), b.part.share())], 0);
    let view = (asked.view).unwrap_or(ViewSpec::Named(NamedView::Front));
    let camera = view.resolve(&alone, cx.views, 1.0)?;
    let label = view.label();
    let cameras = vec![Labelled {
        label: label.clone(),
        camera,
    }];
    let size = [asked.size; 2];
    let hidden = MaskTint {
        shown: false,
        ..MaskTint::default()
    };
    let request = RenderRequest {
        matcap: Some(HEAT_LOOK.into()),
        display: DisplaySettings {
            mask: hidden,
            ..DisplaySettings::default()
        },
        ..RenderRequest::new(cameras, size, vec![b.part.transform()])
    };
    let picture = crate::view::render(cx, &[&store], &request)?;
    let (path, png_base64) = crate::view::deliver(&picture.png, asked.path)?;
    let image = ImageOutput {
        width: picture.width,
        height: picture.height,
        views: vec![label],
        shading: Shading::Matcap,
        path,
        png_base64,
    };
    Ok(HeatmapOutput { scale, image })
}

/// The displacement from `a` to `b`, which have the same vertex count, in f64.
fn displacement(a: &Read, b: &Read, epsilon: f64) -> Displacement {
    let (mut max, mut sum, mut squares) = (0.0f64, 0.0, 0.0);
    let (mut along_sum, mut along_min, mut along_max) = (0.0, f64::INFINITY, f64::NEG_INFINITY);
    let mut moved = Vec::new();
    let vertices = a.mesh.positions.iter().zip(&b.mesh.positions);
    for ((pa, pb), n) in vertices.zip(&a.normals) {
        let d = sub(pb.map(f64::from), pa.map(f64::from));
        let length = dot(d, d).sqrt();
        max = max.max(length);
        sum += length;
        squares += length * length;
        let along = dot(d, n.map(f64::from));
        along_sum += along;
        along_min = along_min.min(along);
        along_max = along_max.max(along);
        if length > epsilon {
            moved.extend([*pa, *pb]);
        }
    }
    let count = a.mesh.positions.len().max(1) as f64;
    let finite = |v: f64| if v.is_finite() { v } else { 0.0 };
    Displacement {
        max,
        mean: sum / count,
        rms: (squares / count).sqrt(),
        moved: moved.len() as u64 / 2,
        bounds: Bounds::of(&moved),
        along_normal: AlongNormal {
            mean: along_sum / count,
            min: finite(along_min),
            max: finite(along_max),
        },
    }
}
