//! The session's light on one device (decision 0023): the environment `view.environment`
//! chose and the lights `light.set` set, as gs-render lights a view with them. Every view draws
//! through [`crate::draw::Look::view`], so the window and `view.snapshot` light alike.
//!
//! Environments are prepared once (seconds for a file: [`prepare`], which needs no device, so
//! the window is not held while it runs) and uploaded once per device, a few kept at a time.

use std::path::Path;
use std::sync::Arc;

use gs_engine::{DisplaySettings, Environment, EnvironmentInfo, KeyLight};
use gs_render::shading::hdr::Equirect;
use gs_render::shading::light::{LightSetting, LightValues, Lights};
use gs_render::shading::lit::LitEnvironment;
use gs_render::{DEFAULT_ENVIRONMENT, GpuEnvironment, Light, Renderer};

/// Environments kept prepared and uploaded at once: the shown one with room to switch.
const KEPT: usize = 4;

/// The environments of one device, and the lights as last drawn.
#[derive(Default)]
pub struct Lighting {
    /// Environments read from files, prepared, most recently used first. gs-render keeps the
    /// shipped ones.
    files: Vec<(Environment, Arc<LitEnvironment>)>,
    /// Environments on the device, most recently used first; `Own` is stored as the shipped
    /// environment it stands for.
    uploaded: Vec<(Environment, GpuEnvironment)>,
    lights: Lights,
}

impl Lighting {
    /// Keeps `prepared` as `environment`, so the next frame lights with it without reading
    /// the file again.
    pub fn keep(&mut self, environment: &Environment, prepared: Arc<LitEnvironment>) {
        if matches!(environment, Environment::File { .. }) {
            self.files.retain(|(e, _)| e != environment);
            self.files.truncate(KEPT - 1);
            self.files.insert(0, (environment.clone(), prepared));
        }
    }

    /// The light of a lit view: `display`'s environment (`own` for a look's own) on
    /// `renderer`'s device, and its lights.
    pub(crate) fn light(
        &mut self,
        renderer: &Renderer,
        display: &DisplaySettings,
        own: &str,
    ) -> Result<Light<'_>, String> {
        let key = match &display.environment {
            Environment::Own => Environment::Shipped(own.into()),
            other => other.clone(),
        };
        let entry = if let Some(at) = self.uploaded.iter().position(|(e, _)| *e == key) {
            self.uploaded.remove(at)
        } else {
            let prepared = match self.files.iter().find(|(e, _)| *e == key) {
                Some((_, p)) => Arc::clone(p),
                None => prepare(&key)?,
            };
            self.keep(&key, Arc::clone(&prepared));
            let gpu = renderer.upload_environment(&prepared);
            (key, gpu)
        };
        self.uploaded.truncate(KEPT - 1);
        self.uploaded.insert(0, entry);
        self.lights = lights(display);
        Ok(Light {
            environment: Some(&self.uploaded[0].1),
            lights: &self.lights,
        })
    }
}

/// Reads and prepares `environment` for Lit mode (`Own`: the default studio). Seconds for a
/// file; a shipped environment is prepared once per process.
pub fn prepare(environment: &Environment) -> Result<Arc<LitEnvironment>, String> {
    let text = |e: gs_render::RenderError| e.to_string();
    match environment {
        Environment::Own => LitEnvironment::shipped(DEFAULT_ENVIRONMENT).map_err(text),
        Environment::Shipped(name) => LitEnvironment::shipped(name).map_err(text),
        Environment::File { path, turn, .. } => {
            let bytes = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let image = Equirect::read_hdr(&bytes).map_err(|e| format!("{path}: {e}"))?;
            let turn = turn.map_or_else(|| image.key_turn(), f32::to_radians);
            let name = Path::new(path)
                .file_name()
                .map_or(path.as_str(), |n| n.to_str().unwrap_or(path.as_str()));
            Ok(Arc::new(LitEnvironment::of(&image.environment(name, turn))))
        }
    }
}

/// What `view.environment` reports of `environment`, prepared.
pub fn info(environment: &Environment, prepared: &LitEnvironment) -> EnvironmentInfo {
    let key = prepared.key.map(|k| {
        let v = LightValues::of(&LightSetting::default(), Some(&k), true);
        KeyLight {
            direction: v.direction,
            size_deg: v.size_deg,
            intensity: v.intensity,
            color: v.color,
        }
    });
    EnvironmentInfo {
        name: (!matches!(environment, Environment::Own)).then(|| prepared.name.clone()),
        key,
    }
}

/// `display`'s lights as gs-render takes them.
fn lights(display: &DisplaySettings) -> Lights {
    Lights {
        ambient: display.ambient,
        each: display.lights.map(|l| LightSetting {
            on: l.on,
            color: l.color,
            intensity: l.intensity,
            direction: l.direction,
            size_deg: l.size_deg,
        }),
    }
}
