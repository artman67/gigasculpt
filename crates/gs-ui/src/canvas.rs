//! The canvas a 3D view is drawn on, as the session's display sets it (catalog items
//! `canvas.floor-grid`, `canvas.wireframe-view` and `canvas.background`): the engine's settings
//! as gs-render takes them, for every view `draw.rs` draws.

use gs_engine::DisplaySettings;
use gs_render::{Background, FloorGrid, View, WireStyle};

/// `view` on the canvas `display` sets.
pub(crate) fn canvas<'a>(display: &DisplaySettings, view: View<'a>) -> View<'a> {
    let (g, w, b) = (&display.floor_grid, &display.wire, &display.background);
    View {
        floor: FloorGrid {
            planes: [g.x, g.y, g.z],
            elevation: g.elevation,
            snap: g.snap,
            size: g.size,
            tiles: g.tiles,
            fill: g.fill,
            tint: g.tint,
        },
        wire_style: WireStyle {
            line: w.line,
            fill: w.fill,
            color: w.color,
            opacity: w.opacity,
        },
        background: Background {
            color: b.color,
            gradient: b.gradient,
            center: b.center,
            softness: b.softness,
        },
        ..view
    }
}
