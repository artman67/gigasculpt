//! Views and looks (decision 0017, ARCHITECTURE 7.2 and 7.5).
//!
//! Cameras belong to whoever looks: the session's views (`views`) are moved by the View
//! commands (`commands`), each logged once per gesture and never part of document undo. The
//! offscreen looks, `view.snapshot` and `view.contact_sheet`, render through the session's
//! [`RenderHost`](crate::RenderHost) with cameras of their own or a view's, which never moves,
//! and work the same in a headless session. Every camera a command names goes through one
//! resolver (`spec`), so a pixel means the same point to a look, a stroke and a pick.

mod commands;
mod contact_sheet;
mod frame;
mod reference;
mod secondary;
mod snapshot;
mod spec;
mod views;

use base64::Engine as _;
use gs_core::MeshStore;
use gs_core::camera::Camera;
use gs_core::transform::Similarity;
use gs_protocol::{CommandError, ErrorCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use commands::pin;
pub(crate) use reference::saved;
pub(crate) use secondary::Scene;
pub use secondary::{Alignment, Link, RefLook, ReferenceInfo, Source};
pub use spec::{ExactCamera, NamedView, Orbit, ViewName, ViewSpec};
pub use views::{AGENT_VIEWS, MainSettings, NavSettings, OrbitArg, PivotRule, ViewInfo, Views};
pub(crate) use views::{Ended, MAIN, own_view};

use crate::display::DisplaySettings;
use crate::errors::{error, unavailable};
use crate::{Ctx, Unavailable};

/// How surfaces are shaded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Shading {
    /// The selected MatCap (`material.select`; MatCap Gray until one is chosen). The default.
    #[default]
    Matcap,
    /// The selected look as a material lit live by the session's environment and lights
    /// (`view.environment`, `light.set`), with live subsurface for looks that scatter.
    Lit,
    /// The clay look (light brown clay), lit live as Lit mode lights.
    Clay,
    /// Reflection stripes over clay: small changes in shape show as large changes in the
    /// stripes.
    Zebra,
    /// World-space normals as colors.
    Normals,
}

/// What a render host draws: every camera, in a tile of `tile` pixels (width, height) each,
/// `columns` tiles across. One camera is one picture; several are a contact sheet with each
/// camera's label in its tile's corner.
pub struct RenderRequest {
    pub cameras: Vec<Labelled>,
    pub tile: [u32; 2],
    pub columns: u32,
    pub shading: Shading,
    /// The look `matcap` and `lit` shading draw, by name; `None`: the selected one. A
    /// Reference View has its own (decision 0020).
    pub matcap: Option<String>,
    /// The session's display (decision 0023): exposure, view transform, environment, lights,
    /// subsurface, occlusion and the mask's tint. Its floor, outline and face groups are the
    /// window's; a look draws by `draws`.
    pub display: DisplaySettings,
    /// What the picture draws besides the shaded parts. Its `active_outline` is what was asked
    /// for; `outlined` is the part that draws.
    pub draws: Draws,
    /// Where each part drawn sits in the scene (decision 0029), in the order the parts are
    /// given: the GPU copy of a part's store is drawn through it.
    pub places: Vec<Similarity>,
    /// The part outlined as the active one (decision 0033, section 5), by its index in
    /// `places`; `None` outlines nothing.
    pub outlined: Option<usize>,
    /// The parts whose paint is hidden (`part.set_paint_visible`), by their index in `places`:
    /// they draw in their material alone.
    pub unpainted: Vec<usize>,
}

impl RenderRequest {
    /// One picture of the parts at `places` through `cameras` in tiles of `tile` pixels, one
    /// column, in the default look: the selected MatCap, the default display, anti-aliased,
    /// and nothing drawn but the parts. Every request starts here, so a field added later has
    /// one default.
    pub(crate) fn new(cameras: Vec<Labelled>, tile: [u32; 2], places: Vec<Similarity>) -> Self {
        Self {
            cameras,
            tile,
            columns: 1,
            shading: Shading::Matcap,
            matcap: None,
            display: DisplaySettings::default(),
            draws: Draws::default(),
            places,
            outlined: None,
            unpainted: Vec::new(),
        }
    }
}

/// What a picture draws besides its parts' shaded surfaces: the switches `view.snapshot` and
/// `view.contact_sheet` share. Each is off when left out (anti-aliasing on), whatever the
/// window shows, so a picture never depends on the window's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per thing a picture may draw"
)]
pub struct Draws {
    /// Draw the edges over the surface.
    #[serde(default)]
    pub wireframe: bool,
    /// Smooth edges and ambient occlusion by averaging 4 samples per pixel, each with the
    /// camera shifted by a fraction of a pixel. False draws one sample per pixel, so a pixel
    /// shows exactly what its center sees (as `pick.ray` at that pixel does). Left out: true.
    #[serde(default = "crate::command::yes")]
    pub antialias: bool,
    /// Draw the floor grid under the parts (lines at powers of ten of the scene unit, as
    /// dense on screen at any zoom). Left out: false, whether or not the window shows its
    /// floor (`view.display`'s `floor`), so the user's Floor button never changes the picture.
    #[serde(default)]
    pub grid: bool,
    /// Outline the active part in orange, as the window does, when the picture shows two parts
    /// or more. Left out: false, whether or not the window outlines it (`view.display`'s
    /// `active_outline`), so the picture never depends on the window's setting.
    #[serde(default)]
    pub active_outline: bool,
    /// Show every part's face groups, one flat color each, in place of its paint. Left out:
    /// false, whether or not the window shows them (`view.display`'s `face_groups`).
    #[serde(default)]
    pub face_groups: bool,
    /// Color every part by the size of its polygons on screen: red large, green, blue small.
    /// Left out: false, whether or not the window shows it (`view.wire`'s `density`).
    #[serde(default)]
    pub density: bool,
}

impl Default for Draws {
    fn default() -> Self {
        Self {
            wireframe: false,
            antialias: true,
            grid: false,
            active_outline: false,
            face_groups: false,
            density: false,
        }
    }
}

/// An exact camera and what a contact sheet labels its tile with.
pub struct Labelled {
    pub label: String,
    pub camera: Camera,
}

/// The picture, and what it shows.
#[derive(Serialize, JsonSchema)]
pub struct ImageOutput {
    /// Size in pixels.
    pub width: u32,
    pub height: u32,
    /// The views, as labelled, left to right and top to bottom.
    pub views: Vec<String>,
    pub shading: Shading,
    /// Where the PNG was written, when a path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The PNG as base64, when no path was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png_base64: Option<String>,
}

/// The settings both look commands share.
struct Look {
    views: Vec<ViewSpec>,
    tile: [u32; 2],
    columns: u32,
    /// `None`: the session's (`view.display`).
    shading: Option<Shading>,
    draws: Draws,
    path: Option<String>,
}

/// Why a look cannot be taken now: there is nothing to look at.
fn check(cx: &Ctx) -> Result<(), Unavailable> {
    if cx.doc.part_names().next().is_none() {
        return Err(Unavailable::new("the scene has no parts to look at")
            .fix("create one first, for example with primitive.sphere"));
    }
    Ok(())
}

/// The Reference View a look sees through, if any. A picture shows one scene, so a reference
/// is looked at alone: its own scene, in its own look.
fn through<'v>(
    views: &'v Views,
    specs: &[ViewSpec],
) -> Result<Option<&'v secondary::Secondary>, CommandError> {
    let reference = |v: &ViewSpec| match v {
        ViewSpec::View(name) if views.secondary(&name.0).is_some() => Some(name.0.clone()),
        _ => None,
    };
    let seen: Vec<Option<String>> = specs.iter().map(reference).collect();
    match seen.first() {
        Some(Some(name)) if seen.iter().all(|s| s.as_ref() == Some(name)) => {
            Ok(views.secondary(name))
        }
        _ if seen.iter().all(Option::is_none) => Ok(None),
        _ => Err(crate::errors::args_invalid(
            "/views",
            "a picture shows one scene, and a reference view shows its own: look at it alone \
             (view.snapshot view=ref:1)",
        )),
    }
}

/// A session view a look names draws its edges (`view.look`), so the look sees what the
/// view shows.
fn shows_edges(views: &Views, spec: &ViewSpec) -> bool {
    matches!(spec, ViewSpec::View(name) if views.wireframe(&name.0))
}

/// Renders through the session's render host, and writes or returns the PNG.
fn look(cx: &mut Ctx, look: Look) -> Result<ImageOutput, CommandError> {
    // Resolved here, so a stroke aimed through the same view sees exactly this picture.
    let doc = cx.doc.document();
    let reference = through(cx.views, &look.views)?;
    let aspect = look.tile[0] as f32 / look.tile[1] as f32;
    let cameras = (look.views.iter())
        .map(|v| {
            let camera = v.resolve(doc, cx.views, aspect)?;
            Ok(Labelled {
                label: v.label(),
                camera,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    let views = cameras.iter().map(|c| c.label.clone()).collect();
    let shading = look.shading.unwrap_or(cx.display.shading);
    let scene = reference.map_or(doc, |r| r.scene.doc(doc));
    let shown: Vec<_> = scene.shown().collect();
    // The session's active part, outlined when asked for; a Reference View's own scene is
    // never outlined, since no stroke reaches it.
    let session_scene = reference.is_none_or(|r| matches!(r.scene, Scene::Live));
    let outlined = (look.draws.active_outline && session_scene)
        .then(|| (shown.iter()).position(|(name, _)| Some(name.as_str()) == doc.active()))
        .flatten();
    let places = shown.iter().map(|(_, part)| part.transform()).collect();
    let unpainted = (shown.iter().enumerate())
        .filter(|(_, (_, part))| part.props().paint_hidden)
        .map(|(i, _)| i)
        .collect();
    let edges = look.views.iter().any(|v| shows_edges(cx.views, v));
    let request = RenderRequest {
        columns: look.columns,
        shading,
        matcap: reference.and_then(|r| r.look.matcap.clone()),
        display: cx.display.clone(),
        draws: Draws {
            wireframe: look.draws.wireframe || edges,
            ..look.draws
        },
        outlined,
        unpainted,
        ..RenderRequest::new(cameras, look.tile, places)
    };
    let parts: Vec<&MeshStore> = shown.iter().map(|(_, part)| part.store()).collect();
    let picture = render(cx, &parts, &request)?;
    let (path, png_base64) = deliver(&picture.png, look.path)?;
    Ok(ImageOutput {
        width: picture.width,
        height: picture.height,
        views,
        shading,
        path,
        png_base64,
    })
}

/// `parts` drawn through the session's render host as `request` asks.
pub(crate) fn render(
    cx: &Ctx,
    parts: &[&MeshStore],
    request: &RenderRequest,
) -> Result<crate::Screenshot, CommandError> {
    let picture = cx
        .render
        .with(|r| r.render(parts, request))
        .ok_or_else(|| {
            unavailable(
                Unavailable::new("this session was started without a renderer")
                    .fix("start sessions with the gigasculpt program, as gs spawn does"),
            )
        })?;
    picture.map_err(|e| error(ErrorCode::Precondition, format!("the render failed: {e}")))
}

/// A PNG written to `path` (returned as the path), or else returned as base64.
pub(crate) fn deliver(
    png: &[u8],
    path: Option<String>,
) -> Result<(Option<String>, Option<String>), CommandError> {
    match path {
        Some(path) => {
            std::fs::write(&path, png).map_err(|e| CommandError {
                path: Some("/path".into()),
                ..error(ErrorCode::Precondition, format!("cannot write {path}: {e}"))
            })?;
            Ok((Some(path), None))
        }
        None => Ok((
            None,
            Some(base64::engine::general_purpose::STANDARD.encode(png)),
        )),
    }
}
