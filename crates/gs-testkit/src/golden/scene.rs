//! Golden scenes: a list of command steps run in a fresh session, then rendered offscreen at
//! 512x512 through gs-render in each of the scene's looks.

use std::path::Path;

use gs_protocol::Step;
use gs_render::Light;
use gs_render::matcap::{Recipe, bake};
use gs_render::shading::lit::LitEnvironment;
use gs_render::{
    Camera, GpuMesh, Image, Preset, RenderSize, Renderer, Scene as Frame, Shading, Sides,
    Subsurface, SubsurfaceMode, UiLayer, UploadOptions, View,
};
use serde::Deserialize;
use serde_json::json;

use crate::session::{Scene, Session};
use crate::validate::{Expect, validate};

/// Width and height of every golden image (ARCHITECTURE 10.3).
pub const SIZE: u32 = 512;

/// How a golden image is shaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Look {
    /// Reflection stripes, which turn small shape changes into large color changes. Every
    /// scene that checks a sculpting result has one.
    Zebra,
    /// Two-light clay.
    Clay,
    /// A shipped MatCap, by file stem in `assets/matcaps/`.
    MatCap(String),
    /// A shipped look drawn lit live (decision 0023): its recipe's material in its recipe's
    /// environment, with that environment's key light.
    Lit(String),
}

impl Look {
    /// `zebra`, `clay`, `matcap:<name>` or `lit:<name>` (lowercase letters, digits and
    /// dashes, as in `matcap:skin-4`).
    pub fn parse(text: &str) -> Result<Look, String> {
        let named = |prefix: &str| {
            text.strip_prefix(prefix).filter(|name| {
                let allowed = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
                !name.is_empty() && name.bytes().all(allowed)
            })
        };
        match (text, named("matcap:"), named("lit:")) {
            ("zebra", ..) => Ok(Look::Zebra),
            ("clay", ..) => Ok(Look::Clay),
            (_, Some(name), _) => Ok(Look::MatCap(name.into())),
            (_, _, Some(name)) => Ok(Look::Lit(name.into())),
            _ => Err(format!(
                "unknown look {text:?}: zebra, clay, matcap:<name> or lit:<name>"
            )),
        }
    }

    /// The look as scene files and manifests write it: `zebra`, `clay`, `matcap:<name>` or
    /// `lit:<name>`.
    pub fn text(&self) -> String {
        match self {
            Look::MatCap(name) => format!("matcap:{name}"),
            Look::Lit(name) => format!("lit:{name}"),
            other => other.file_name(),
        }
    }

    /// The look in file names: `zebra`, `clay`, `matcap-<name>` or `lit-<name>`.
    pub fn file_name(&self) -> String {
        match self {
            Look::Zebra => "zebra".into(),
            Look::Clay => "clay".into(),
            Look::MatCap(name) => format!("matcap-{name}"),
            Look::Lit(name) => format!("lit-{name}"),
        }
    }
}

/// A scene file in `goldens/scenes/`, as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneFile {
    about: String,
    #[serde(default)]
    seed: u64,
    #[serde(default = "iso")]
    view: String,
    looks: Vec<String>,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    show: Show,
    step: Vec<Step>,
}

fn iso() -> String {
    "iso".into()
}

/// What a scene's parts show on their surface (decision 0039, section 9): a scene file's
/// `[show]` table, every key optional.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Show {
    /// Parts show their point colors, as a part does until Colorize is turned off.
    pub colors: bool,
    /// The view shows face group colors instead (`view.display`'s `face_groups`).
    pub face_groups: bool,
    /// MatCaps read mirrored (`view.display`'s `matcap_flip`).
    pub matcap_flip: bool,
}

impl Default for Show {
    fn default() -> Self {
        Show {
            colors: true,
            face_groups: false,
            matcap_flip: false,
        }
    }
}

/// A golden scene: how to build it and how to look at it.
#[derive(Clone, Debug)]
pub struct GoldenScene {
    /// The file stem; pictures are named `<scene>.<look>`.
    pub name: String,
    /// What the scene checks, in a sentence.
    pub about: String,
    /// The session seed (commands with random numbers depend on it).
    pub seed: u64,
    /// The camera: a preset fitted to every part.
    pub view: Preset,
    pub looks: Vec<Look>,
    /// Every part must be a closed manifold (a mesh check for approval).
    pub closed: bool,
    pub show: Show,
    pub steps: Vec<Step>,
}

impl GoldenScene {
    /// Reads a scene file.
    pub fn parse(name: &str, text: &str) -> Result<GoldenScene, String> {
        let file: SceneFile = toml::from_str(text).map_err(|e| format!("{name}: {e}"))?;
        let view = Preset::ALL
            .into_iter()
            .find(|p| p.name() == file.view)
            .ok_or_else(|| format!("{name}: unknown view {:?}", file.view))?;
        let looks = (file.looks.iter())
            .map(|l| Look::parse(l))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{name}: {e}"))?;
        if looks.is_empty() || file.step.is_empty() {
            return Err(format!(
                "{name}: a scene needs at least one look and one step"
            ));
        }
        Ok(GoldenScene {
            name: name.into(),
            about: file.about,
            seed: file.seed,
            view,
            looks,
            closed: file.closed,
            show: file.show,
            steps: file.step,
        })
    }

    /// Every `*.toml` scene in `dir`, by name.
    pub fn load_all(dir: &Path) -> Result<Vec<GoldenScene>, String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut paths: Vec<_> = (entries.filter_map(Result::ok))
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        paths.sort();
        paths
            .iter()
            .map(|p| {
                let text =
                    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned());
                GoldenScene::parse(&stem.unwrap_or_default(), &text)
            })
            .collect()
    }

    /// The picture name for one of the scene's looks.
    pub fn picture(&self, look: &Look) -> String {
        format!("{}.{}", self.name, look.file_name())
    }

    /// Closed scenes are drawn front faces only. A closed mesh seen from outside never shows a
    /// back face; drawing both sides only adds noise between renderers, where a back face can
    /// win the depth test at a silhouette on one and lose it on another.
    pub fn sides(&self) -> Sides {
        if self.closed {
            Sides::Front
        } else {
            Sides::Both
        }
    }

    /// How [`render`] draws the scene: its camera preset, the sides drawn and what the parts
    /// show.
    pub fn how(&self) -> (Preset, Sides, Show) {
        (self.view, self.sides(), self.show)
    }

    /// Runs the steps in a fresh session with the scene's seed and returns every part.
    pub fn run(&self) -> Result<Scene, String> {
        let mut session = Session::headless_seeded(self.seed);
        for (i, step) in self.steps.iter().enumerate() {
            let args = if step.args.is_null() {
                json!({})
            } else {
                step.args.clone()
            };
            session.run(&step.command, args).map_err(|e| {
                let n = i + 1;
                format!(
                    "{}: step {n} ({}) failed: {}",
                    self.name, step.command, e.message
                )
            })?;
        }
        Ok(session.scene())
    }

    /// What the mesh validator finds wrong with the parts: always the basic checks, and closed
    /// manifolds where the scene says so. One line per part and defect, at most five per part.
    pub fn defects(&self, parts: &Scene) -> Vec<String> {
        let expect = Expect {
            manifold: self.closed,
            closed: self.closed,
            euler: None,
        };
        let mut out = Vec::new();
        for (name, mesh) in parts {
            if let Err(found) = validate(mesh, expect) {
                out.extend(found.iter().take(5).map(|d| format!("{name}: {d:?}")));
            }
        }
        out
    }
}

/// Renders `parts` at [`SIZE`] with one camera fitted to all of them from `view`, drawing
/// `sides` and what `show` asks, shaded by `look` (look recipes are read from `matcaps`).
pub fn render(
    r: &Renderer,
    parts: &Scene,
    (view, sides, show): (Preset, Sides, Show),
    look: &Look,
    matcaps: &Path,
) -> Result<Image, String> {
    let text = |e: gs_render::RenderError| e.to_string();
    let mut meshes = (parts.values())
        .map(|m| r.upload(m, UploadOptions::default()))
        .map(|m| m.map(|m| m.painted(show.colors)))
        .collect::<Result<Vec<GpuMesh>, _>>()
        .map_err(text)?;
    for mesh in &mut meshes {
        mesh.set_sides(sides);
    }
    let points: Vec<[f32; 3]> = meshes.iter().flat_map(GpuMesh::fit_points).collect();
    let recipe = |name: &str| {
        let path = matcaps.join(format!("{name}.toml"));
        std::fs::read_to_string(&path)
            .map_err(|e| format!("{}: {e}", path.display()))
            .and_then(|t| Recipe::parse(&t))
    };
    let (matcap, lit, environment);
    let mut light = Light::default();
    let shading = match look {
        Look::Zebra => Shading::Zebra,
        Look::Clay => Shading::Clay,
        Look::MatCap(name) => {
            matcap = r.upload_bake(&*bake(&recipe(name)?).map_err(text)?);
            Shading::MatCap(&matcap)
        }
        Look::Lit(name) => {
            lit = recipe(name)?;
            let prepared = LitEnvironment::shipped(&lit.environment).map_err(text)?;
            environment = r.upload_environment(&prepared);
            light.environment = Some(&environment);
            Shading::Lit(&lit.material)
        }
    };
    let refs: Vec<&GpuMesh> = meshes.iter().collect();
    let frame = Frame {
        meshes: &refs,
        // Live subsurface on everywhere: `auto` leaves it off on an integrated GPU, and a
        // golden must draw the same picture on every adapter (lavapipe's `auto` is on).
        view: View {
            light,
            subsurface: Subsurface {
                mode: SubsurfaceMode::On,
                ..Subsurface::DEFAULT
            },
            face_groups: show.face_groups,
            matcap_flip: show.matcap_flip,
            ..View::new(Camera::fitted(view, &points, 1.0), shading)
        },
        viewport: [0, 0, SIZE, SIZE],
    };
    let target = r.frame_target(RenderSize::square(SIZE)).map_err(text)?;
    (r.draw_frame(&target, &frame, &mut r.ui_painter(), &UiLayer::default()))
        .and_then(|()| r.read_frame(&target))
        .map_err(text)
}
