//! The floor grid (catalog item `canvas.floor-grid`; research 14, sections 3.3 and 7.5;
//! decisions 0019 and 0023): up to three planes, each square to an axis, placed from the bounds
//! of everything drawn. By default one horizontal plane just below
//! everything (Elevation -1), endless, with lines at powers of ten of the scene unit chosen from
//! the size of a pixel at the camera's target, so it reads the same on a ring and on a
//! building. The display pass draws it per pixel (`floor_planes` in `shaders/display.wgsl`).

use crate::Camera;

/// Finer lines appear when they are this many pixels apart, and reach full strength ten times
/// farther apart, when the next finer level starts to appear.
const MIN_PIXELS: f32 = 8.0;

/// A plane at Elevation -1 or 1 sits this fraction of the bounds' side past them, so a flat
/// bottom resting on it does not flicker between the two.
const BELOW: f32 = 0.002;

/// How the floor grid is drawn (catalog item `canvas.floor-grid`): which planes, where they
/// sit, how big they are and how they are filled. `FloorGrid::default()` is the one horizontal
/// plane just below everything drawn, endless and filled with the background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorGrid {
    /// The planes shown, each by the axis it is square to: x, y (the floor) and z.
    pub planes: [bool; 3],
    /// Where each plane sits along its axis, -1 to 1: -1 just past the low side of the bounds
    /// of everything drawn, 0 through their center, 1 just past the high side.
    pub elevation: f32,
    /// Each plane sits just past the side of the bounds away from the eye, so it is always
    /// behind what is drawn; `elevation` is then not used.
    pub snap: bool,
    /// 0: endless, lines at powers of ten of the scene unit. Otherwise a square this many times
    /// the bounds' largest side, centered on them, with `tiles` cells a side.
    pub size: f32,
    /// Cells along a side of a sized grid.
    pub tiles: u32,
    /// 0 opaque, 1 slightly see-through, 2 strongly see-through, 3 slightly see-through with
    /// the parts see-through where they face the eye (a Fresnel look).
    pub fill: u32,
    /// How strongly a plane's lines take its axis color (red x, green y, blue z), 0 to 1.
    pub tint: f32,
}

impl Default for FloorGrid {
    fn default() -> Self {
        Self {
            planes: [false, true, false],
            elevation: -1.0,
            snap: false,
            size: 0.0,
            tiles: 16,
            fill: 0,
            tint: 0.0,
        }
    }
}

impl FloorGrid {
    /// How much a plane's fill hides what lies behind it, by fill mode.
    pub(crate) fn fill_opacity(&self) -> f32 {
        [1.0, 0.6, 0.25, 0.6][self.fill.min(3) as usize]
    }

    /// The planes shown, as bits: 1 x, 2 y, 4 z.
    pub(crate) fn plane_bits(&self) -> u32 {
        (0..3).filter(|&a| self.planes[a]).map(|a| 1 << a).sum()
    }
}

/// Where the planes lie and how dense they are, measured from the eye: every number the display
/// pass works with is small near the eye, so a view a millionth of the scene's size still
/// places its lines exactly (scene positions far from the origin would not, in f32).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Floor {
    /// Each plane's place along its axis minus the eye's.
    pub planes: [f32; 3],
    /// The finer level's line spacing in scene units; the coarser level is `ratio` times it.
    pub spacing: f32,
    /// The finer level's strength, 0 when it has just appeared to 1.
    pub fine: f32,
    pub ratio: f32,
    /// Half the side of a sized grid (0: endless), and its center minus the eye.
    pub half: f32,
    pub center: [f32; 3],
    /// The eye past the coarser line before it along each axis (0 to `ratio` spacings).
    pub past: [f32; 3],
    /// The eye, for the axis lines.
    pub eye: [f32; 3],
}

impl Floor {
    pub const NONE: Floor = Floor {
        planes: [-1.0; 3],
        spacing: 1.0,
        fine: 0.0,
        ratio: 10.0,
        half: 0.0,
        center: [0.0; 3],
        past: [0.0; 3],
        eye: [0.0; 3],
    };

    /// The grid `style` places around `points` (the chunk bounds' corners), seen by `camera`
    /// in a viewport `size` pixels wide and tall; `None` when nothing is drawn.
    pub fn of(
        camera: &Camera,
        points: &[[f32; 3]],
        size: [u32; 2],
        style: &FloorGrid,
    ) -> Option<Floor> {
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in points {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        if lo.iter().chain(&hi).any(|v| !v.is_finite()) {
            return None;
        }
        let pixel = camera.pixel_size(camera.distance, size.map(|v| v.max(1) as f32));
        let eye = camera.eye().map(f64::from);
        let e = style.elevation.clamp(-1.0, 1.0);
        let planes = std::array::from_fn(|a| {
            let margin = BELOW * (hi[a] - lo[a]).max(pixel);
            let (low, high) = (lo[a] - margin, hi[a] + margin);
            let mid = f32::midpoint(lo[a], hi[a]);
            let place = match (style.snap, eye[a] > f64::from(mid)) {
                (true, true) => low,
                (true, false) => high,
                // Exactly `low` at -1, `mid` at 0 and `high` at 1.
                (false, _) if e < 0.0 => low + (1.0 + e) * (mid - low),
                (false, _) => mid + e * (high - mid),
            };
            (f64::from(place) - eye[a]) as f32
        });
        let [lo, hi] = [lo, hi].map(|b| b.map(f64::from));
        let center: [f64; 3] = std::array::from_fn(|a| f64::midpoint(lo[a], hi[a]));
        let largest = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        let side = f64::from(style.size.max(0.0)) * largest.max(f64::from(pixel));
        let (spacing, fine, ratio, half) = if side > 0.0 {
            let tiles = f64::from(style.tiles.max(1));
            (side / tiles, 1.0, tiles, side / 2.0)
        } else {
            // The smallest power of ten at least MIN_PIXELS apart on screen.
            let level = libm::ceil(libm::log10(f64::from(MIN_PIXELS * pixel)));
            let spacing = libm::pow(10.0, level);
            let on_screen = (spacing / f64::from(pixel)) as f32;
            let fine = ((on_screen - MIN_PIXELS) / (9.0 * MIN_PIXELS)).clamp(0.0, 1.0);
            (spacing, fine, 10.0, 0.0)
        };
        // Lines run from the origin, or from a sized grid's corner.
        let anchor = |a: usize| if half > 0.0 { center[a] - half } else { 0.0 };
        let period = ratio * spacing;
        let past = |a: usize| {
            let c = eye[a] - anchor(a);
            (c - libm::floor(c / period) * period) as f32
        };
        Some(Floor {
            planes,
            spacing: spacing as f32,
            fine,
            ratio: ratio as f32,
            half: half as f32,
            center: std::array::from_fn(|a| (center[a] - eye[a]) as f32),
            past: std::array::from_fn(past),
            eye: eye.map(|v| v as f32),
        })
    }
}
