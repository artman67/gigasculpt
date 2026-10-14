//! What drawing needs: the renderer and what it keeps between frames, and the one way the
//! window draws a 3D view into a frame.

use std::collections::{BTreeMap, BTreeSet};

use gs_engine::{CallReport, LiveDraw, LivePeel, PartReport, Stand, Tint};
use gs_render::{
    FrameTarget, GpuMesh, Operand, PEEL_OPERANDS, Preview, RenderSize, Renderer, Scene, UiLayer,
    UiPainter,
};

use crate::draw::{Look, drawn};
use crate::lighting::Lighting;
use crate::matcaps::MatCaps;
use crate::theme::Colors;

/// How each part stands in for a Live Boolean result, by part id: not drawn (`None`), or drawn
/// as an outline in a color (decision 0057, section 9).
type Stands = BTreeMap<u64, Option<[u8; 3]>>;

/// How each part joins Live Boolean's peel, by part id (decision 0057, section 9, as amended
/// for LB8).
type Peels = BTreeMap<u64, Operand>;

/// What drawing needs: the renderer and what it keeps between frames.
pub struct Render {
    pub renderer: Renderer,
    pub(super) painter: UiPainter,
    pub(super) target: Option<FrameTarget>,
    /// The looks; the selected one shades the 3D view and the offscreen looks.
    pub(crate) matcaps: MatCaps,
    /// The environments Lit mode lights with, on this device.
    pub(crate) lighting: Lighting,
    /// Each drawn part: its name, the id of the part it was uploaded from, and its GPU mesh,
    /// which draws straight from the part's mesh store.
    pub(super) meshes: Vec<(String, u64, GpuMesh)>,
    /// Live Boolean's results, walls and marks (`CallReport::live`), by id, drawn with the parts.
    live: Vec<(u64, GpuMesh)>,
    /// Each Reference View the window shows, by name (decisions 0020, 0024).
    pub(super) references: BTreeMap<String, Pictured>,
    /// Bytes the last update sent to the GPU.
    uploaded: u64,
    /// GPU copies the last update looked at to find each part's own (a work counter,
    /// ARCHITECTURE 10.1).
    looked: u64,
    /// Jittered samples a still view accumulates (1 for byte-exact tests, 16 on a real GPU;
    /// decision 0019).
    pub(super) samples: u32,
}

impl Render {
    /// Drawing on `renderer`, with MatCap Gray (ARCHITECTURE 6) baked for it.
    pub fn new(renderer: Renderer, samples: u32) -> Result<Self, String> {
        let mut matcaps = MatCaps::default();
        matcaps.gpu(&renderer)?;
        Ok(Self {
            painter: renderer.ui_painter(),
            renderer,
            target: None,
            matcaps,
            lighting: Lighting::default(),
            meshes: Vec::new(),
            live: Vec::new(),
            references: BTreeMap::new(),
            uploaded: 0,
            looked: 0,
            samples,
        })
    }

    /// The last frame drawn.
    pub fn target(&self) -> Option<&FrameTarget> {
        self.target.as_ref()
    }

    /// The last frame of Reference View `view`'s picture: the view alone, no interface over
    /// it.
    pub fn reference_frame(&self, view: &str) -> Option<&FrameTarget> {
        self.references.get(view)?.target.as_ref()
    }

    /// Brings the GPU copies of the scene's parts and of each Reference View's own parts up to
    /// date (see [`sync`]). The scene's active part is marked, for the views that outline it;
    /// a reference's own parts never are, since no stroke reaches them (decision 0033, section
    /// 5). A reference that closed keeps its frame until the next draw frees its texture. Live
    /// Boolean's meshes are drawn with the scene's parts, in `colors`' tints, and its operands
    /// stand in for them.
    pub(super) fn sync(&mut self, report: &CallReport, colors: &Colors) {
        let rgb = |tint| {
            let [r, g, b, _] = colors.live(tint).to_array();
            [r, g, b]
        };
        let stand = |s| match s {
            Stand::Ghost(tint) => Some(rgb(tint)),
            Stand::Hidden => None,
        };
        let mut stands: Stands = (report.live.stands.iter())
            .map(|&(id, s)| (id, stand(s)))
            .collect();
        let (peels, replaced) = peeled(&self.renderer, &report.live, rgb, &mut stands);
        let main = (&report.parts[..], true, &stands, &peels);
        let [sent, looked] = sync(&self.renderer, &mut self.meshes, main);
        (self.uploaded, self.looked) = (sent, looked);
        self.uploaded += self.sync_live(&report.live, rgb, &replaced);
        // The open deformer's preview on the graphics card (decision 0041, amended for DF9a).
        for (_, id, gpu) in &mut self.meshes {
            let p = report.preview.filter(|p| p.part == *id);
            let preview = p.map(|p| Preview {
                ops: &p.ops,
                weights: &p.weights,
                weighed: p.weighed,
            });
            self.uploaded += self.renderer.preview(gpu, preview);
        }
        for r in &report.references {
            let pictured = self.references.entry(r.setup.view.clone()).or_default();
            let own = r.parts.as_deref().unwrap_or_default();
            let none = (&Stands::new(), &Peels::new());
            let own = (own, false, none.0, none.1);
            let [sent, looked] = sync(&self.renderer, &mut pictured.meshes, own);
            self.uploaded += sent;
            self.looked += looked;
        }
    }

    /// Brings the GPU copies of `live`'s meshes up to date: each uploaded once, by id, and
    /// placed, marked, painted and tinted (`rgb` gives a tint's color) as reported, kept but not
    /// drawn while a peel replaces it; one no longer reported is dropped. Returns the bytes
    /// sent.
    fn sync_live(
        &mut self,
        live: &LiveDraw,
        rgb: impl Fn(Tint) -> [u8; 3],
        replaced: &BTreeSet<u64>,
    ) -> u64 {
        let mut old: BTreeMap<u64, GpuMesh> = std::mem::take(&mut self.live).into_iter().collect();
        let mut sent = 0;
        for m in live.meshes.iter().filter(|m| drawn(m.store)) {
            let gpu = old.remove(&m.id).unwrap_or_else(|| {
                let gpu = self.renderer.upload_store(m.store);
                sent += gpu.bytes();
                gpu
            });
            let shown = !replaced.contains(&m.id);
            let gpu = (gpu.placed(m.transform, shown).marked(m.marked))
                .painted(m.painted)
                .tinted(m.tint.map(&rgb));
            self.live.push((m.id, gpu));
        }
        sent
    }

    /// Bytes the last update after a call sent to the GPU: whole parts when they are new, and
    /// only the changed nodes of a part edited in place.
    pub fn uploaded(&self) -> u64 {
        self.uploaded
    }

    /// GPU copies the last update after a call looked at to find each part's own: a work
    /// counter that keeps the update's work growing with the part count, not its square.
    pub fn looked(&self) -> u64 {
        self.looked
    }

    /// The window's one way of drawing a 3D view (decision 0024): the GPU meshes `slot` shows,
    /// through `look`, into `slot`'s frame (made again whenever `size` changes) at `viewport`,
    /// with `ui` over them. A still view adds one jittered sample per call until its frame
    /// settles (decision 0019).
    pub(super) fn draw_view(
        &mut self,
        slot: Slot,
        (size, viewport): (RenderSize, [u32; 4]),
        look: &Look,
        ui: &UiLayer,
    ) -> Result<(), String> {
        let text = |e: gs_render::RenderError| e.to_string();
        let Self {
            renderer,
            painter,
            matcaps,
            lighting,
            target,
            meshes,
            live: results,
            references,
            ..
        } = self;
        let (target, shown, results) = match slot {
            Slot::Main => (target, &*meshes, &results[..]),
            Slot::Reference { view, live } => {
                let pictured = references.entry(view.to_string()).or_default();
                let none: &[_] = &[];
                let (shown, results) = if live {
                    (&*meshes, &results[..])
                } else {
                    (&pictured.meshes, none)
                };
                (&mut pictured.target, shown, results)
            }
        };
        if target.as_ref().is_none_or(|t| t.size() != size) {
            *target = Some(renderer.frame_target(size).map_err(text)?);
        }
        let Some(target) = target.as_ref() else {
            return Ok(());
        };
        let results = results.iter().map(|(_, m)| m);
        let meshes: Vec<&GpuMesh> = shown.iter().map(|(_, _, m)| m).chain(results).collect();
        let scene = Scene {
            meshes: &meshes,
            view: look.view(renderer, (matcaps, lighting))?,
            viewport,
        };
        renderer
            .draw_frame(target, &scene, painter, ui)
            .map_err(text)
    }
}

/// Which of the window's frames a view draws into, and which GPU meshes it shows.
#[derive(Clone, Copy)]
pub(super) enum Slot<'a> {
    /// The window's frame: the scene's parts, under the interface.
    Main,
    /// Reference View `view`'s picture: the scene's parts while it shows them live, else its
    /// own.
    Reference { view: &'a str, live: bool },
}

/// A Reference View the window shows: its own parts' GPU meshes (empty while it shows the
/// scene's parts, live) and the frame its picture is drawn into, the interface's texture
/// [`picture`] of it.
#[derive(Default)]
pub(super) struct Pictured {
    meshes: Vec<(String, u64, GpuMesh)>,
    pub(super) target: Option<FrameTarget>,
}

/// Brings the GPU copies in `meshes` up to date with `parts`, in their order: a new part (or
/// one put in place of another under its name) is uploaded whole, a part edited in place has
/// only its changed nodes written again, and a part that is gone is dropped. Each is placed at
/// its part's transform; a hidden part's copy is kept but not drawn, so showing it again sends
/// nothing (decision 0029), and so is one Dynamic isolate hides (decision 0028, section 10).
/// With `mark_active`, the active part is marked (decision 0033). A part in `stands` stands in
/// for a Live Boolean result: kept but not drawn, or drawn as an outline; a part in `peels` is
/// drawn through the peel. Each part finds its copy in a map by part id, so a scene of
/// thousands of parts costs one look per part, not a search of the copies each.
/// Returns the bytes sent and the copies looked at.
fn sync(
    renderer: &Renderer,
    meshes: &mut Vec<(String, u64, GpuMesh)>,
    (parts, mark_active, stands, peels): (&[PartReport], bool, &Stands, &Peels),
) -> [u64; 2] {
    let mut old: BTreeMap<u64, GpuMesh> = std::mem::take(meshes)
        .into_iter()
        .map(|(_, id, gpu)| (id, gpu))
        .collect();
    let (mut sent, mut looked) = (0, 0);
    for p in parts.iter().filter(|p| drawn(p.store)) {
        looked += 1;
        let gpu = if let Some(mut gpu) = old.remove(&p.id) {
            sent += renderer.update_store(&mut gpu, p.store, &p.dirty).bytes;
            gpu
        } else {
            let gpu = renderer.upload_store(p.store);
            sent += gpu.bytes();
            gpu
        };
        let stand = stands.get(&p.id);
        let gpu = gpu
            .placed(p.transform, p.drawn && stand != Some(&None))
            .marked(mark_active && p.active)
            .painted(p.painted)
            .ghosted(stand.copied().flatten())
            .peeled(peels.get(&p.id).copied());
        meshes.push((p.name.to_string(), p.id, gpu));
    }
    [sent, looked]
}

/// Live Boolean's source for each group `live` offers the peel (decision 0057, section 9, as
/// amended for LB8): the peel where `renderer` peels, the group has at most [`PEEL_OPERANDS`]
/// parts and its base is not messy, in place of the stand-ins in `stands`: its base and Adds
/// peeled, its Subtracts and Intersects peeled and outlined in their tint (`rgb`), a messy part
/// drawn plain. Returns how each part joins the peel, and the meshes the peels replace.
fn peeled(
    renderer: &Renderer,
    live: &LiveDraw,
    rgb: impl Fn(Tint) -> [u8; 3],
    stands: &mut Stands,
) -> (Peels, BTreeSet<u64>) {
    let (mut peels, mut replaced) = (Peels::new(), BTreeSet::new());
    let fits = |p: &&LivePeel| {
        let base = p.operands.first();
        p.operands.len() <= PEEL_OPERANDS && base.is_some_and(|b| !b.messy)
    };
    for peel in (live.peels.iter())
        .filter(|_| renderer.peels())
        .filter(fits)
    {
        replaced.extend(peel.replaces.iter().copied());
        for (k, o) in peel.operands.iter().enumerate() {
            stands.remove(&o.id);
            let operand = match o.tint {
                _ if o.messy => continue,
                _ if k == 0 => Operand::Base,
                Tint::Difference => Operand::Difference(rgb(o.tint)),
                Tint::Intersect => Operand::Intersect(rgb(o.tint)),
                _ => Operand::Union,
            };
            if let Operand::Difference(c) | Operand::Intersect(c) = operand {
                stands.insert(o.id, Some(c));
            }
            peels.insert(o.id, operand);
        }
    }
    (peels, replaced)
}

/// Whether the 3D view draws part `p`: it has a GPU copy (see [`sync`]) placed to be drawn,
/// so `ui.describe` reports the same parts with a renderer or with none.
pub(super) fn in_view(p: &PartReport) -> bool {
    drawn(p.store) && p.drawn
}

/// A rectangle in points as x, y, width, height in whole pixels inside `size`.
pub(super) fn pixels(rect: egui::Rect, ppp: f32, size: RenderSize) -> [u32; 4] {
    if !rect.is_positive() {
        return [0, 0, 0, 0];
    }
    let r = rect * ppp;
    let clamp = |v: f32, max: u32| (v.max(0.0).round() as u32).min(max);
    let (x0, y0) = (clamp(r.min.x, size.width), clamp(r.min.y, size.height));
    let (x1, y1) = (clamp(r.max.x, size.width), clamp(r.max.y, size.height));
    [x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0)]
}
