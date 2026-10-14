//! The floor grid (catalog item `canvas.floor-grid`, research 14 phase D, decision 0019): its
//! lines are as far apart on screen at three zooms a thousand times apart, between 8 and 80
//! pixels for the finer level, and it lies under the parts, hidden where they are in front.

use crate::common::{BACKGROUND, front_ortho, lavapipe, luma, snap, upload};
use crate::scene;
use gs_render::{Camera, FloorGrid, GpuMesh, Image, Preset, Projection, RenderSize, Shading, View};

const SIZE: u32 = 256;

/// Columns where a grid line crosses row `y`, as the centers of runs of pixels brighter than
/// the background.
fn lines(image: &Image, y: u32) -> Vec<f32> {
    let lit = |x: u32| luma(image.pixel(x, y)) > luma(BACKGROUND) + 6;
    let mut centers = Vec::new();
    let mut x = 0;
    while x < SIZE {
        if lit(x) {
            let start = x;
            while x < SIZE && lit(x) {
                x += 1;
            }
            centers.push((start + x - 1) as f32 / 2.0);
        }
        x += 1;
    }
    centers
}

#[test]
fn grid_lines_keep_their_spacing_on_screen_at_any_zoom() {
    let r = lavapipe();
    // A small part far outside every view below: it only sets where the floor lies.
    let part = scene::plain(
        vec![
            [500.0, 0.0, 500.0],
            [501.0, 0.0, 500.0],
            [500.0, 1.0, 500.0],
        ],
        vec![[0, 1, 2, u32::MAX]],
    );
    let mesh = upload(r, &part);
    let mut spacings = Vec::new();
    for zoom in [1.0f32, 1e-3, 1e-6] {
        // Looking straight down in parallel at the origin, which lies on grid lines at every
        // level, so the pattern is the same at every zoom. At 0.62 a pixel is 0.002 across, so
        // the finer lines (0.1 apart) are 50 pixels apart and well past fading in.
        let camera = Camera {
            distance: 0.62 * zoom,
            target: [0.0, 5.0, 0.0],
            projection: Projection::Orthographic,
            ..Camera::preset(Preset::Top)
        };
        let view = View {
            grid: true,
            ..View::new(camera, Shading::Clay)
        };
        let size = RenderSize {
            samples: 4,
            ..RenderSize::square(SIZE)
        };
        let image = r.snapshot(&mesh, &view, size).expect("renders");
        // A row clear of the axis lines through the middle.
        let found = lines(&image, SIZE / 2 + 20);
        let gaps: Vec<f32> = found.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.len() >= 3, "zoom {zoom}: lines at {found:?}");
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        assert!(
            (8.0..80.0).contains(&mean) && gaps.iter().all(|g| (g - mean).abs() <= 1.5),
            "zoom {zoom}: even lines 8 to 80 pixels apart, found {found:?}"
        );
        spacings.push(mean);
    }
    assert!(
        spacings.iter().all(|s| (s - spacings[0]).abs() <= 1.0),
        "the same spacing on screen at every zoom: {spacings:?}"
    );
}

#[test]
fn the_grid_lies_under_the_parts() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(3));
    let camera = Camera::fitted(Preset::Iso, &mesh.fit_points(), 1.0);
    let plain = View::new(camera, Shading::Clay);
    let gridded = View {
        grid: true,
        ..plain
    };
    let size = RenderSize::square(SIZE);
    let without = r.snapshot(&mesh, &plain, size).expect("renders");
    let with = r.snapshot(&mesh, &gridded, size).expect("renders");
    let mut covered = 0;
    let mut lit_background = 0;
    for (p, q) in with.rgba.chunks(4).zip(without.rgba.chunks(4)) {
        if q == BACKGROUND {
            lit_background += usize::from(p != BACKGROUND);
        } else {
            assert_eq!(p, q, "the sphere hides the grid behind and under it");
            covered += 1;
        }
    }
    assert!(covered > 1000, "the sphere is in view");
    assert!(
        lit_background > 1000,
        "grid lines show around the sphere ({lit_background} pixels)"
    );
}

/// What a grid styled `floor` changes in a Clay picture of `mesh` through `camera`: the
/// part's pixels changed above the picture's middle row and below it, and the background
/// pixels it lit.
fn changes(mesh: &GpuMesh, camera: Camera, floor: FloorGrid) -> (usize, usize, usize) {
    let r = lavapipe();
    let plain = View::new(camera, Shading::Clay);
    let gridded = View {
        grid: true,
        floor,
        ..plain
    };
    let without = snap(r, mesh, &plain, SIZE);
    let with = snap(r, mesh, &gridded, SIZE);
    let (mut above, mut below, mut lit) = (0, 0, 0);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (p, q) = (with.pixel(x, y), without.pixel(x, y));
            if q == BACKGROUND {
                lit += usize::from(p != BACKGROUND);
            } else if p != q && y < SIZE / 2 {
                above += 1;
            } else if p != q {
                below += 1;
            }
        }
    }
    (above, below, lit)
}

/// Elevation -1 (the default) puts the floor just below the part, so seen from above it hides
/// none of it; 0 puts it through the part's center, so it lies in front of the lower half
/// only (seen from above, the part's center is the picture's middle); 1 just above, in front of
/// all of it. Seen from below, the floor at -1 is in front of the part, and Snap moves it past
/// the part's far side.
#[test]
fn elevation_minus_one_lies_below_the_part_and_zero_through_its_center() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(3));
    let iso = Camera::fitted(Preset::Iso, &mesh.fit_points(), 1.0);
    let at = |elevation| FloorGrid {
        elevation,
        ..FloorGrid::default()
    };
    let (above, below, lit) = changes(&mesh, iso, FloorGrid::default());
    assert_eq!((above, below), (0, 0), "-1: the part hides the floor");
    assert!(lit > 1000, "-1: the floor shows around the part ({lit})");
    let (above, below, _) = changes(&mesh, iso, at(0.0));
    assert!(
        above == 0 && below > 200,
        "0: lines over the lower half only ({above} above, {below} below)"
    );
    let (above, below, _) = changes(&mesh, iso, at(1.0));
    assert!(
        above > 200 && below > 200,
        "1: lines over all of it ({above} above, {below} below)"
    );
    let from_below = Camera::fitted(Preset::Bottom, &mesh.fit_points(), 1.0);
    let (above, below, _) = changes(&mesh, from_below, FloorGrid::default());
    assert!(above + below > 400, "from below, -1 is in front");
    let snapped = FloorGrid {
        snap: true,
        ..FloorGrid::default()
    };
    let (above, below, lit) = changes(&mesh, from_below, snapped);
    assert_eq!((above, below), (0, 0), "Snap puts it behind the part");
    assert!(
        lit > 1000,
        "the snapped floor shows around the part ({lit})"
    );
}

/// Each axis switch shows its own plane: with the lines tinted by their axis, the X plane's
/// lines are red, the Y plane's green and the Z plane's blue.
#[test]
fn each_axis_shows_its_own_plane() {
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(3));
    let iso = Camera::fitted(Preset::Iso, &mesh.fit_points(), 1.0);
    let plain = snap(r, &mesh, &View::new(iso, Shading::Clay), SIZE);
    for axis in 0..3 {
        let mut planes = [false; 3];
        planes[axis] = true;
        let view = View {
            grid: true,
            floor: FloorGrid {
                planes,
                tint: 1.0,
                ..FloorGrid::default()
            },
            ..View::new(iso, Shading::Clay)
        };
        let with = snap(r, &mesh, &view, SIZE);
        // How many clearly lit background pixels gained most in each channel.
        let mut most = [0usize; 3];
        for (p, q) in with.rgba.chunks(4).zip(plain.rgba.chunks(4)) {
            let gain = |c: usize| i32::from(p[c]) - i32::from(BACKGROUND[c]);
            let top = (0..3).max_by_key(|&c| gain(c)).unwrap_or(0);
            if q == BACKGROUND && gain(top) > 20 {
                most[top] += 1;
            }
        }
        let shown = (0..3).max_by_key(|&c| most[c]).unwrap_or(3);
        assert!(
            shown == axis && most[axis] > 500,
            "plane {axis} alone shows lines of its own color: {most:?}"
        );
    }
}

/// Fill mode 3 draws the parts see-through where they face the eye and solid edge-on: a
/// sphere in front of a plane shows more of what is behind it at its middle than at its rim.
#[test]
fn fill_mode_3_shows_through_surfaces_facing_the_eye_more_than_edge_on_ones() {
    const N: u32 = 128;
    let r = lavapipe();
    let mesh = upload(r, &scene::icosphere(4));
    let floor = |fill| FloorGrid {
        planes: [false, false, true],
        snap: true,
        fill,
        ..FloorGrid::default()
    };
    let view = |fill| View {
        grid: true,
        floor: floor(fill),
        ..front_ortho(Shading::Clay)
    };
    let solid = snap(r, &mesh, &view(0), N);
    let fresnel = snap(r, &mesh, &view(3), N);
    // How far each pixel moved toward the background: 0 solid, 1 fully see-through.
    let (mut middle, mut rim) = (Vec::new(), Vec::new());
    let radius = N as f32 / 2.2;
    for y in 0..N {
        for x in 0..N {
            let off = [x as f32 - 63.5, y as f32 - 63.5];
            let d = (off[0] * off[0] + off[1] * off[1]).sqrt() / radius;
            // Clear of the axis lines through the middle, which lie behind the sphere.
            if off[0].abs() < 4.0 || off[1].abs() < 4.0 {
                continue;
            }
            let (p, q) = (fresnel.pixel(x, y), solid.pixel(x, y));
            let span: i32 = (0..3)
                .map(|c| i32::from(BACKGROUND[c]) - i32::from(q[c]))
                .sum();
            let moved: i32 = (0..3).map(|c| i32::from(p[c]) - i32::from(q[c])).sum();
            if span.abs() < 30 {
                continue;
            }
            let through = moved as f32 / span as f32;
            if d < 0.4 {
                middle.push(through);
            } else if (0.95..0.99).contains(&d) {
                rim.push(through);
            }
        }
    }
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let (m, e) = (mean(&middle), mean(&rim));
    assert!(
        middle.len() > 200 && rim.len() > 100,
        "{} middle and {} rim pixels",
        middle.len(),
        rim.len()
    );
    assert!(
        m > 0.5 && m > e + 0.1,
        "the middle shows through more than the rim: {m} against {e}"
    );
}
