//! Session views (decision 0017): cameras belong to whoever looks. `main` is the window's view;
//! each agent gets `agent:<its client name>`, made from `main`'s camera at its first call (and
//! logged, see [`Views::make`]); `ref:<n>` is a Reference View, a secondary view with a scene of
//! its own and a link to `main` (decision 0020, `secondary`), made by `reference.open`; `widget`
//! is kept for a corner figure with a scene of its own. A view is never part of the document:
//! moving it changes no part, revision or undo step. Each keeps its own camera history of
//! [`HISTORY`](history::HISTORY) steps.
//!
//! A view's camera starts out framed: the front view fitting every part at the view's aspect,
//! worked out whenever it is read. At the start of the first call that finds parts in the scene
//! (and, for `main` in a window, the window's view size known) the engine keeps that camera, so
//! later edits do not reframe the view; when the scene is emptied, views go back to framed.
//! Only `main` in a window depends on something the log does not hold (the window's size), so
//! only its kept framing is logged. A gesture (calls sharing an id) is one history step, and the engine logs it once, as
//! the camera it ended on.
//!
//! This file holds the views map and what every part of it reads; `settings` the navigation
//! settings, `gesture` camera changes and gestures, `history` the camera history,
//! `references` the Reference Views, and `info` what `view.get` reports.

mod gesture;
mod history;
mod info;
mod references;
mod settings;

use std::collections::BTreeMap;

use gs_core::camera::{Camera, Preset};
use gs_protocol::{CommandError, Role};

pub(crate) use self::gesture::Ended;
use self::gesture::Gesture;
pub use self::info::ViewInfo;
pub use self::settings::{MainSettings, NavSettings, OrbitArg, PivotRule};
use super::frame::fitted;
use super::secondary::Secondary;
use crate::ctx::Origin;
use crate::document::Document;
use crate::errors;
use crate::{Ctx, Unavailable};

/// The image size a view is seen at until it is told otherwise: a snapshot's.
pub const DEFAULT_VIEWPORT: [u32; 2] = [1024, 1024];
/// Agent views a session keeps: making one more drops the one used longest ago (review 0003
/// s3).
pub const AGENT_VIEWS: usize = 64;

/// One view.
#[derive(Clone, Debug)]
pub(crate) struct View {
    /// `None` until a command sets it: then it is framed (see the module docs).
    pub camera: Option<Camera>,
    pub viewport: [u32; 2],
    pub locked: bool,
    pub orbit: OrbitArg,
    pub pivot: PivotRule,
    pub nav: NavSettings,
    /// Its edges are drawn over the surface (`view.look`). A Reference View keeps this in
    /// its look instead, which its saved setup holds.
    pub wireframe: bool,
    pub last_edit: Option<[f32; 3]>,
    /// The camera the last framing of every part gave, while that was the last framing: a
    /// press of Frame on it frames the active part instead (canvas.frame).
    pub framed_all: Option<Camera>,
    /// Earlier cameras, oldest first.
    undo: Vec<Option<Camera>>,
    redo: Vec<Option<Camera>>,
    gesture: Option<Gesture>,
    /// When a call last named it, on the views' own clock.
    used: u64,
    /// A Reference View's scene and link to `main`; `camera` then holds what is its own.
    pub secondary: Option<Secondary>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            camera: None,
            viewport: DEFAULT_VIEWPORT,
            locked: false,
            orbit: OrbitArg::Y,
            pivot: PivotRule::Target,
            nav: NavSettings::default(),
            wireframe: false,
            last_edit: None,
            framed_all: None,
            undo: Vec::new(),
            redo: Vec::new(),
            gesture: None,
            used: 0,
            secondary: None,
        }
    }
}

/// Every view of a session.
#[derive(Clone, Debug)]
pub struct Views {
    list: BTreeMap<String, View>,
    ended: Vec<Ended>,
    /// Counts the times views were named, for which was used longest ago.
    clock: u64,
    /// The size the window last showed a Reference View at: where a new one will show.
    reference_seen: Option<[u32; 2]>,
    /// The active Reference View (decision 0024): the one opened last, or moved to by
    /// `reference.next`; while it is closed, the lowest-numbered open one is.
    active: Option<String>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            list: BTreeMap::from([(MAIN.to_string(), View::default())]),
            ended: Vec::new(),
            clock: 0,
            reference_seen: None,
            active: None,
        }
    }
}

/// The window's view.
pub const MAIN: &str = "main";

/// The view a client's view commands act on when it names none: `main` for the human, its own
/// `agent:<name>` for an agent or an observer.
pub(crate) fn own_view(origin: &Origin) -> String {
    match origin.role {
        Role::Human => MAIN.into(),
        Role::Agent | Role::Observer => format!("agent:{}", origin.client),
    }
}

/// What a camera command is part of: nothing (one step on its own), or a gesture; and whether
/// it is a step of the camera history at all.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Part {
    pub gesture: Option<u64>,
    pub end: bool,
    pub remember: bool,
}

/// A camera change about to happen: the camera to change and the point to turn about.
pub(crate) struct Moving {
    pub camera: Camera,
    pub pivot: gs_core::Vec3,
}

impl Views {
    /// `name` is a view that exists or can be made now: `main`, an agent's, or an open
    /// Reference View. `widget` and a reference that is not open refuse with what they wait for;
    /// anything else is not a view.
    pub(crate) fn check_name(&self, name: &str) -> Result<(), CommandError> {
        if name == "widget" {
            let reason = Unavailable::new(
                "widget is kept for a corner figure with a scene of its own, which is not built \
                 yet",
            )
            .fix("use main, a reference view, or your own view (leave view out)");
            return Err(errors::unavailable(reason));
        }
        if reference_number(name).is_some() {
            if self.secondary(name).is_some() {
                return Ok(());
            }
            let reason = Unavailable::new(format!("{name} is not open"))
                .fix(format!("open it with reference.open view={name}"));
            return Err(errors::unavailable(reason));
        }
        let agent = name.strip_prefix("agent:").is_some_and(|n| !n.is_empty());
        if name == MAIN || agent {
            return Ok(());
        }
        Err(errors::not_found(
            "view",
            name,
            self.list.keys().map(String::as_str),
        ))
    }

    /// The view called `name`; for an agent view not made yet (another client's, or in a dry
    /// run), the view [`Views::make`] would make now.
    fn view(&self, name: &str) -> View {
        self.list.get(name).cloned().unwrap_or_else(|| self.made())
    }

    /// A new agent view: `main`'s camera and settings, its own viewport and history.
    fn made(&self) -> View {
        let main = self.list.get(MAIN).cloned().unwrap_or_default();
        View {
            camera: main.camera,
            orbit: main.orbit,
            pivot: main.pivot,
            nav: main.nav,
            ..View::default()
        }
    }

    /// Makes agent view `name` for a call by `origin`, if it is not made yet (any other name
    /// makes nothing), and marks it used. The one place a view is made: the engine makes every
    /// view a call names before the call runs, its client's own included, so a view is made on
    /// its first read as well as its first change. The camera it copied from `main` waits for
    /// its log line (a `view.set` that is no history step), so a replay makes it the same
    /// whatever `main` holds at that point of the replay, a gesture still open on `main`
    /// included (review 0003 M2). A framed copy needs no line: framing depends on the document
    /// only. Beyond [`AGENT_VIEWS`], the agent view used longest ago is dropped.
    pub(crate) fn make(&mut self, name: &str, origin: &Origin) {
        self.clock += 1;
        if let Some(view) = self.list.get_mut(name) {
            view.used = self.clock;
            return;
        }
        if self.check_name(name).is_err() {
            return;
        }
        let made = View {
            used: self.clock,
            ..self.made()
        };
        if let Some(camera) = made.camera {
            self.ended.push(Ended {
                view: name.to_string(),
                origin: origin.clone(),
                camera,
                viewport: made.viewport,
                remember: false,
            });
        }
        self.list.insert(name.to_string(), made);
        self.drop_unused();
    }

    /// Drops the agent view used longest ago while there are more than [`AGENT_VIEWS`], its
    /// gesture closed first so the gesture keeps its log line. Agents that connect once per
    /// command (the `gs` command line does) keep their view between calls; a stream of
    /// one-off client names cannot grow the session without bound (review 0003 s3).
    fn drop_unused(&mut self) {
        let agents = |list: &BTreeMap<String, View>| {
            (list.iter())
                .filter(|(name, _)| name.starts_with("agent:"))
                .count()
        };
        while agents(&self.list) > AGENT_VIEWS {
            let oldest = (self.list.iter())
                .filter(|(name, _)| name.starts_with("agent:"))
                .min_by_key(|(_, view)| view.used)
                .map(|(name, _)| name.clone());
            let Some(oldest) = oldest else { return };
            self.close(&oldest);
            self.list.remove(&oldest);
        }
    }

    fn view_mut(&mut self, name: &str) -> &mut View {
        self.make(name, &engine());
        self.list.entry(name.to_string()).or_default()
    }

    /// The view's settings and state, for `view.get` and the window.
    pub(crate) fn get(&self, name: &str) -> View {
        self.view(name)
    }

    /// The camera of view `name` as it is now: its own, or framed at its aspect; for a
    /// Reference View, its own with what is linked taken from `main`.
    pub(crate) fn camera_of(&self, name: &str, doc: &Document) -> Result<Camera, CommandError> {
        self.check_name(name)?;
        Ok(self.resolved(name, doc))
    }

    /// The camera of view `name` now (see `camera_of`); `doc` is the session's scene.
    fn resolved(&self, name: &str, doc: &Document) -> Camera {
        let view = self.view(name);
        let Some(secondary) = &view.secondary else {
            return framed(&view, doc);
        };
        let own = framed(&view, secondary.scene.doc(doc));
        secondary.camera(own, &self.resolved(MAIN, doc))
    }

    /// The scene view `name` shows: a Reference View's own, else `doc`, the session's.
    pub(crate) fn scene<'a>(&'a self, name: &str, doc: &'a Document) -> &'a Document {
        match self.secondary(name) {
            Some(s) => s.scene.doc(doc),
            None => doc,
        }
    }

    /// The window's 3D view is `viewport` pixels: `main` is seen at that size.
    pub(crate) fn seen_at(&mut self, name: &str, viewport: [u32; 2]) {
        let reference = reference_number(name).is_some();
        let closed = reference && self.secondary(name).is_none();
        if reference && viewport[0] > 0 && viewport[1] > 0 {
            self.reference_seen = Some(viewport);
        }
        if viewport[0] > 0 && viewport[1] > 0 && !closed {
            self.view_mut(name).viewport = viewport;
        }
    }

    /// May the caller move view `name`'s camera? The human moves any view and an agent any
    /// agent's; `main` and the Reference Views, while a window shows them, only the human,
    /// unless the human allowed agents.
    pub(crate) fn may_change(cx: &Ctx, name: &str) -> Result<(), Unavailable> {
        let windows = name == MAIN || reference_number(name).is_some();
        let open = !windows || cx.origin.role == Role::Human || cx.control.agents_move_view;
        if open || !cx.window.with(|w| w.has_window()) {
            return Ok(());
        }
        Err(Unavailable::new(format!(
            "{name} is the window's view: only the human at the window moves it"
        ))
        .fix(
            "move your own view instead (leave view out); a look with view \"main\" sees exactly \
             what the window shows without moving it. The human can allow agents to move the \
             window's views with session.control agents_move_view",
        ))
    }

    /// Keeps the framing of every framed view `seen` says is seen at a known size, once the
    /// scene has parts; `main`'s waits for its log line when `window` (the window's size is
    /// nowhere in the log). Sends every view back to framed once the scene has none.
    pub(crate) fn keep_framing(
        &mut self,
        doc: &Document,
        window: bool,
        seen: impl Fn(&str) -> bool,
    ) {
        let empty = doc.len() == 0;
        let engine = engine();
        // Reference Views frame their own scenes when they open, and keep their cameras.
        for (name, view) in self.list.iter_mut().filter(|(_, v)| v.secondary.is_none()) {
            if empty {
                view.camera = None;
                continue;
            }
            if view.camera.is_some() || !seen(name) {
                continue;
            }
            let camera = framed(view, doc);
            view.camera = Some(camera);
            if !(window && name == MAIN) {
                continue;
            }
            self.ended.push(Ended {
                view: name.clone(),
                origin: engine.clone(),
                camera,
                viewport: view.viewport,
                remember: false,
            });
        }
    }

    /// Changes view `name`'s settings.
    pub(crate) fn settle(&mut self, name: &str, f: impl FnOnce(&mut View)) {
        f(self.view_mut(name));
    }

    /// A stroke on view `name`'s behalf first touched the surface at `at`.
    pub(crate) fn edited(&mut self, name: &str, at: [f32; 3]) {
        self.view_mut(name).last_edit = Some(at);
    }

    /// The point view `name` turns about by its own rule, for a move that starts at pixel
    /// `at` (for `under-pointer`).
    pub(crate) fn pivot(&self, name: &str, doc: &Document, at: Option<[f32; 2]>) -> [f32; 3] {
        let view = self.view(name);
        let camera = self.resolved(name, doc);
        let scene = self.scene(name, doc);
        let last = || view.last_edit.unwrap_or(camera.target);
        match view.pivot {
            PivotRule::Target => camera.target,
            PivotRule::LastEdit => last(),
            PivotRule::Part => super::frame::center(scene).unwrap_or(camera.target),
            PivotRule::UnderPointer => {
                let size = view.viewport.map(|v| v as f32);
                at.and_then(|[x, y]| super::frame::hit(scene, camera.ray(x, y, size)))
                    .unwrap_or_else(last)
            }
        }
    }

    /// Whether view `name` draws its edges over the surface: a Reference View's look says, any
    /// other view its own switch.
    pub(crate) fn wireframe(&self, name: &str) -> bool {
        match self.secondary(name) {
            Some(s) => s.look.wireframe,
            None => self.list.get(name).is_some_and(|v| v.wireframe),
        }
    }
}

/// The view's own camera, or the front view framing every part of `doc` at the view's aspect.
fn framed(view: &View, doc: &Document) -> Camera {
    let [w, h] = view.viewport;
    let aspect = w.max(1) as f32 / h.max(1) as f32;
    view.camera
        .unwrap_or_else(|| fitted(Preset::Front, doc, aspect))
}

/// Who the engine's own view lines are logged as.
fn engine() -> Origin {
    Origin {
        client_id: 0,
        client: "engine".into(),
        role: Role::Human,
    }
}

/// The number of a Reference View's name: 3 for `ref:3`.
pub(crate) fn reference_number(name: &str) -> Option<u32> {
    name.strip_prefix("ref:")?.parse().ok()
}

/// Why view `name` cannot move: it is locked.
pub(crate) fn locked(name: &str) -> CommandError {
    let reason = Unavailable::new(format!("{name} is locked, so its camera cannot move"))
        .fix(format!("unlock it with view.lock locked=false view={name}"));
    errors::unavailable(reason)
}
