//! The packer (decision 0061 section 1, step UV4). Each chart is scaled to one UV area per surface
//! area, times 4 to its mean density hint (section 6), and a chart and its twin (section 7) are
//! one unit. At a trial scale each unit is rasterized, grown by half the padding and turned 4
//! ways; the units are then placed largest first at the lowest free (y, x). The rows are tested in
//! parallel and the lowest (y, x, turn) wins, so the bits do not depend on the thread count. Eight
//! steps of binary search find the largest scale at which all fit. A chart is only scaled, moved
//! and turned by quarter turns, never mirrored.

use std::cmp::Reverse;
use std::fmt;

use gs_protocol::{arity, corners, face_triangles};

use super::Corners;
use super::metrics::{Layout, triangles};

/// Steps of the binary search for the scale.
const STEPS: usize = 8;

/// How near a triangle, in pixels, a raster cell counts as touched, so that UVs rounded to `f32`
/// cannot reach a cell left empty.
const MARGIN: f64 = 0.01;

/// The most candidate rows tested at once.
const BATCH: usize = 64;

/// How the charts are packed (decision 0061 sections 6 to 8).
#[derive(Clone, Copy, Debug)]
pub struct Pack<'a> {
    /// The map's side in pixels, which sets the padding's size (default 2048).
    pub map_size: u32,
    /// The fewest empty pixels between two charts at `map_size` (default 8).
    pub padding_px: u32,
    /// `uv_density` per point, from -1 to 1: a chart's UV area per surface area is scaled by 4
    /// to its faces' mean. Left out reads 0.
    pub density: Option<&'a [f32]>,
    /// The charts' mirror twins and how they are laid out.
    pub twins: Option<Twins<'a>>,
}

impl Default for Pack<'_> {
    fn default() -> Self {
        Pack {
            map_size: 2048,
            padding_px: 8,
            density: None,
            twins: None,
        }
    }
}

/// Each chart's twin across the mirror plane (decision 0061 section 7). A twin's UVs are its
/// partner's with u negated, as the flattener leaves them.
#[derive(Clone, Copy, Debug)]
pub struct Twins<'a> {
    /// Each chart's twin, the chart itself when it has none.
    pub twin: &'a [u32],
    /// How a chart and its twin are laid out.
    pub mirror: Mirror,
}

/// How twins are laid out (decision 0061 section 7, `mirror`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirror {
    /// Side by side as one unit, every texel unique.
    SideBySide,
    /// The twin takes its partner's UVs.
    Shared,
}

/// The charts do not fit the square with their padding at any scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crowded {
    /// The charts to pack.
    pub charts: usize,
    /// The square's side in pixels.
    pub map_size: u32,
    /// The padding in pixels.
    pub padding_px: u32,
}

impl fmt::Display for Crowded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} charts do not fit a {} px square with {} px between them; lower padding_px or \
             raise map_size",
            self.charts, self.map_size, self.padding_px
        )
    }
}

/// Packs `layout`'s charts into the unit square: each scaled to one texel density (times 2 to its
/// mean density hint), turned by quarter turns and placed with `padding_px` between charts and
/// half of it at the square's edge, at the largest scale the search finds. Refused when the
/// charts do not fit at any scale it tries.
///
/// # Panics
/// If `twins` does not pair the charts: each chart's twin must have faces and have it as its twin.
pub fn pack(layout: &Layout, options: &Pack) -> Result<Vec<Corners>, Crowded> {
    let count = layout.chart.iter().max().map_or(0, |&c| c as usize + 1);
    let members = members(layout.chart, count);
    let sums = gs_par::map(count, |c| sums(layout, options.density, &members[c]));
    let units = units(&sums, &members, options.twins);
    let job = Job {
        layout,
        members,
        sums,
        units,
        size: options.map_size as usize,
        padding: options.padding_px as usize,
    };
    // At `t` the units cover `t * t * weight` of the square, so `t` stays below where that is 1.
    let weight: f64 = (job.units.iter())
        .map(|u| u.base * u.base * u.pieces.iter().map(|&c| job.sums[c].area2).sum::<f64>())
        .sum();
    let (mut lo, mut hi) = (
        0.0,
        if weight > 0.0 {
            1.0 / weight.sqrt()
        } else {
            1.0
        },
    );
    let mut best = None;
    for _ in 0..STEPS {
        let mid = f64::midpoint(lo, hi);
        match trial(&job, mid) {
            Some(found) => {
                best = Some(found);
                lo = mid;
            }
            None => hi = mid,
        }
    }
    let (frames, spots) = best.ok_or(Crowded {
        charts: count,
        map_size: options.map_size,
        padding_px: options.padding_px,
    })?;
    Ok(write(&job, &frames, &spots))
}

/// What every trial reads.
struct Job<'a> {
    layout: &'a Layout<'a>,
    /// Each chart's faces.
    members: Vec<Vec<usize>>,
    sums: Vec<Sums>,
    units: Vec<Unit>,
    /// The square's side and the padding, in pixels.
    size: usize,
    padding: usize,
}

/// Each chart's faces, lowest first.
fn members(chart: &[u32], count: usize) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new(); count];
    for (f, &c) in chart.iter().enumerate() {
        out[c as usize].push(f);
    }
    out
}

/// One chart's sums over its faces.
#[derive(Clone, Copy, Debug)]
struct Sums {
    /// Its surface area.
    area3: f64,
    /// Its UV area.
    area2: f64,
    /// Each face's surface area times the mean of its corners' density hints.
    dense: f64,
    /// The lowest and highest corners of its UV box.
    lo: [f32; 2],
    hi: [f32; 2],
}

/// The sums of the chart with faces `faces`.
fn sums(layout: &Layout, density: Option<&[f32]>, faces: &[usize]) -> Sums {
    let mut s = Sums {
        area3: 0.0,
        area2: 0.0,
        dense: 0.0,
        lo: [f32::INFINITY; 2],
        hi: [f32::NEG_INFINITY; 2],
    };
    for &f in faces {
        let (face, uv) = (layout.surface.faces[f], layout.uvs[f]);
        let (mut area3, mut area2) = (0.0, 0.0);
        for t in triangles(face, |k| layout.surface.points[face[k] as usize], |k| uv[k]) {
            area3 += t.area3;
            area2 += t.area2.abs();
        }
        let real = corners(&face);
        let hint = density.map_or(0.0, |d| {
            real.iter().map(|&v| f64::from(d[v as usize])).sum::<f64>() / real.len() as f64
        });
        s.area3 += area3;
        s.area2 += area2;
        s.dense += area3 * hint;
        for p in &uv[..real.len()] {
            s.lo = [s.lo[0].min(p[0]), s.lo[1].min(p[1])];
            s.hi = [s.hi[0].max(p[0]), s.hi[1].max(p[1])];
        }
    }
    s
}

/// Charts packed as one.
#[derive(Clone, Debug)]
struct Unit {
    /// A chart alone, or a chart and its twin side by side, lower first.
    pieces: Vec<usize>,
    /// A twin laid on its partner, `pieces[0]`, under `Mirror::Shared`.
    shadow: Option<usize>,
    /// Its scale before the shared one: the square root of its surface area over its UV area,
    /// times 2 to its mean density hint. Twins are scaled as one.
    base: f64,
}

/// The units: each chart with faces once, a twin with its partner.
fn units(sums: &[Sums], members: &[Vec<usize>], twins: Option<Twins>) -> Vec<Unit> {
    let mut out = Vec::new();
    for c in (0..sums.len()).filter(|&c| !members[c].is_empty()) {
        let t = twins.map_or(c, |tw| tw.twin.get(c).map_or(usize::MAX, |&t| t as usize));
        assert!(
            twins.is_none_or(|tw| tw.twin.get(t) == Some(&(c as u32))
                && members.get(t).is_some_and(|m| !m.is_empty())),
            "chart {c}'s twin {t} has faces and has chart {c} as its twin"
        );
        if t < c {
            continue;
        }
        let (pieces, shadow) = match twins.map(|tw| tw.mirror) {
            Some(Mirror::Shared) if t != c => (vec![c], Some(t)),
            _ if t != c => (vec![c, t], None),
            _ => (vec![c], None),
        };
        let all = || pieces.iter().chain(&shadow).map(|&k| sums[k]);
        let area3: f64 = all().map(|s| s.area3).sum();
        let area2: f64 = all().map(|s| s.area2).sum();
        let dense: f64 = all().map(|s| s.dense).sum();
        let base = if area3 > 0.0 && area2 > 0.0 {
            (area3 / area2).sqrt() * libm::exp2(dense / area3)
        } else {
            0.0
        };
        out.push(Unit {
            pieces,
            shadow,
            base,
        });
    }
    out
}

/// A unit at one trial scale, before turning, in pixels.
#[derive(Clone, Debug)]
struct Frame {
    /// Pixels per UV unit of its charts as they came.
    k: f64,
    /// Each piece's left edge.
    left: Vec<f64>,
    /// Half the padding: the margin round the unit, and its pieces' bottom edge.
    margin: f64,
    width: usize,
    height: usize,
}

impl Frame {
    /// Unit `unit` at trial scale `t`: its pieces in a row with the padding between them, half
    /// the padding round them, and each piece's box rounded up to whole cells.
    fn new(job: &Job, unit: &Unit, t: f64) -> Frame {
        let r = job.padding.div_ceil(2);
        let k = t * job.size as f64 * unit.base;
        // The cells piece `c`'s box spans along axis `i`, kept within the square's side.
        let cells = |c: usize, i: usize| {
            let s = job.sums[c];
            let span = (f64::from(s.hi[i]) - f64::from(s.lo[i])) * k + MARGIN;
            span.floor().min(job.size as f64) as usize + 1
        };
        let (mut left, mut x, mut height) = (Vec::new(), r, 0);
        for (j, &c) in unit.pieces.iter().enumerate() {
            if j > 0 {
                x += job.padding;
            }
            left.push(x as f64);
            x += cells(c, 0);
            height = height.max(cells(c, 1));
        }
        Frame {
            k,
            left,
            margin: r as f64,
            width: x + r,
            height: height + 2 * r,
        }
    }

    /// Where UV `uv` of piece `j`, whose box starts at `lo`, falls in the unit.
    fn at(&self, j: usize, lo: [f32; 2], uv: [f32; 2]) -> [f64; 2] {
        [
            (f64::from(uv[0]) - f64::from(lo[0])) * self.k + self.left[j],
            (f64::from(uv[1]) - f64::from(lo[1])) * self.k + self.margin,
        ]
    }

    /// Point `p` of the unit after `turn` quarter turns counterclockwise, as `Shape::new` turns
    /// its cells.
    fn turned(&self, p: [f64; 2], turn: usize) -> [f64; 2] {
        let (w, h) = (self.width as f64, self.height as f64);
        match turn {
            0 => p,
            1 => [h - p[1], p[0]],
            2 => [w - p[0], h - p[1]],
            _ => [p[1], w - p[0]],
        }
    }
}

/// Where a unit lands: the lowest corner of its turned raster, in pixels, and its quarter turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Spot {
    x: usize,
    y: usize,
    turn: usize,
}

/// Each unit's frame and spot at trial scale `t`, when all fit.
fn trial(job: &Job, t: f64) -> Option<(Vec<Frame>, Vec<Spot>)> {
    let frames: Vec<Frame> = job.units.iter().map(|u| Frame::new(job, u, t)).collect();
    if (frames.iter()).any(|f| f.width > job.size || f.height > job.size) {
        return None;
    }
    let shapes = gs_par::map(job.units.len(), |u| {
        let (f, cells) = (&frames[u], raster(job, u, &frames[u]));
        [0, 1, 2, 3].map(|turn| Shape::new(&cells, f.width, f.height, turn))
    });
    if shapes.iter().map(|s| s[0].area).sum::<usize>() > job.size * job.size {
        return None;
    }
    let mut order: Vec<usize> = (0..shapes.len()).collect();
    order.sort_by_key(|&u| (Reverse(shapes[u][0].area), u));
    let mut grid = Grid::new(job.size);
    let mut spots = vec![Spot::default(); shapes.len()];
    for u in order {
        let spot = grid.spot(&shapes[u])?;
        grid.take(&shapes[u][spot.turn], spot);
        spots[u] = spot;
    }
    Some((frames, spots))
}

/// The cells unit `u`'s triangles touch, grown by half the padding, row by row.
fn raster(job: &Job, u: usize, frame: &Frame) -> Vec<u8> {
    let mut cells = vec![0_u8; frame.width * frame.height];
    let (points, faces) = (job.layout.surface.points, job.layout.surface.faces);
    for (j, &c) in job.units[u].pieces.iter().enumerate() {
        for &f in &job.members[c] {
            let face = faces[f];
            for t in face_triangles(face, |k| points[face[k] as usize]) {
                let q = t.map(|k| frame.at(j, job.sums[c].lo, job.layout.uvs[f][k]));
                touch(&q, frame.width, &mut cells);
            }
        }
    }
    grow(&cells, frame.width, job.padding.div_ceil(2))
}

/// Marks each cell of a raster `w` wide that triangle `q` comes within `MARGIN` of.
fn touch(q: &[[f64; 2]; 3], w: usize, cells: &mut [u8]) {
    let h = cells.len() / w;
    let (lo, hi) = (q.iter()).fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
        (a.min(p[1]), b.max(p[1]))
    });
    let first = (lo - MARGIN).floor().max(0.0) as usize;
    let last = ((hi + MARGIN).floor().max(0.0) as usize).min(h - 1);
    for y in first..=last {
        let Some((a, b)) = across(q, y as f64 - MARGIN, y as f64 + 1.0 + MARGIN) else {
            continue;
        };
        let x0 = (a - MARGIN).floor().max(0.0) as usize;
        let x1 = ((b + MARGIN).floor().max(0.0) as usize).min(w - 1);
        if x0 <= x1 {
            cells[y * w + x0..=y * w + x1].fill(1);
        }
    }
}

/// The x range of triangle `q` between heights `y0` and `y1`, if it reaches there.
fn across(q: &[[f64; 2]; 3], y0: f64, y1: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for e in 0..3 {
        let (a, b) = (q[e], q[(e + 1) % 3]);
        if (y0..=y1).contains(&a[1]) {
            (lo, hi) = (lo.min(a[0]), hi.max(a[0]));
        }
        for y in [y0, y1] {
            if (a[1] < y) != (b[1] < y) {
                let x = a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]);
                (lo, hi) = (lo.min(x), hi.max(x));
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// `cells`, a raster `w` wide, grown by `r` cells in every direction, corners included.
fn grow(cells: &[u8], w: usize, r: usize) -> Vec<u8> {
    let h = cells.len() / w;
    let mut wide = cells.to_vec();
    for (src, dst) in cells.chunks_exact(w).zip(wide.chunks_exact_mut(w)) {
        for d in 1..=r.min(w - 1) {
            or_into(&mut dst[..w - d], &src[d..]);
            or_into(&mut dst[d..], &src[..w - d]);
        }
    }
    let mut out = wide.clone();
    for d in 1..=r.min(h - 1) {
        let n = cells.len() - d * w;
        or_into(&mut out[..n], &wide[d * w..]);
        or_into(&mut out[d * w..], &wide[..n]);
    }
    out
}

/// Sets each cell of `dst` that is set in `src`.
fn or_into(dst: &mut [u8], src: &[u8]) {
    for (a, b) in dst.iter_mut().zip(src) {
        *a |= b;
    }
}

/// A unit's cells after quarter turns, as runs: row y's are `runs[start[y]..start[y + 1]]`, each
/// `[x, end)`.
struct Shape {
    width: usize,
    height: usize,
    start: Vec<usize>,
    runs: Vec<[usize; 2]>,
    /// Each row's longest run.
    need: Vec<usize>,
    /// Its longest run's row and length.
    widest: (usize, usize),
    /// Its cells.
    area: usize,
}

impl Shape {
    /// The cells of a raster `w` by `h` turned counterclockwise `turn` times: one turn takes cell
    /// (x, y) to (h - 1 - y, x).
    fn new(cells: &[u8], w: usize, h: usize, turn: usize) -> Shape {
        let (width, height) = if turn.is_multiple_of(2) {
            (w, h)
        } else {
            (h, w)
        };
        // The raster's cell under turned cell (x, y).
        let index = |x: usize, y: usize| match turn {
            0 => y * w + x,
            1 => (h - 1 - x) * w + y,
            2 => (h - 1 - y) * w + (w - 1 - x),
            _ => x * w + (w - 1 - y),
        };
        let mut s = Shape {
            width,
            height,
            start: vec![0],
            runs: Vec::new(),
            need: vec![0; height],
            widest: (0, 0),
            area: 0,
        };
        for y in 0..height {
            let mut x = 0;
            while x < width {
                if cells[index(x, y)] == 0 {
                    x += 1;
                    continue;
                }
                let a = x;
                while x < width && cells[index(x, y)] != 0 {
                    x += 1;
                }
                s.runs.push([a, x]);
                s.area += x - a;
                s.need[y] = s.need[y].max(x - a);
                if x - a > s.widest.1 {
                    s.widest = (y, x - a);
                }
            }
            s.start.push(s.runs.len());
        }
        s
    }

    fn row(&self, y: usize) -> &[[usize; 2]] {
        &self.runs[self.start[y]..self.start[y + 1]]
    }
}

/// The square's taken pixels, a bit each, row by row, and each row's longest free run.
struct Grid {
    size: usize,
    words: usize,
    bits: Vec<u64>,
    free: Vec<usize>,
}

impl Grid {
    fn new(size: usize) -> Grid {
        let words = size.div_ceil(64);
        Grid {
            size,
            words,
            bits: vec![0; words * size],
            free: vec![size; size],
        }
    }

    fn row(&self, y: usize) -> &[u64] {
        &self.bits[y * self.words..(y + 1) * self.words]
    }

    /// The lowest (y, x, turn) at which one of `shapes`, a unit's 4 turns, fits. Candidate rows
    /// are tested in parallel, in batches in order, and the first row that fits wins at its
    /// lowest (x, turn), so the batches' sizes cannot change the pick.
    fn spot(&self, shapes: &[Shape; 4]) -> Option<Spot> {
        // No row of the shape may be longer than the free run of the row it would lie on.
        let may = |s: &Shape, y: usize| {
            y + s.height <= self.size
                && self.free[y + s.widest.0] >= s.widest.1
                && (s.need.iter().zip(&self.free[y..])).all(|(n, f)| f >= n)
        };
        let mut rows = (0..self.size).filter(|&y| shapes.iter().any(|s| may(s, y)));
        let mut batch = 1;
        loop {
            let some: Vec<usize> = rows.by_ref().take(batch).collect();
            if some.is_empty() {
                return None;
            }
            let found = gs_par::map(some.len(), |i| {
                (0..4)
                    .filter(|&t| may(&shapes[t], some[i]))
                    .filter_map(|t| self.lowest_x(&shapes[t], some[i]).map(|x| (x, t)))
                    .min()
            });
            if let Some((y, (x, turn))) =
                (some.iter().zip(found)).find_map(|(&y, f)| f.map(|f| (y, f)))
            {
                return Some(Spot { x, y, turn });
            }
            batch = (batch * 2).min(BATCH);
        }
    }

    /// The lowest x at which `shape` fits with its first row at `y`. A run that meets a taken
    /// pixel moves x just past the last one, which every x between would also meet.
    fn lowest_x(&self, shape: &Shape, y: usize) -> Option<usize> {
        let (mut x, mut hot) = (0, shape.widest.0);
        'next: while x + shape.width <= self.size {
            for i in (hot..shape.height).chain(0..hot) {
                for &[a, b] in shape.row(i) {
                    if let Some(q) = self.last_taken(y + i, x + a, x + b) {
                        (x, hot) = (q + 1 - a, i);
                        continue 'next;
                    }
                }
            }
            return Some(x);
        }
        None
    }

    /// The last taken pixel of row `y` in `[lo, hi)`.
    fn last_taken(&self, y: usize, lo: usize, hi: usize) -> Option<usize> {
        let row = self.row(y);
        let mut end = hi;
        while end > lo {
            let w = (end - 1) / 64;
            let from = lo.max(w * 64);
            let word = row[w] & mask(from - w * 64, end - w * 64);
            if word != 0 {
                return Some(w * 64 + 63 - word.leading_zeros() as usize);
            }
            end = from;
        }
        None
    }

    /// Takes `shape`'s cells at `at`.
    fn take(&mut self, shape: &Shape, at: Spot) {
        for i in 0..shape.height {
            let y = at.y + i;
            let row = &mut self.bits[y * self.words..(y + 1) * self.words];
            for &[a, b] in shape.row(i) {
                let (mut x, hi) = (at.x + a, at.x + b);
                while x < hi {
                    let w = x / 64;
                    let end = hi.min(w * 64 + 64);
                    row[w] |= mask(x - w * 64, end - w * 64);
                    x = end;
                }
            }
            self.free[y] = self.longest_free(y);
        }
    }

    /// Row `y`'s longest run of free pixels.
    fn longest_free(&self, y: usize) -> usize {
        let (mut best, mut run) = (0, 0);
        for (w, &word) in self.row(y).iter().enumerate() {
            // `at` is the first bit of this word not yet counted.
            let (mut rest, mut at) = (word, 0);
            while rest != 0 {
                let one = rest.trailing_zeros() as usize;
                best = best.max(run + one - at);
                at = one + (!(rest >> one)).trailing_zeros() as usize;
                run = 0;
                rest = if at >= 64 { 0 } else { rest & (u64::MAX << at) };
            }
            run += (self.size - w * 64).min(64) - at;
        }
        best.max(run)
    }
}

/// Bits `a` to `b - 1` of a word, for `a < b <= 64`.
fn mask(a: usize, b: usize) -> u64 {
    (u64::MAX >> (64 - b)) & (u64::MAX << a)
}

/// The packed UVs: each corner through its unit's frame, turn and spot into the unit square.
fn write(job: &Job, frames: &[Frame], spots: &[Spot]) -> Vec<Corners> {
    // Each chart's unit and piece, and whether it is a twin laid on that piece.
    let mut home = vec![(0, 0, false); job.sums.len()];
    for (u, unit) in job.units.iter().enumerate() {
        for (j, &c) in unit.pieces.iter().enumerate() {
            home[c] = (u, j, false);
        }
        if let Some(c) = unit.shadow {
            home[c] = (u, 0, true);
        }
    }
    let size = job.size as f64;
    gs_par::map(job.layout.chart.len(), |f| {
        let (u, j, shadow) = home[job.layout.chart[f] as usize];
        let (frame, spot) = (&frames[u], spots[u]);
        let lo = job.sums[job.units[u].pieces[j]].lo;
        let mut out = [[0.0; 2]; 4];
        let real = arity(job.layout.surface.faces[f]);
        for (o, &uv) in out.iter_mut().zip(&job.layout.uvs[f]).take(real) {
            let uv = if shadow { [-uv[0], uv[1]] } else { uv };
            let p = frame.turned(frame.at(j, lo, uv), spot.turn);
            *o = [
                ((spot.x as f64 + p[0]) / size) as f32,
                ((spot.y as f64 + p[1]) / size) as f32,
            ];
        }
        out
    })
}
