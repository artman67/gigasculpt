//! Our own glue between the platform and egui (no eframe, no egui-winit): platform input to
//! egui events, the mouse's held buttons to pen samples, pen samples to pointer events, and
//! egui's output to the renderer's interface layer.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, TouchPhase, epaint};
use gs_platform::{Button, Input};
use gs_protocol::PenSample;
use gs_render::{UiImage, UiLayer, UiMesh, UiTexture, UiVertex};

/// egui modifiers from the platform's.
pub fn modifiers(m: gs_platform::Modifiers) -> Modifiers {
    Modifiers {
        alt: m.alt,
        ctrl: m.ctrl,
        shift: m.shift,
        mac_cmd: m.logo,
        command: m.ctrl,
    }
}

/// egui's key for an SDL key name, where the names differ.
pub fn key(name: &str) -> Option<Key> {
    let renamed = match name {
        "Return" | "Keypad Enter" => "Enter",
        "Left" => "ArrowLeft",
        "Right" => "ArrowRight",
        "Up" => "ArrowUp",
        "Down" => "ArrowDown",
        other => other,
    };
    Key::from_name(renamed)
}

/// egui events for the non-pen input of one frame. `ppp` is pixels per point.
pub fn events(inputs: &[Input], ppp: f32, held: Modifiers) -> Vec<Event> {
    let at = |x: f64, y: f64| Pos2::new(x as f32 / ppp, y as f32 / ppp);
    inputs
        .iter()
        .filter_map(|input| match input {
            Input::MouseMove { x, y } => Some(Event::PointerMoved(at(*x, *y))),
            Input::MouseButton { button, down, x, y } => Some(Event::PointerButton {
                pos: at(*x, *y),
                button: match button {
                    Button::Left => PointerButton::Primary,
                    Button::Middle => PointerButton::Middle,
                    Button::Right => PointerButton::Secondary,
                },
                pressed: *down,
                modifiers: held,
            }),
            Input::Wheel { dx, dy } => Some(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(*dx, *dy),
                phase: TouchPhase::Move,
                modifiers: held,
            }),
            Input::Key {
                key: name,
                down,
                repeat,
                modifiers: m,
            } => Some(Event::Key {
                key: key(name)?,
                physical_key: None,
                pressed: *down,
                repeat: *repeat,
                modifiers: modifiers(*m),
            }),
            Input::Text(text) => Some(Event::Text(text.clone())),
            Input::PointerGone => Some(Event::PointerGone),
            Input::Pen(_) | Input::Resized | Input::Close => None,
        })
        .collect()
}

/// The pen samples of one pump, and everything else, both in order.
pub fn split_pen(inputs: &[Input]) -> (Vec<PenSample>, Vec<Input>) {
    let mut samples = Vec::new();
    let mut others = Vec::new();
    for input in inputs {
        match input {
            Input::Pen(sample) => samples.push(*sample),
            other => others.push(other.clone()),
        }
    }
    (samples, others)
}

/// The mouse as a pen (decision 0017 as amended, and decision 0025's Win-feel amendment): its
/// motion, presses and releases become pen samples, so the mouse hovers, sculpts and navigates
/// through the pen's path and shows the brush's circle as a pen does. Motion with no button
/// held is a hovering pen at pressure 0; the left button is the tip with pressure 1 and no
/// tilt, right is barrel button 1 and middle barrel button 2. When the last button lets go the
/// mouse hovers there; it leaves range when it leaves the window. Beside a pen (in range, or
/// for `PEN_QUIET_NS` after its last sample) motion stays mouse motion, so the mouse the
/// system moves with the pen adds no second cursor, and a press then leaves range on release.
#[derive(Clone, Debug, Default)]
pub struct MousePen {
    /// The buttons held, as bits: 1 the left, 2 the right, 4 the middle.
    held: u8,
    /// The last sample's time, so samples of one pump keep their order.
    last_ns: u64,
    /// The mouse is in range as a pen with no button held, at `at`.
    hovering: bool,
    /// Where the mouse last was, in window pixels.
    at: (f64, f64),
    /// A pen is in range, as its last sample said.
    pen_in: bool,
    /// When the last pen sample came.
    pen_ns: Option<u64>,
}

/// How long after a pen's last sample the mouse stays quiet: a pen leaving range moves the
/// system's mouse with it for a moment.
const PEN_QUIET_NS: u64 = 500_000_000;

impl MousePen {
    /// One pump's input with the mouse's motion and buttons turned into pen samples, read at
    /// `now_ns`.
    pub fn convert(&mut self, inputs: &[Input], now_ns: u64) -> Vec<Input> {
        let mut out = Vec::with_capacity(inputs.len());
        for input in inputs {
            match *input {
                Input::Pen(s) => {
                    (self.pen_in, self.pen_ns, self.hovering) =
                        (s.in_proximity, Some(now_ns), false);
                    out.push(Input::Pen(s));
                }
                Input::MouseButton { button, down, x, y } => {
                    let bit = match button {
                        Button::Left => 1,
                        Button::Right => 2,
                        Button::Middle => 4,
                    };
                    self.held = if down {
                        self.held | bit
                    } else {
                        self.held & !bit
                    };
                    self.at = (x, y);
                    out.push(Input::Pen(self.sample(x, y, now_ns)));
                    if self.held == 0 && self.pen_near(now_ns) {
                        let gone = PenSample {
                            in_proximity: false,
                            ..self.sample(x, y, now_ns)
                        };
                        out.extend([Input::Pen(gone), Input::MouseMove { x, y }]);
                    }
                    self.hovering = self.held == 0 && !self.pen_near(now_ns);
                }
                Input::MouseMove { x, y } if self.held != 0 || !self.pen_near(now_ns) => {
                    (self.at, self.hovering) = ((x, y), self.held == 0);
                    out.push(Input::Pen(self.sample(x, y, now_ns)));
                }
                Input::PointerGone if self.hovering => {
                    self.hovering = false;
                    let (x, y) = self.at;
                    let gone = PenSample {
                        in_proximity: false,
                        ..self.sample(x, y, now_ns)
                    };
                    out.extend([Input::Pen(gone), Input::PointerGone]);
                }
                ref other => out.push(other.clone()),
            }
        }
        out
    }

    /// A pen is in range, or its last sample came less than `PEN_QUIET_NS` before `now_ns`.
    fn pen_near(&self, now_ns: u64) -> bool {
        self.pen_in || (self.pen_ns).is_some_and(|t| now_ns.saturating_sub(t) < PEN_QUIET_NS)
    }

    /// A sample at (`x`, `y`) with the buttons held now.
    fn sample(&mut self, x: f64, y: f64, now_ns: u64) -> PenSample {
        self.last_ns = now_ns.max(self.last_ns + 1);
        let tip = self.held & 1 != 0;
        PenSample {
            contact: tip,
            pressure: if tip { 1.0 } else { 0.0 },
            buttons: self.held >> 1,
            ..PenSample::hover(self.last_ns, x, y)
        }
    }
}

/// Pen samples as pointer events: the tip is the primary button, barrel button 1 the
/// secondary one and barrel button 2 the middle one (so pen and mouse navigate alike: 2 pans),
/// leaving range is the pointer leaving.
#[derive(Clone, Debug, Default)]
pub struct PenRouter {
    last: Option<PenSample>,
}

impl PenRouter {
    /// The events for one sample.
    pub fn route(&mut self, s: &PenSample, ppp: f32, held: Modifiers) -> Vec<Event> {
        let pos = Pos2::new(s.x as f32 / ppp, s.y as f32 / ppp);
        // A pen new to range holds nothing yet.
        let before = self.last.unwrap_or(PenSample::hover(s.t_ns, s.x, s.y));
        let barrel = |p: &PenSample, bit: u8| p.in_proximity && p.buttons & bit != 0;
        let tip = |p: &PenSample| p.in_proximity && p.contact;
        let mut out = vec![Event::PointerMoved(pos)];
        let changes = [
            (tip(&before), tip(s), PointerButton::Primary),
            (barrel(&before, 1), barrel(s, 1), PointerButton::Secondary),
            (barrel(&before, 2), barrel(s, 2), PointerButton::Middle),
        ];
        for (was, is, button) in changes {
            if was != is {
                out.push(Event::PointerButton {
                    pos,
                    button,
                    pressed: is,
                    modifiers: held,
                });
            }
        }
        if !s.in_proximity {
            out.push(Event::PointerGone);
        }
        self.last = s.in_proximity.then_some(*s);
        out
    }
}

/// egui's texture id as one number: managed and user textures kept apart.
pub(crate) fn texture_key(id: egui::TextureId) -> u64 {
    match id {
        egui::TextureId::Managed(n) => n * 2,
        egui::TextureId::User(n) => n * 2 + 1,
    }
}

/// egui's paint output as the renderer's interface layer. Textures egui frees are freed at the
/// start of the next layer, after this frame has drawn with them; `freed` carries them over.
pub fn layer(
    ctx: &egui::Context,
    mut output: egui::FullOutput,
    freed: &mut Vec<UiTexture>,
) -> UiLayer {
    let ppp = output.pixels_per_point;
    let mut textures = std::mem::take(freed);
    let mut set: Vec<(egui::TextureId, Vec<epaint::ImageDelta>)> =
        std::mem::take(&mut output.textures_delta.set)
            .into_iter()
            .map(|(id, deltas)| (id, deltas.into_iter().collect()))
            .collect();
    set.sort_by_key(|(id, _)| texture_key(*id));
    for (id, deltas) in set {
        textures.extend(deltas.iter().map(|delta| UiTexture::Set {
            id: texture_key(id),
            at: delta.pos.map(|[x, y]| [x as u32, y as u32]),
            image: image(delta),
        }));
    }
    let mut free: Vec<u64> = std::mem::take(&mut output.textures_delta.free)
        .into_iter()
        .map(texture_key)
        .collect();
    free.sort_unstable();
    freed.extend(free.into_iter().map(UiTexture::Free));
    let meshes = ctx
        .tessellate(std::mem::take(&mut output.shapes), ppp)
        .into_iter()
        .filter_map(|clipped| match clipped.primitive {
            epaint::Primitive::Mesh(mesh) => Some(ui_mesh(clipped.clip_rect, &mesh, ppp)),
            epaint::Primitive::Callback(_) => None,
        })
        .collect();
    UiLayer { textures, meshes }
}

fn image(delta: &epaint::ImageDelta) -> UiImage {
    let epaint::ImageData::Color(color) = &delta.image;
    UiImage {
        width: color.size[0] as u32,
        height: color.size[1] as u32,
        rgba: color
            .pixels
            .iter()
            .flat_map(egui::Color32::to_array)
            .collect(),
        smooth: delta.options.magnification == epaint::textures::TextureFilter::Linear,
    }
}

fn ui_mesh(clip: egui::Rect, mesh: &epaint::Mesh, ppp: f32) -> UiMesh {
    let clip = clip * ppp;
    UiMesh {
        clip: [clip.min.x, clip.min.y, clip.max.x, clip.max.y],
        texture: texture_key(mesh.texture_id),
        vertices: mesh
            .vertices
            .iter()
            .map(|v| UiVertex {
                pos: [v.pos.x * ppp, v.pos.y * ppp],
                uv: [v.uv.x, v.uv.y],
                color: v.color.to_array(),
            })
            .collect(),
        indices: mesh.indices.clone(),
    }
}
