//! Live Boolean's stand-ins (decision 0057, section 9, as amended for LB7): a tinted mesh drawn
//! over a copy of the same faces shows its paint times the tint there and leaves every other
//! pixel as it was, its mask and the active part's outline still showing; a ghosted mesh draws
//! an outline only, just outside its silhouette, through whatever is in front of it, in its
//! color or, while it is the active part in a view with the outline on, the outline's orange.

use crate::common::{BACKGROUND, front_ortho, lavapipe, luma, snap, upload};
use crate::scene;
use gs_core::Vec3;
use gs_core::transform::Similarity;
use gs_protocol::{AttrDomain, AttrValues, Attribute, MASK, PlainMesh};
use gs_render::{GpuMesh, Image, Shading, View};

const SIZE: u32 = 128;
const RED: [u8; 3] = [230, 40, 40];
const BLUE: [u8; 3] = [40, 90, 240];
/// The active part's outline, #FFAA40.
const ORANGE: [u8; 4] = [255, 170, 64, 255];

fn picture(meshes: &[&GpuMesh], view: &View) -> Image {
    snap(lavapipe(), meshes, view, SIZE)
}

fn pixel(image: &Image, x: i64, y: i64) -> Option<[u8; 4]> {
    let inside = (0..i64::from(SIZE)).contains(&x) && (0..i64::from(SIZE)).contains(&y);
    let i = || 4 * (y * i64::from(image.width) + x) as usize;
    inside.then(|| [0, 1, 2, 3].map(|k| image.rgba[i() + k]))
}

/// Every pixel, as (x, y).
fn pixels() -> impl Iterator<Item = (i64, i64)> {
    (0..i64::from(SIZE)).flat_map(|y| (0..i64::from(SIZE)).map(move |x| (x, y)))
}

/// The unit icosphere's faces right of x = 0.25, on all of its points: the walls a cut of the
/// sphere would have there.
fn right_faces(sphere: &PlainMesh) -> PlainMesh {
    let faces = (sphere.faces.iter().copied())
        .filter(|f| {
            f[..3]
                .iter()
                .all(|&v| sphere.positions[v as usize][0] > 0.25)
        })
        .collect();
    scene::plain(sphere.positions.clone(), faces)
}

fn red(p: [u8; 4]) -> bool {
    i32::from(p[0]) > i32::from(p[1]) + 20 && i32::from(p[0]) > i32::from(p[2]) + 20
}

/// Whether `a` and `b` differ by more than a little: occlusion, worked out from the depth,
/// sees the walls' nudge as a step of a level or two beside them.
fn differ(a: Option<[u8; 4]>, b: Option<[u8; 4]>) -> bool {
    let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
    a.iter().zip(b).any(|(s, t)| s.abs_diff(t) > 4)
}

#[test]
fn a_tinted_copy_shows_its_tint_over_its_own_faces_and_leaves_the_rest() {
    let (r, sphere) = (lavapipe(), scene::icosphere(3));
    let body = upload(r, &sphere);
    let walls = upload(r, &right_faces(&sphere)).tinted(Some(RED));
    let view = front_ortho(Shading::Clay);
    let (plain, tinted) = (picture(&[&body], &view), picture(&[&body, &walls], &view));
    let mut changed = 0;
    for (x, y) in pixels() {
        let (before, after) = (pixel(&plain, x, y), pixel(&tinted, x, y));
        if !differ(before, after) {
            continue;
        }
        changed += 1;
        assert_ne!(
            before,
            Some(BACKGROUND),
            "({x}, {y}): only the body's pixels change"
        );
        // x = 0.25 falls about column 78.
        assert!(x >= 72, "({x}, {y}): only over the walls' faces");
        assert!(
            red(after.unwrap_or_default()),
            "({x}, {y}): {before:?} to {after:?} takes the tint"
        );
    }
    assert!(changed > 400, "the walls show: {changed} pixels");
}

/// A small sphere inside the unit one, so it hides wholly behind its surface.
fn hidden_sphere(r: &gs_render::Renderer) -> GpuMesh {
    let at = Similarity {
        translation: Vec3::new(0.3, 0.1, -0.2),
        scale: 0.5,
        ..Similarity::IDENTITY
    };
    upload(r, &scene::icosphere(2)).placed(at, true)
}

#[test]
fn a_ghost_draws_only_its_outline_through_what_is_in_front_of_it() {
    let r = lavapipe();
    let body = upload(r, &scene::icosphere(3));
    let view = View {
        outline: true,
        ..front_ortho(Shading::Clay)
    };
    let alone = picture(&[&hidden_sphere(r)], &view);
    let covered = |x: i64, y: i64| pixel(&alone, x, y).is_some_and(|p| p != BACKGROUND);
    let plain = picture(&[&body], &view);
    for (marked, color) in [(false, [BLUE[0], BLUE[1], BLUE[2], 255]), (true, ORANGE)] {
        let ghost = hidden_sphere(r).ghosted(Some(BLUE)).marked(marked);
        let ghosted = picture(&[&body, &ghost], &view);
        let mut changed = 0;
        for (x, y) in pixels() {
            let after = pixel(&ghosted, x, y);
            if pixel(&plain, x, y) == after {
                continue;
            }
            changed += 1;
            assert_eq!(after, Some(color), "({x}, {y}): marked {marked}");
            assert!(
                !covered(x, y),
                "({x}, {y}): never on the ghost's own pixels"
            );
            let near = (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)));
            let beside = near
                .filter(|(dx, dy)| dx * dx + dy * dy <= 4)
                .any(|(dx, dy)| covered(x + dx, y + dy));
            assert!(beside, "({x}, {y}): within 2 pixels of the ghost");
        }
        // Every pixel just beside the ghost's silhouette, on the body, is outlined.
        for (x, y) in pixels().filter(|&(x, y)| !covered(x, y)) {
            if [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .any(|(dx, dy)| covered(x + dx, y + dy))
            {
                assert_eq!(pixel(&ghosted, x, y), Some(color), "({x}, {y}) is outlined");
            }
        }
        assert!(
            changed > 60,
            "marked {marked}: an outline of {changed} pixels"
        );
    }
}

#[test]
fn a_marked_result_keeps_its_outline_with_its_walls_drawn_over_it() {
    let (r, sphere) = (lavapipe(), scene::icosphere(3));
    let left = Similarity {
        translation: Vec3::new(-1.6, 0.0, 0.0),
        scale: 0.4,
        ..Similarity::IDENTITY
    };
    let other = upload(r, &sphere).placed(left, true);
    let body = upload(r, &sphere).marked(true);
    let walls = upload(r, &right_faces(&sphere)).tinted(Some(RED));
    let view = View {
        outline: true,
        ..front_ortho(Shading::Clay)
    };
    let plain = picture(&[&body, &other], &view);
    let drawn = picture(&[&body, &walls.marked(true), &other], &view);
    let orange = pixels().filter(|&(x, y)| pixel(&plain, x, y) == Some(ORANGE));
    assert!(orange.count() > 100, "the body is outlined");
    for (x, y) in pixels() {
        let (before, after) = (pixel(&plain, x, y), pixel(&drawn, x, y));
        if before == Some(ORANGE) {
            assert_eq!(after, before, "({x}, {y}): the outline stays");
        } else if differ(before, after) {
            let after = after.unwrap_or_default();
            assert!(red(after), "({x}, {y}): only the walls change");
        }
    }
}

#[test]
fn the_mask_darkens_a_tinted_copy() {
    let (r, sphere) = (lavapipe(), scene::icosphere(3));
    let body = upload(r, &sphere);
    let mut masked = right_faces(&sphere);
    let full = Attribute {
        domain: AttrDomain::Vertex,
        values: AttrValues::F32(vec![1.0; masked.positions.len()]),
    };
    masked.attributes.insert(MASK.into(), full);
    let view = front_ortho(Shading::Clay);
    let walls = upload(r, &right_faces(&sphere)).tinted(Some(RED));
    let clear = picture(&[&body, &walls], &view);
    let walls = upload(r, &masked).tinted(Some(RED));
    let dark = picture(&[&body, &walls], &view);
    let mut darker = 0;
    for (x, y) in pixels() {
        let (a, b) = (pixel(&clear, x, y), pixel(&dark, x, y));
        if a != b {
            assert!(
                luma(b.unwrap_or_default()) < luma(a.unwrap_or_default()),
                "({x}, {y})"
            );
            darker += 1;
        }
    }
    assert!(darker > 400, "the mask shows on the walls: {darker} pixels");
}
