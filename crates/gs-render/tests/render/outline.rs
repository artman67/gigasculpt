//! The active part's outline (decision 0033, section 5): with two parts drawn, the marked one
//! gets an orange line just outside its own pixels and the other none; with the view's outline
//! off, or fewer than two parts drawn, the picture is byte for byte what it was.

use crate::common::{BACKGROUND, diff, lavapipe, upload};
use crate::scene;
use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_render::{Camera, Fit, GpuMesh, Image, Lens, Preset, Projection, RenderSize, Shading, View};

const SIZE: u32 = 128;

/// Two unit spheres side by side at x = -1.2 and 1.2, about 10 pixels apart, the second
/// shown or not, each marked as `marked` says.
fn spheres(marked: [bool; 2], second_shown: bool) -> [GpuMesh; 2] {
    let r = lavapipe();
    let mesh = scene::icosphere(3);
    let at = |x: f32| Similarity {
        translation: Vec3::new(x, 0.0, 0.0),
        ..Similarity::IDENTITY
    };
    [
        upload(r, &mesh).placed(at(-1.2), true).marked(marked[0]),
        upload(r, &mesh)
            .placed(at(1.2), second_shown)
            .marked(marked[1]),
    ]
}

/// Front orthographic, -2.6..2.6 both ways, in `shading`, the outline on or off.
fn view(shading: Shading<'_>, outline: bool) -> View<'_> {
    let camera = Camera {
        distance: 2.6,
        lens: Lens::from_fov(std::f32::consts::FRAC_PI_2, Fit::Vertical),
        projection: Projection::Orthographic,
        ..Camera::preset(Preset::Front)
    };
    View {
        outline,
        ..View::new(camera, shading)
    }
}

fn picture(meshes: &[&GpuMesh], view: &View) -> Image {
    let size = RenderSize::square(SIZE);
    lavapipe().snapshot(meshes, view, size).expect("renders")
}

fn pixel(image: &Image, x: i64, y: i64) -> Option<[u8; 4]> {
    let inside =
        (0..i64::from(image.width)).contains(&x) && (0..i64::from(image.height)).contains(&y);
    let i = 4 * (y * i64::from(image.width) + x) as usize;
    inside.then(|| [0, 1, 2, 3].map(|k| image.rgba[i + k]))
}

#[test]
fn only_the_marked_part_is_outlined_just_outside_its_own_pixels() {
    for (look, shading) in [("clay", Shading::Clay), ("normals", Shading::Normals)] {
        let plain = picture(&spheres([false; 2], true).each_ref(), &view(shading, true));
        for (marked, left) in [([true, false], true), ([false, true], false)] {
            let drawn = picture(&spheres(marked, true).each_ref(), &view(shading, true));
            let (mut changed, mut orange) = (0, 0);
            for y in 0..i64::from(SIZE) {
                for x in 0..i64::from(SIZE) {
                    let (before, after) = (pixel(&plain, x, y), pixel(&drawn, x, y));
                    if before == after {
                        continue;
                    }
                    changed += 1;
                    // Only background changes, beside the marked sphere's half of the picture.
                    assert_eq!(before, Some(BACKGROUND), "{look}: ({x}, {y}) was a part");
                    assert_eq!(x < 64, left, "{look}: ({x}, {y}) is beside the other part");
                    let near = (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)));
                    let beside = near
                        .filter(|(dx, dy)| dx * dx + dy * dy <= 4)
                        .any(|(dx, dy)| {
                            pixel(&plain, x + dx, y + dy).is_some_and(|p| p != BACKGROUND)
                        });
                    assert!(beside, "{look}: ({x}, {y}) is within 2 pixels of the part");
                    let a = after.unwrap_or_default();
                    let full = [255, 170, 64];
                    orange += usize::from((0..3).all(|k| a[k].abs_diff(full[k]) <= 1));
                }
            }
            // A sphere about 38 pixels across: a line a little over 2 pixels wide around it.
            assert!(changed > 200, "{look}: {changed} pixels outlined");
            assert!(orange > 100, "{look}: {orange} pixels in the full orange");
        }
    }
}

#[test]
fn with_the_outline_off_or_one_part_drawn_nothing_changes() {
    let clay = |outline| view(Shading::Clay, outline);
    let plain = picture(&spheres([false; 2], true).each_ref(), &clay(false));
    let off = picture(&spheres([true, false], true).each_ref(), &clay(false));
    assert_eq!(
        diff(&plain, &off),
        (0, 0),
        "the outline off draws as before"
    );
    let [one, _] = spheres([false; 2], true);
    let alone = picture(&[&one], &clay(true));
    let [marked, _] = spheres([true, false], true);
    let marked_alone = picture(&[&marked], &clay(true));
    assert_eq!(
        diff(&alone, &marked_alone),
        (0, 0),
        "one part draws as before"
    );
    let hidden = picture(&spheres([true, false], false).each_ref(), &clay(true));
    assert_eq!(
        diff(&alone, &hidden),
        (0, 0),
        "a hidden second part does not count"
    );
}
