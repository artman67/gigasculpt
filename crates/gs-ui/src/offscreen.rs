//! Offscreen looks (`view.snapshot`, `view.contact_sheet`) and the material palette the engine
//! hands over (decision 0043): the engine's [`RenderHost`] for the window, drawing on the
//! window's own device with the window's look, and for headless sessions, which open a device
//! at the first look.
//!
//! Both draw what the engine hands over (the parts' mesh stores) the way the window draws its
//! views ([`crate::draw::look`], decision 0024).

use gs_engine::{DisplaySettings, Environment, EnvironmentInfo, MaterialPalette, RenderHost};
use gs_engine::{RenderRequest, Screenshot};
use gs_render::{AdapterKind, MeshStore, Renderer};

use crate::draw::{look, subsurface};
use crate::host::Host;
use crate::lighting::{self, Lighting};
use crate::matcaps::MatCaps;

impl RenderHost for Host {
    fn render(
        &mut self,
        parts: &[&MeshStore],
        request: &RenderRequest,
    ) -> Option<Result<Screenshot, String>> {
        let mut studio = self.lock();
        let render = studio.render.as_mut()?;
        let looks = (&mut render.matcaps, &mut render.lighting);
        Some(look(&render.renderer, looks, parts, request))
    }

    /// Prepared without holding the window, which keeps drawing meanwhile; the next frame
    /// lights with it.
    fn environment(
        &mut self,
        environment: &Environment,
    ) -> Option<Result<EnvironmentInfo, String>> {
        self.lock().render.as_ref()?;
        let prepared = match lighting::prepare(environment) {
            Ok(prepared) => prepared,
            Err(e) => return Some(Err(e)),
        };
        let info = lighting::info(environment, &prepared);
        let mut studio = self.lock();
        studio.render.as_mut()?.lighting.keep(environment, prepared);
        studio.dirty = true;
        Some(Ok(info))
    }

    /// The active material is baked for the window's device right away, so the next frame
    /// shows it.
    fn materials(&mut self, palette: &MaterialPalette) -> Option<Result<(), String>> {
        let mut studio = self.lock();
        let render = studio.render.as_mut()?;
        render.matcaps.show(palette);
        let drawn = render.matcaps.gpu(&render.renderer).map(|_| ());
        studio.dirty = true;
        Some(drawn)
    }

    fn subsurface_runs(&mut self, display: &DisplaySettings) -> Option<bool> {
        let integrated = self.lock().render.as_ref()?.renderer.gpu().integrated();
        Some(subsurface(display).mode.runs(integrated))
    }
}

/// The adapter a headless session renders on when none is named: lavapipe on Linux, which draws
/// the same bytes every run (the golden pictures depend on it); elsewhere none, so the first
/// adapter wgpu offers.
const DEFAULT: Option<AdapterKind> = if cfg!(target_os = "linux") {
    Some(AdapterKind::Lavapipe)
} else {
    None
};

/// A headless session's renderer. It opens its device at the first look, so a session that
/// never looks never pays for one.
pub struct Offscreen {
    kind: Option<AdapterKind>,
    device: Option<Renderer>,
    matcaps: MatCaps,
    lighting: Lighting,
}

impl Offscreen {
    /// Renders on `adapter` (`lavapipe`, `nvidia` or `radv`), else on [`DEFAULT`].
    pub fn new(adapter: Option<&str>) -> Result<Self, String> {
        let unknown = |name| format!("unknown adapter {name}: use lavapipe, nvidia or radv");
        let kind = match adapter {
            Some(name) => Some(AdapterKind::from_name(name).ok_or_else(|| unknown(name))?),
            None => DEFAULT,
        };
        Ok(Self {
            kind,
            device: None,
            matcaps: MatCaps::default(),
            lighting: Lighting::default(),
        })
    }

    /// For `gs_engine::cli::main_with`: the renderer of a headless session.
    pub fn host(adapter: Option<&str>) -> Result<Box<dyn RenderHost>, String> {
        Ok(Box::new(Self::new(adapter)?))
    }
}

impl RenderHost for Offscreen {
    fn render(
        &mut self,
        parts: &[&MeshStore],
        request: &RenderRequest,
    ) -> Option<Result<Screenshot, String>> {
        if self.device.is_none() {
            match Renderer::new(self.kind) {
                Ok(renderer) => self.device = Some(renderer),
                Err(e) => {
                    let kind = self.kind.map_or("an adapter", AdapterKind::name);
                    return Some(Err(format!("cannot open {kind}: {e}")));
                }
            }
        }
        let renderer = self.device.as_ref()?;
        Some(look(
            renderer,
            (&mut self.matcaps, &mut self.lighting),
            parts,
            request,
        ))
    }

    /// The palette is only kept: the active material is baked at the next look, on the device
    /// that look opens.
    fn materials(&mut self, palette: &MaterialPalette) -> Option<Result<(), String>> {
        self.matcaps.show(palette);
        Some(Ok(()))
    }

    /// Prepared now, so a file that cannot be read is refused now; uploaded at the next look.
    fn environment(
        &mut self,
        environment: &Environment,
    ) -> Option<Result<EnvironmentInfo, String>> {
        let prepared = lighting::prepare(environment);
        Some(prepared.map(|p| {
            let info = lighting::info(environment, &p);
            self.lighting.keep(environment, p);
            info
        }))
    }

    /// On the device, or before the first look on the adapter it will open.
    fn subsurface_runs(&mut self, display: &DisplaySettings) -> Option<bool> {
        let integrated = match &self.device {
            Some(renderer) => renderer.gpu().integrated(),
            None => AdapterKind::integrated(self.kind)?,
        };
        Some(subsurface(display).mode.runs(integrated))
    }
}
