//! The window minus the operating system's window: the interface, its egui context and the
//! renderer that draws the parts and the interface into a frame. The real window adds SDL and
//! the surface (see `app.rs`); tests and the engine's window commands use this directly, so a
//! screenshot is drawn by exactly the code that draws the window.

mod gesture;
mod input;
mod path;
mod pen;
mod render;

use std::collections::BTreeMap;
use std::time::Duration;

use gs_engine::Shading as Shade;
use gs_engine::{CallReport, Screenshot, UiDescription, ViewportInfo};
use gs_platform::Input;
use gs_render::matcap::DEFAULT_LOOK;
use gs_render::{FrameTarget, RenderSize, UiLayer, UiTexture};

use input::InputState;
pub(crate) use path::draw_path;
pub use render::Render;
use render::{Slot, in_view, pixels};

use crate::draw::Look;
use crate::glue::{events, layer, texture_key};
use crate::perf::Perf;
use crate::pieces::picker;
use crate::reference::picture;
use crate::shell::Shell;

/// The window's state: everything but SDL and the surface.
pub struct Studio {
    pub shell: Shell,
    pub render: Option<Render>,
    pub perf: Perf,
    /// With no renderer, the parts the 3D view would draw as the last call left them, by name
    /// (see `render::in_view`): what `ui.describe` lists, as a renderer's placed copies are.
    drawn: Vec<String>,
    pub(crate) ctx: egui::Context,
    /// The input waiting for the next frame, and where the pen is in a stroke or a gesture.
    input: InputState,
    size: [u32; 2],
    density: f32,
    /// The display's content scale beyond `density` (see [`Studio::content_scale`]).
    content: f32,
    /// Textures egui freed last frame, freed at the start of the next.
    freed: Vec<UiTexture>,
    /// Each Reference View shown, by name, and the picture size it was last framed for.
    reference_framed: BTreeMap<String, [u32; 2]>,
    /// Something changed that the screen does not show yet.
    pub dirty: bool,
    /// The window's clock, in nanoseconds (see [`Studio::at`]).
    now_ns: u64,
    /// When the interface asked to draw again, by the same clock.
    wake: Option<u64>,
}

/// What one frame asks of the window. The commands it asks for wait in the shell.
#[derive(Debug, Default)]
pub struct Frame {
    /// A text field has the keyboard: the window turns text input on.
    pub typing: bool,
    /// egui wants another frame soon (an animation, a blinking cursor).
    pub again: bool,
}

impl Studio {
    /// A window of `size` physical pixels at `density` pixels per point.
    pub fn new(shell: Shell, render: Option<Render>, size: [u32; 2], density: f32) -> Self {
        Self {
            shell,
            render,
            perf: Perf::default(),
            drawn: Vec::new(),
            ctx: egui::Context::default(),
            input: InputState::default(),
            size,
            density,
            content: 1.0,
            freed: Vec::new(),
            reference_framed: BTreeMap::new(),
            dirty: true,
            now_ns: 0,
            wake: None,
        }
    }

    /// The window's clock reads `now_ns`, in nanoseconds: gs-platform's clock, which the pen
    /// samples' times are on, in the window; tests move it. It never runs back. egui's time is
    /// this clock, so every timer the interface keeps (a tap, a slider key's activation, a long
    /// press, a tooltip) counts seconds however far apart frames are. Once the time the
    /// interface asked to draw again by comes, the studio is dirty: the next frame draws with
    /// no input.
    pub fn at(&mut self, now_ns: u64) {
        self.now_ns = self.now_ns.max(now_ns);
        if self.wake.is_some_and(|w| w <= self.now_ns) {
            self.wake = None;
            self.dirty = true;
        }
    }

    /// The window's clock, in nanoseconds, as last read (see [`Studio::at`]).
    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    /// Physical pixels per egui point: the display's density and content scale, times the
    /// layout's scale.
    fn ppp(&self) -> f32 {
        self.density * self.content * self.shell.layout.layout.ui_scale
    }

    /// How large the gizmo is drawn and picked on this screen: its size from the layout state at
    /// this screen's pixels per point. `gizmo.describe` draws with the same.
    pub fn gizmo_style(&self) -> gs_core::gizmo::Style {
        self.shell.layout.layout.gizmo.style(self.ppp())
    }

    /// The theme this window draws with: the one its last frame showed.
    pub fn theme(&self) -> &'static crate::Theme {
        crate::theme::theme(&self.ctx)
    }

    /// The display's content scale beyond the pixel density, as SDL reports it: the scale the
    /// system asks for that the pixels do not already carry (1 on Wayland, where the density
    /// does; the desktop's scale on X11). The interface scale multiplies it.
    pub fn content_scale(&mut self, content: f32) {
        let content = if content.is_finite() && content > 0.0 {
            content
        } else {
            1.0
        };
        if content.total_cmp(&self.content).is_ne() {
            self.content = content;
            self.dirty = true;
        }
    }

    /// The memory the app holds now, in mebibytes (`status::resident_mb`), for the status line.
    /// The window measures it about twice a second; a change of a mebibyte or more redraws.
    pub fn memory(&mut self, mb: Option<u64>) {
        if mb != self.shell.status.memory_mb {
            self.shell.status.memory_mb = mb;
            self.dirty = true;
        }
    }

    /// The window changed size or moved to a display of another density.
    pub fn resize(&mut self, size: [u32; 2], density: f32) {
        self.size = size;
        self.density = density;
        self.dirty = true;
    }

    /// Mouse, keyboard and window input for the next frame; `held` are the modifiers now.
    pub fn input(&mut self, inputs: &[Input], held: gs_platform::Modifiers) {
        let inputs = &self.stroke_keys(inputs);
        let held = self.in_effect(inputs, crate::glue::modifiers(held));
        // egui only learns which modifiers are held from this event (navigation needs it). Sent
        // only when they change: an event asks egui for frames.
        let changed = held != self.input.held;
        if changed {
            self.input.held = held;
            self.input.pending.push(egui::Event::ModifiersChanged(held));
            self.alt_now();
        }
        let ppp = self.ppp();
        self.input.pending.extend(events(inputs, ppp, held));
        self.dirty |= !inputs.is_empty() || changed;
    }

    /// Takes in what a call did, and brings the GPU copies of the parts up to date. A press
    /// the engine found on empty canvas navigates, unless it was a Ctrl gesture's.
    pub fn after_call(&mut self, report: &CallReport) {
        self.shell.after_call(report);
        if !self.background_answer(report.on_canvas) {
            self.shell.canvas |= report.on_canvas;
        }
        if let Some(render) = &mut self.render {
            render.sync(report, &crate::theme::theme(&self.ctx).colors);
        } else {
            let drawn = report.parts.iter().filter(|p| in_view(p));
            self.drawn = drawn.map(|p| p.name.to_string()).collect();
        }
        self.dirty = true;
    }

    /// Runs the interface for one frame and, with a renderer, draws the parts and the
    /// interface into the frame target. egui's clock is the window's ([`Studio::at`]), so the
    /// same input at the same times always draws the same frames.
    pub fn frame(&mut self) -> Result<Frame, String> {
        let (ui, frame) = self.interface();
        self.paint(&ui)?;
        Ok(frame)
    }

    /// Runs the interface for one frame: the layer to draw over the parts, and what the frame
    /// asks of the window. The commands it asks for wait in the shell; the window runs them
    /// before it paints, so a navigation drag shows in the frame it happened in. When egui asks
    /// to draw again (at once after input, or after a delay: a slider key's activation), the
    /// studio wakes at that time on its clock ([`Studio::at`]).
    pub fn interface(&mut self) -> (UiLayer, Frame) {
        let ppp = self.ppp();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(self.size[0] as f32, self.size[1] as f32) / ppp,
            )),
            time: Some(self.now_ns as f64 / 1e9),
            events: std::mem::take(&mut self.input.pending),
            ..Default::default()
        };
        raw.viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let typing = self.ctx.egui_wants_keyboard_input();
        self.shell.early(&mut raw.events, typing, ppp);
        picker::still(&self.ctx, picker::stamp(&self.shell));
        self.shell.status.frame_ms = self.perf.last_frame_ms();
        if let Some(render) = &self.render {
            self.shell.looks = render.matcaps.list();
            (self.shell.materials).clone_from(&render.matcaps.palette().materials);
        }
        let shell = &mut self.shell;
        let output = self.ctx.run_ui(raw, |ui| shell.ui(ui));
        // No request is Duration::MAX.
        let delay = (output.viewport_output.values())
            .map(|v| v.repaint_delay)
            .min()
            .filter(|d| *d < Duration::MAX);
        let again = delay.is_some_and(|d| d.is_zero());
        self.wake = delay.map(|d| {
            let ns = u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
            self.now_ns.saturating_add(ns)
        });
        let ui = layer(&self.ctx, output, &mut self.freed);
        let frame = Frame {
            typing: self.ctx.egui_wants_keyboard_input(),
            again,
        };
        self.reframe_reference();
        (ui, frame)
    }

    /// Frames each Reference View again whenever its picture takes a new size and nothing is
    /// being dragged: its first framing could only guess the picture's shape, and a reference
    /// opening or closing splits the place again. Skipped while its zoom or slide follows main.
    /// A `view.frame` call like any other, logged with the size.
    fn reframe_reference(&mut self) {
        let sizes = self.reference_sizes();
        (self.reference_framed).retain(|view, _| sizes.iter().any(|(v, _)| v == view));
        if self.ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        for (view, size) in sizes {
            if self.reference_framed.get(&view) == Some(&size) {
                continue;
            }
            let shown = self.shell.reference.shown.iter();
            let linked = (shown.clone().find(|s| s.setup.view == view))
                .is_some_and(|s| s.setup.link.zoom || s.setup.link.pan);
            if !linked {
                let args = serde_json::json!({ "view": view, "viewport": size });
                self.shell
                    .queue(crate::record::with_args("view.frame", args));
            }
            self.reference_framed.insert(view, size);
        }
    }

    /// Draws the parts through `main`'s camera and `ui` over them. A still view adds a jittered
    /// sample each frame until its picture has settled (decision 0019), every Reference View's
    /// picture as well, so the studio stays dirty, asking for more frames, until all have.
    pub fn paint(&mut self, ui: &UiLayer) -> Result<(), String> {
        // A swatch's drag with Alt reads the pixel drawn under the pen (decision 0039, s. 11).
        let pen = (self.shell.cursor.pen).filter(|_| picker::pixel_wanted(&self.ctx));
        if let (Some(s), Some(t)) = (pen, self.render.as_ref().and_then(|r| r.target())) {
            t.pick([s.x as u32, s.y as u32], self.shell.over_view(&s));
        }
        self.draw(ui)?;
        let picked = (self.render.as_ref()).and_then(|r| Some(r.renderer.picked(r.target()?)));
        picker::drawn(
            &self.ctx,
            picked.transpose().map_err(|e| e.to_string())?.flatten(),
        );
        let settled = self.render.as_ref().is_none_or(|r| {
            let pictures = r.references.values().map(|p| &p.target);
            (pictures.chain([&r.target])).all(|t| t.as_ref().is_none_or(FrameTarget::settled))
        });
        self.dirty = !settled;
        Ok(())
    }

    /// The 3D view's size in pixels, as last laid out (laying it out first if no frame has
    /// run yet); `None` when it has no area.
    pub fn viewport_size(&mut self) -> Option<[u32; 2]> {
        if !self.shell.viewport().is_positive()
            && let Err(problem) = self.settle()
        {
            self.shell.status.problem = Some(problem);
        }
        let size = RenderSize {
            width: self.size[0].max(1),
            height: self.size[1].max(1),
            samples: 1,
        };
        let [_, _, w, h] = pixels(self.shell.viewport(), self.ppp(), size);
        (w > 0 && h > 0).then_some([w, h])
    }

    /// Runs frames until egui has nothing left to settle (a window's first-show sizing pass),
    /// at most four. Commands the frames ask for wait in the shell (`take_commands`).
    fn settle(&mut self) -> Result<(), String> {
        for _ in 0..4 {
            if !self.frame()?.again {
                break;
            }
        }
        Ok(())
    }

    /// Each Reference View picture's size in pixels, as last laid out, by view name; a picture
    /// with no area is left out.
    pub fn reference_sizes(&self) -> Vec<(String, [u32; 2])> {
        let size = RenderSize {
            width: self.size[0].max(1),
            height: self.size[1].max(1),
            samples: 1,
        };
        (self.shell.reference.pictures.iter())
            .filter_map(|(view, rect)| {
                let [_, _, w, h] = pixels(*rect, self.ppp(), size);
                (w > 0 && h > 0).then(|| (view.clone(), [w, h]))
            })
            .collect()
    }

    /// Draws each Reference View's picture into its own frame, which the interface shows as
    /// its [`picture`] texture (decisions 0020, 0024), then the parts in the 3D view's area with
    /// the interface over them: all through [`Render::draw_view`]. A reference no longer shown
    /// has its frame dropped and its texture freed.
    fn draw(&mut self, ui: &UiLayer) -> Result<(), String> {
        let ppp = self.ppp();
        let window = self.size;
        let Some(render) = &mut self.render else {
            return Ok(());
        };
        let size = RenderSize {
            width: window[0].max(1),
            height: window[1].max(1),
            samples: render.samples,
        };
        let display = &self.shell.state.display;
        let reference = &self.shell.reference;
        for (view, rect) in &reference.pictures {
            let Some(shown) = reference.shown.iter().find(|s| &s.setup.view == view) else {
                continue;
            };
            let [_, _, w, h] = pixels(*rect, ppp, size);
            if w == 0 || h == 0 {
                continue;
            }
            // A reference draws in the session's shading with its own MatCap.
            let look = Look {
                matcap: shown.setup.matcap.as_deref(),
                wireframe: shown.setup.wireframe,
                ..Look::main(shown.camera.camera, display)
            };
            let at = RenderSize {
                width: w,
                height: h,
                samples: render.samples,
            };
            let slot = Slot::Reference {
                view,
                live: shown.live,
            };
            render.draw_view(slot, (at, [0, 0, w, h]), &look, &UiLayer::default())?;
            if let Some(target) = (render.references.get(view)).and_then(|p| p.target.as_ref()) {
                let key = texture_key(picture(view));
                render.renderer.show_frame(&mut render.painter, key, target);
            }
        }
        let shown = |view: &String| reference.shown.iter().any(|s| &s.setup.view == view);
        let gone: Vec<String> = (render.references.keys())
            .filter(|view| !shown(view))
            .cloned()
            .collect();
        let mut freed = Vec::new();
        for view in gone {
            let pictured = render.references.remove(&view);
            if pictured.is_some_and(|p| p.target.is_some()) {
                freed.push(UiTexture::Free(texture_key(picture(&view))));
            }
        }
        let with_freed;
        let ui = if freed.is_empty() {
            ui
        } else {
            let mut layer = ui.clone();
            layer.textures.extend(freed);
            with_freed = layer;
            &with_freed
        };
        let viewport = pixels(self.shell.viewport(), ppp, size);
        // Dots (`view.wire`): the wire shows only while the view moves.
        let moving = !display.wire.dots || self.shell.nav.dragging().is_some();
        // A material the pen rests on in the Library, for as long as it rests (decision 0043,
        // step (d)); one the palette no longer has draws nothing new.
        let previewing = self.shell.library.previewing.as_deref();
        let previewing = previewing.filter(|n| render.matcaps.recipe(Some(n)).is_ok());
        let look = Look {
            wireframe: self.shell.state.view.wireframe && moving,
            matcap: previewing,
            ..Look::main(self.shell.main.camera, display)
        };
        render.draw_view(Slot::Main, (size, viewport), &look, ui)
    }

    /// The interface as `ui.describe` reports it, after the input so far. A frame that fails
    /// to draw is reported in the status bar.
    pub fn describe(&mut self) -> UiDescription {
        if let Err(problem) = self.settle() {
            self.shell.status.problem = Some(problem);
        }
        let ppp = self.ppp();
        let rect = self.shell.viewport() * ppp;
        UiDescription {
            window: true,
            size: self.size,
            pixel_density: self.density,
            panels: self.shell.panels().to_vec(),
            cursor: self.shell.cursor.pen,
            viewport: Some(ViewportInfo {
                rect: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                parts: self.render.as_ref().map_or_else(
                    || self.drawn.clone(),
                    |r| {
                        r.meshes
                            .iter()
                            .filter(|m| m.2.shown())
                            .map(|(name, _, _)| name.clone())
                            .collect()
                    },
                ),
                shading: self.shading(),
                camera: self.shell.main.camera.into(),
            }),
            perf: Some(self.perf.info()),
        }
    }

    /// The 3D view's shading as `ui.describe` names it: `matcap <look>`, `lit <look>`,
    /// `clay`, `zebra` or `normals`.
    fn shading(&self) -> String {
        let look = (self.render.as_ref()).map_or(DEFAULT_LOOK, |r| r.matcaps.selected());
        match self.shell.state.display.shading {
            Shade::Matcap => format!("matcap {look}"),
            Shade::Lit => format!("lit {look}"),
            Shade::Clay => "clay".into(),
            Shade::Zebra => "zebra".into(),
            Shade::Normals => "normals".into(),
        }
    }

    /// Draws a frame and reads it back as a PNG.
    pub fn screenshot(&mut self) -> Result<Screenshot, String> {
        self.settle()?;
        let render = self.render.as_ref().ok_or("this window draws nothing")?;
        let target = render.target().ok_or("no frame yet")?;
        let image = render
            .renderer
            .read_frame(target)
            .map_err(|e| e.to_string())?;
        Ok(Screenshot {
            width: image.width,
            height: image.height,
            png: image.png().map_err(|e| e.to_string())?,
        })
    }
}
