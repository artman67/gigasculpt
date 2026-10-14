//! The pen over the 3D view: each sample goes to the machine that owns its press (the
//! classifier in `gesture.rs`), and the strokes the engine takes from them.

use std::f64::consts::FRAC_PI_4;

use gs_core::Vec3;
use gs_core::brush::{Shape, StrokeSpec};
use gs_engine::{BarrelTwo, PenDown, PenReads, PenStroke, PenTaken};
use gs_platform::Input;
use gs_protocol::{PenSample, PenTool};
use gs_render::RenderSize;
use serde_json::json;

use super::Studio;
use super::gesture::Machine;
use super::input::Background;
use super::path::Path;
use super::render::pixels;
use crate::gizmo::Seen;
use crate::glue;
use crate::held::Held;
use crate::pieces::picker;
use crate::record::with_args;

impl Studio {
    /// Pen samples, through the same pointer path as the mouse. Returns the cursor, and whether
    /// any sample drove the gizmo. Each press goes to the machine that owns it
    /// ([`Machine`]): while the gizmo tool is on, the gizmo takes the tip's presses on the 3D
    /// view (`gizmo::Tool::sample`), reading the session through `reads`. Otherwise, while the
    /// tip touches the 3D view (and neither barrel button is held: both navigate, unless button
    /// 2 stands for a modifier key, [`Studio::bound`]), the samples
    /// sculpt: the engine takes them with [`Studio::take_strokes`]. A press the engine found on
    /// empty canvas navigates instead, until the pen lifts, or with Ctrl alone held works the
    /// mask ([`Studio::background_answer`]).
    pub fn pen(&mut self, samples: &[PenSample], reads: &PenReads) -> PenTaken {
        let ppp = self.ppp();
        let mut gizmo = false;
        for sample in samples {
            let sample = &self.bound(sample);
            self.pick(sample, reads);
            let owner = self.tip_owner(sample);
            if owner == Some(Machine::Gizmo) && self.gizmo(sample, reads) {
                gizmo = true;
            } else {
                self.sculpt(sample, owner, reads);
            }
            self.tip_lifted(sample);
            let routed = self.input.router.route(sample, ppp, self.input.held);
            self.input.pending.extend(routed);
            self.shell.cursor.pen = sample.in_proximity.then_some(*sample);
        }
        if !samples.is_empty() {
            self.shell.cursor.dabs = self.dabs(reads);
            self.shell.cursor.surface = self.surface(reads);
            let ray = self.pen_ray().filter(|_| !self.shell.cursor.stroking);
            picker::under(&self.ctx, ray, reads, picker::stamp(&self.shell));
        }
        self.dirty |= !samples.is_empty();
        PenTaken {
            cursor: self.shell.cursor.pen,
            gizmo,
        }
    }

    /// `s` as the window reads it: barrel button 2 set to stand for a modifier key
    /// (`ui.navigation` `barrel_two`) is that key while it is down, held as the keyboard's are
    /// (`Sticky`), and no button; the eraser end stands for the keys `ui.navigation`'s
    /// `eraser` names from its first touch until it leaves range (the tablet names the end only
    /// when it touches), so it strokes as the tip with them held. The pen touching or a button
    /// going down makes the keys held down a stroke or a click, not a tap.
    fn bound(&mut self, s: &PenSample) -> PenSample {
        let nav = self.shell.layout.layout.navigation;
        let key = nav.barrel_two;
        let sticky = &mut self.shell.keys.sticky;
        if s.in_proximity && (s.contact || s.buttons != 0) {
            sticky.spoil();
        }
        sticky.pen(key, s.in_proximity && s.buttons & 2 != 0);
        let eraser = s.in_proximity && s.tool == PenTool::Eraser;
        sticky.eraser(if eraser {
            nav.eraser.keys()
        } else {
            [false; 3]
        });
        let held = sticky.held();
        if held != self.input.held {
            self.input.held = held;
            self.input.pending.push(egui::Event::ModifiersChanged(held));
        }
        let buttons = if key == BarrelTwo::Pan {
            s.buttons
        } else {
            s.buttons & !2
        };
        PenSample { buttons, ..*s }
    }

    /// The modifiers in effect with the keys `keys` held down: with sticky modifier keys
    /// (`ui.settings`), a key a tap turned on stays on, and barrel button 2 may stand for one
    /// (`Sticky`). A key or mouse button going down among `inputs` makes the keys held down a
    /// shortcut or a click, not a tap.
    pub(super) fn in_effect(&mut self, inputs: &[Input], keys: egui::Modifiers) -> egui::Modifiers {
        let now = self.now_ns as f64 / 1e9;
        let on = self.shell.layout.layout.settings.sticky_modifiers;
        let other = inputs.iter().any(|i| match i {
            Input::Key {
                key, down, repeat, ..
            } => *down && !*repeat && glue::key(key).is_some(),
            Input::MouseButton { down, .. } => *down,
            _ => false,
        });
        self.shell.keys.sticky.keys(keys, now, on, other)
    }

    /// Where the dab under the pen lands and where the brush's symmetry repeats it, the dab
    /// first, for the cursor's mirror rings: one ray cast against the active part, in the
    /// session (`PenReads::dabs`). Empty while symmetry is off, the mirror rings are turned off
    /// (`ui.settings` `mirror_rings`), the gizmo tool is on, or the pen is away from the 3D
    /// view.
    fn dabs(&self, reads: &PenReads) -> Vec<Vec3> {
        let symmetry = &self.shell.state.brush.symmetry;
        let (m, p) = (symmetry.mirror, symmetry.parallel);
        let shown = self.shell.layout.layout.settings.mirror_rings && !self.shell.gizmo.on;
        let planes = m.x || m.y || m.z || p.x || p.y || p.z;
        let on = (planes || symmetry.radial.is_some()) && shown;
        match self.pen_ray().filter(|_| on) {
            Some((o, d)) => (reads.dabs)(o, d, symmetry),
            None => Vec::new(),
        }
    }

    /// The surface under the pen, for the brush's rings to lie on (`Cursor::surface`): a scene
    /// point and its unit normal (`PenReads::under`), unless the rings are set to stay circles
    /// on the screen (`ui.settings` `ring_on_surface`).
    fn surface(&self, reads: &PenReads) -> Option<(Vec3, Vec3)> {
        let on = self.shell.layout.layout.settings.ring_on_surface;
        let (o, d) = self.pen_ray().filter(|_| on)?;
        let s = (reads.under)(o, d)?;
        Some((s.point, s.normal.normalized()))
    }

    /// The scene ray under the pen, from the eye, while the pen is over the 3D view.
    fn pen_ray(&self) -> Option<(Vec3, Vec3)> {
        let pen = (self.shell.cursor.pen).filter(|p| self.shell.over_view(p))?;
        let (ppp, rect) = (self.ppp(), self.shell.viewport());
        let size = [rect.width() * ppp, rect.height() * ppp];
        let at = [
            pen.x as f32 - rect.min.x * ppp,
            pen.y as f32 - rect.min.y * ppp,
        ];
        Some(self.shell.main.camera.ray(at[0], at[1], size))
    }

    /// What one sample does to the gizmo: true when the gizmo took it, so it does not sculpt.
    /// The commands it asks for wait in the shell; a press on empty canvas navigates.
    fn gizmo(&mut self, s: &PenSample, reads: &PenReads) -> bool {
        let ppp = self.ppp();
        let over_view = self.shell.over_view(s);
        let shell = &mut self.shell;
        let rect = shell.viewport();
        if !rect.is_positive() {
            return false;
        }
        let prefs = shell.layout.layout.gizmo;
        let seen = Seen {
            camera: &shell.main.camera,
            corner: [rect.min.x * ppp, rect.min.y * ppp],
            size: [rect.width() * ppp, rect.height() * ppp],
            style: prefs.style(ppp),
            brush: &shell.state.brush,
            prefs,
            alt: self.input.held.alt,
            shift: self.input.held.shift,
            ctrl: self.input.held.ctrl,
        };
        let took = (shell.gizmo).sample(s, over_view, &seen, reads, &shell.parts);
        shell.canvas |= took.canvas;
        if let Some(problem) = &took.problem {
            shell.status.says(problem);
        }
        for step in took.steps {
            shell.queue(step);
        }
        took.gizmo
    }

    /// What one sample does to a stroke: touching the view starts one, moving while touching
    /// feeds it (with Alt as it is at that sample, and Shift held after the press holding it to
    /// a straight line, [`Line`]), lifting or leaving range ends it. Only a press the stroke owns (`owner`, with
    /// Ctrl alone a Mask press) starts one. A touch that starts off the view (on a panel, or
    /// the Zoom and Pan buttons) never sculpts, even when it slides onto the view. A Ctrl press
    /// on empty canvas sends nothing more once the engine has said so, and its gesture ends
    /// when the pen lifts.
    fn sculpt(&mut self, s: &PenSample, owner: Option<Machine>, reads: &PenReads) {
        let barrel = s.buttons & 1 != 0 || s.buttons & 2 != 0;
        let touching = s.in_proximity && s.contact && !barrel;
        if !touching {
            if !barrel && owner == Some(Machine::Stroke) && self.canvas_click(s) {
                let on = self.shell.state.isolated;
                let args = if on {
                    json!({})
                } else {
                    json!({ "parts": ["@active"] })
                };
                self.shell.queue(with_args("part.isolate", args));
            }
            self.shell.canvas = false;
            self.input.pressed_off = false;
        } else if self.shell.canvas || self.input.pressed_off {
            self.shell.cursor.string = None;
            return;
        }
        let mask = owner == Some(Machine::Mask);
        let owns = mask || owner == Some(Machine::Stroke);
        match (self.shell.cursor.stroking, touching) {
            (false, true) if !owns => {}
            (false, true) => match self.pen_down(s, mask) {
                Some(down) => {
                    let own = down.brush.is_none();
                    let string = own.then(|| self.rope()).flatten().map(|_| [s.x, s.y]);
                    self.shell.cursor.string = string;
                    self.input.alt = down.invert;
                    self.input.line = self.line(s);
                    self.show_path();
                    let stroke = self.shell.preset.as_ref().map(|p| &p.stroke);
                    self.input.space = stroke.is_some_and(StrokeSpec::moves).then_some(false);
                    let (select, rect) = outlined(stroke);
                    self.shell.cursor.outline = if select { vec![[s.x, s.y]] } else { Vec::new() };
                    self.shell.cursor.rect = rect;
                    self.input.strokes.push(PenStroke::Down(down));
                    self.input.strokes.push(PenStroke::Sample(*s));
                    self.shell.cursor.stroking = true;
                    self.input.background = mask.then(|| Background {
                        at: [s.x, s.y],
                        ..Background::default()
                    });
                }
                None => self.input.pressed_off = true,
            },
            (true, true) => {
                let slop = self.click_slop();
                if let Some(b) = &mut self.input.background {
                    let [dx, dy] = [s.x - b.at[0], s.y - b.at[1]];
                    b.dragged |= dx * dx + dy * dy > slop * slop;
                    if b.canvas == Some(true) {
                        return;
                    }
                }
                self.alt_now();
                let s = self.hold_line(s, slop);
                self.input.strokes.push(PenStroke::Sample(s));
                if let Some(path) = self.input.line.as_mut().and_then(|l| l.path.as_mut()) {
                    path.step([s.x, s.y], self.input.space == Some(true));
                    self.show_path();
                }
                self.pull(&s);
                if !self.shell.cursor.outline.is_empty() {
                    self.shell.cursor.outline.push([s.x, s.y]);
                }
            }
            (true, false) => self.pen_up(reads),
            (false, false) => {}
        }
    }

    /// The stroke in progress ends: the pen lifted or left range.
    fn pen_up(&mut self, reads: &PenReads) {
        self.input.strokes.push(PenStroke::Up);
        self.shell.cursor.stroking = false;
        self.input.line = None;
        self.input.space = None;
        self.shell.cursor.line = false;
        self.shell.cursor.string = None;
        self.shell.cursor.outline.clear();
        self.show_path();
        self.lifted(reads);
    }

    /// Whether the tip lifting at `s` ends a click on empty canvas that toggles Isolate
    /// (decision 0028, section 10): its setting is on, there is an active part, the engine
    /// found the press on empty canvas, and the pen lifted within egui's click distance and
    /// time of it with no navigation drag begun.
    fn canvas_click(&self, s: &PenSample) -> bool {
        let shell = &self.shell;
        let on = shell.layout.layout.settings.click_isolate && shell.state.active.is_some();
        if !(on && shell.canvas) || shell.nav.dragging().is_some() {
            return false;
        }
        let (ppp, slop) = (f64::from(self.ppp()), self.click_slop());
        self.ctx.input(|i| {
            let near = (i.pointer.press_origin()).is_some_and(|o| {
                let [dx, dy] = [s.x - f64::from(o.x) * ppp, s.y - f64::from(o.y) * ppp];
                dx * dx + dy * dy <= slop * slop
            });
            near && i.pointer.could_any_button_be_click()
        })
    }

    /// A stroke starting at `s`, if `s` is over the 3D view (as last drawn) and nothing else:
    /// the view's pixels, what the held modifiers ask for (`Held`), and whether the engine
    /// answers a press on empty canvas (to navigate, or for a Ctrl gesture, `mask`, which asks
    /// always).
    fn pen_down(&self, s: &PenSample, mask: bool) -> Option<PenDown> {
        let ppp = self.ppp();
        if !self.shell.over_view(s) {
            return None;
        }
        let size = RenderSize {
            width: self.size[0].max(1),
            height: self.size[1].max(1),
            samples: 1,
        };
        let viewport = pixels(self.shell.viewport(), ppp, size);
        let held = self.held();
        let navigates = self.shell.layout.layout.navigation.classic && !self.shell.main.locked;
        let classic = navigates || mask;
        Some(PenDown {
            classic,
            viewport,
            brush: held.as_ref().and_then(|h| h.brush.clone()),
            invert: self.inverts(),
        })
    }

    /// The pen's brush's smoothing string (decision 0036 section 4): its length in window
    /// pixels, the brush's `stroke.smoothing.radius` times its size on screen (a radius in
    /// pixels at every depth), and how much of each move the dab keeps back. `None` while
    /// smoothing is off, or the size is set as a length, which has no size on screen.
    fn rope(&self) -> Option<(f64, f64)> {
        let k = self.shell.preset.as_ref()?.stroke.smoothing();
        let px = self.shell.state.brush.size_px?;
        (!k.is_off()).then(|| (f64::from(k.radius * px), f64::from(k.factor)))
    }

    /// The dab the string pulls follows the pen to `s` as the engine's smoother moves it
    /// (gs-core `stroke/path.rs`): it stays while the pen is within the string, and else moves
    /// toward the pen until the string is taut, less the part of the move it keeps back.
    fn pull(&mut self, s: &PenSample) {
        let (Some(at), Some((rope, factor))) = (self.shell.cursor.string, self.rope()) else {
            return;
        };
        let [dx, dy] = [s.x - at[0], s.y - at[1]];
        let len = (dx * dx + dy * dy).sqrt();
        if len > rope {
            let k = (len - rope) / len * (1.0 - factor);
            self.shell.cursor.string = Some([at[0] + dx * k, at[1] + dy * k]);
        }
    }

    /// The straight line a stroke pressed at `s` may be held to, whichever brush or slot it
    /// strokes with: a dab stroke (`stroke.freehand`) or a curve (`stroke.curve-line`). Shift
    /// held at the press (a slot's key) holds it only once let go and pressed again. A stroke
    /// that runs through points also keeps its path ([`Path`]), with a held slot's brush too
    /// (decision 0017 as amended 2026-10-12, item 22): the preset is the brush in effect.
    fn line(&self, s: &PenSample) -> Option<Line> {
        let preset = self.shell.preset.as_ref()?;
        let stroke = preset.stroke;
        let curve = match stroke {
            StrokeSpec::Dab { .. } => false,
            StrokeSpec::Curve { .. }
            | StrokeSpec::Gesture {
                shape: Shape::Curve,
                ..
            } => true,
            _ => return None,
        };
        let path = stroke.takes_points().then(|| Path::new([s.x, s.y], preset));
        Some(Line {
            from: [s.x, s.y],
            curve,
            first: None,
            shift: None,
            armed: !self.input.held.shift,
            path,
        })
    }

    /// Alt pressed or let go during a stroke, sent as it happens, even with no sample between:
    /// the samples from now on work that way (decision 0036 section 5). For a stroke that runs
    /// through points, Alt going down also taps a point in at the pen, or a corner (decision
    /// 0055, [`Path::alt`]), and a gesture's shadow side flips with it.
    pub(super) fn alt_now(&mut self) {
        let canvas = self
            .input
            .background
            .as_ref()
            .is_some_and(|b| b.canvas == Some(true));
        if !self.shell.cursor.stroking || self.shell.canvas || canvas {
            return;
        }
        let alt = self.inverts();
        if alt == self.input.alt {
            return;
        }
        self.input.alt = alt;
        self.input.strokes.push(PenStroke::Alt(alt));
        let now = self.now_ns as f64 / 1e9;
        let path = self.input.line.as_mut().and_then(|l| l.path.as_mut());
        if let Some(tap) = path.and_then(|p| p.alt(alt, now)) {
            self.input.strokes.push(tap);
        }
        self.show_path();
    }

    /// `s` held to the stroke's straight line while Shift holds it ([`Line`]), with the cursor
    /// saying so.
    fn hold_line(&mut self, s: &PenSample, slop: f64) -> PenSample {
        let keys = (self.input.held.shift, self.input.space == Some(true));
        let s = (self.input.line.as_mut()).map_or(*s, |l| l.hold(s, keys, slop));
        self.shell.cursor.line = (self.input.line.as_ref()).is_some_and(|l| l.shift.is_some());
        s
    }

    /// The held keys make a stroke work the other way (Alt) from its brush's stored mode, which
    /// the engine applies.
    fn inverts(&self) -> bool {
        self.held().is_some_and(|h| h.invert)
    }

    /// What the held keys do to a stroke: the slot's brush by name (the engine holds the same
    /// slot, `brush.slot`, but the stroke says it), and Alt.
    fn held(&self) -> Option<Held> {
        Held::of(self.input.held, None, None, &self.shell.state.brush.slots)
    }

    /// What the pen did to the 3D view since the last time, in order. A Ctrl press among them
    /// is now asked about: the report after this call says whether it fell on empty canvas.
    pub fn take_strokes(&mut self) -> Vec<PenStroke> {
        let strokes = std::mem::take(&mut self.input.strokes);
        if let Some(b) = &mut self.input.background
            && !strokes.is_empty()
        {
            b.asked = true;
        }
        strokes
    }
}

/// Shift held after a stroke's press holds it to a straight line, in window pixels: a curve
/// (`stroke.curve-line`) runs from its last point (the press while there is none) toward the
/// pen, turned to the nearest 45 degrees (decision 0055, section 3); any other stroke
/// (`stroke.freehand`) runs on from where the pen was when Shift went down, the way it first
/// moved. While Shift is held each sample is the nearest point on the line, unless Space is
/// moving the path.
pub(super) struct Line {
    /// Where the stroke pressed.
    from: [f64; 2],
    /// The stroke is a curve: its line turns in 45 degree steps about its last point.
    curve: bool,
    /// The way the pen first moved farther than a click from the press, a unit vector.
    first: Option<[f64; 2]>,
    /// Where the pen was when Shift went down; `None` while it is up.
    shift: Option<[f64; 2]>,
    /// Shift has been up since the press: Shift held from the press calls a slot, not a line.
    armed: bool,
    /// The path of a stroke that runs through points, as the window draws it.
    pub(super) path: Option<Path>,
}

impl Line {
    /// `s` held to the line while `shift` is held and Space is not `moving` the path; `slop`
    /// is a click's reach in pixels.
    fn hold(&mut self, s: &PenSample, (shift, moving): (bool, bool), slop: f64) -> PenSample {
        self.armed |= !shift;
        let shift = shift && self.armed;
        let at = [s.x, s.y];
        let [dx, dy] = [at[0] - self.from[0], at[1] - self.from[1]];
        let far = (dx * dx + dy * dy).sqrt();
        if self.first.is_none() && far > slop {
            self.first = Some([dx / far, dy / far]);
        }
        self.shift = shift.then(|| self.shift.unwrap_or(at));
        let line = if self.curve {
            let last = self.path.as_ref().and_then(|p| p.points.last());
            let from = last.map_or(self.from, |q| q.0);
            let [dx, dy] = [at[0] - from[0], at[1] - from[1]];
            let far = (dx * dx + dy * dy).sqrt();
            let turn = (libm::atan2(dy, dx) / FRAC_PI_4).round() * FRAC_PI_4;
            (far > slop).then(|| (from, [libm::cos(turn), libm::sin(turn)]))
        } else {
            self.shift.zip(self.first)
        };
        let Some((start, way)) = line.filter(|_| shift && !moving) else {
            return *s;
        };
        let along = (at[0] - start[0]) * way[0] + (at[1] - start[1]) * way[1];
        PenSample {
            x: start[0] + way[0] * along,
            y: start[1] + way[1] * along,
            ..*s
        }
    }
}

/// What the cursor draws while a `stroke` is dragged: whether a select outline shows
/// (`Cursor::outline`), and for a rectangle its Square and Center (`Cursor::rect`).
fn outlined(stroke: Option<&StrokeSpec>) -> (bool, Option<(bool, bool)>) {
    match stroke {
        Some(&StrokeSpec::Gesture {
            shape,
            square,
            center,
            ..
        }) => (
            shape != Shape::Curve,
            (shape == Shape::Rect).then_some((square, center)),
        ),
        _ => (false, None),
    }
}
