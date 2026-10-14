//! The chart cutter (decision 0061 section 1, UV2's amendment): charts grown by one priority
//! queue, each cut to a disc by tree and cotree, checked and halved until it passes, and their
//! seams found by the one seam rule of section 8.

mod context;
mod disc;
mod edges;
mod grow;
mod local;
mod mirror;
mod path;
mod project;
mod split;

use context::{Ctx, NONE};
pub use edges::seams;
use grow::State;
use local::{Local, places};
pub(super) use mirror::Mirror;
pub(super) use project::upright;

use super::{Chart, Charts, Refusal, Steer, Surface};

/// Cuts the part into charts, each checked on [`Chart::project`].
///
/// # Errors
/// [`Refusal::Protected`] when a cut must cross a protected area, and [`Refusal::Unpaired`]
/// when symmetry is on and a point has no mirror partner.
pub fn cut(surface: Surface, steer: &Steer) -> Result<Charts, Refusal> {
    cut_with(surface, steer, Chart::project)
}

/// Cuts the part into charts, each checked on what `flatten` makes of it: a UV per point of the
/// chart's mesh.
///
/// # Errors
/// As [`cut`].
pub fn cut_with<F>(surface: Surface, steer: &Steer, flatten: F) -> Result<Charts, Refusal>
where
    F: Fn(&Chart, Surface) -> Vec<[f32; 2]> + Sync,
{
    let ctx = Ctx::new(surface, steer)?;
    let mut state = grow::grow(&ctx);
    split::settle(&ctx, &mut state, &flatten)?;
    if let Some(mirror) = &ctx.mirror {
        mirror.apply(&mut state, surface.faces);
    }
    Ok(finish(&ctx, &state))
}

/// The charts numbered by lowest face, and their seams: every edge whose faces differ at either
/// end in the wedges each chart's cuts and hard borders leave.
fn finish(ctx: &Ctx, state: &State) -> Charts {
    let faces = ctx.surface.faces;
    let places = places(&state.chart, state.count as usize);
    let cut = |h: usize| (ctx.hard[h / 4] | state.cut[h / 4]) >> (h % 4) & 1 == 1;
    let at = (state.chart.as_slice(), places.slot.as_slice());
    let wedges = gs_par::map(places.faces.len(), |c| {
        Local::new(faces, &ctx.edges, at, places.faces[c].clone(), cut).chart
    });
    let mut label = vec![NONE; 4 * faces.len()];
    let mut next = 0;
    for chart in &wedges {
        for (i, &f) in chart.faces.iter().enumerate() {
            for (k, &w) in chart.corners[i]
                .iter()
                .enumerate()
                .filter(|&(_, &w)| w != NONE)
            {
                label[4 * f as usize + k] = next + w;
            }
        }
        next += chart.points.len() as u32;
    }
    let seams = ctx.edges.seams_by(faces, |a, b| label[a] == label[b]);
    let mut number = vec![NONE; places.faces.len()];
    let mut count = 0;
    let chart = (state.chart.iter())
        .map(|&c| {
            if number[c as usize] == NONE {
                number[c as usize] = count;
                count += 1;
            }
            number[c as usize]
        })
        .collect();
    let bit = |masks: &[u8], h: u32| masks[h as usize / 4] >> (h % 4) & 1 == 1;
    let added_seams = (ctx.edges.runs())
        .filter(|run| run.iter().any(|&(_, h)| bit(&seams, h)))
        .filter(|run| (ctx.kept.as_deref()).is_none_or(|k| !run.iter().any(|&(_, h)| bit(k, h))))
        .count();
    Charts {
        chart,
        count,
        seams,
        added_seams,
    }
}
