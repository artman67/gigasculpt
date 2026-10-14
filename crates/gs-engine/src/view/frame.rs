//! What framing sees of the scene: the boxes around every part, one part, the masked or the
//! unmasked points, and the surface under a ray. One place, so framing skips what is hidden
//! here: hidden faces never count (decision 0015), and a part that does not show (decision
//! 0028) counts only when it is named. Everything is
//! in the scene: a part's node boxes are mapped by its transform (their corners, so a turned
//! box grows a little, which framing tolerates), its points mapped, and rays mapped into it
//! (decision 0029, section 4).

use gs_core::camera::{Camera, Preset};
use gs_core::visibility::Cover;
use gs_core::{Aabb, Vec3};

use crate::document::{Document, Part};

/// How far, in scene units, framing reaches around a point: a box of no size (a part scaled to
/// 0, or made with every point in one place) is framed as the cube this far around it, since a
/// camera needs some distance from what it looks at (review 0008, m3). Reference Views frame a
/// sphere of no size as one of this radius.
pub(crate) const POINT_REACH: f32 = 0.1;

/// The eight corners of a box; none for an empty one. A box of no size gives the corners of
/// the cube [`POINT_REACH`] around it.
pub(crate) fn box_corners(b: Aabb) -> Vec<[f32; 3]> {
    if b == Aabb::EMPTY {
        return Vec::new();
    }
    let b = if b.lo == b.hi {
        let reach = Vec3::new(POINT_REACH, POINT_REACH, POINT_REACH);
        Aabb {
            lo: b.lo - reach,
            hi: b.hi + reach,
        }
    } else {
        b
    };
    let ([x0, y0, z0], [x1, y1, z1]) = (b.lo.to_array(), b.hi.to_array());
    let xs = [[x0, y0], [x1, y0], [x0, y1], [x1, y1]];
    (xs.iter())
        .flat_map(|&[x, y]| [[x, y, z0], [x, y, z1]])
        .collect()
}

/// The box around the faces that show of the parts `keep` picks.
fn parts_box(doc: &Document, keep: impl Fn(&str, &Part) -> bool) -> Aabb {
    (doc.parts())
        .filter(|(name, part)| keep(name, part))
        .map(|(_, part)| shown_box(part))
        .fold(Aabb::EMPTY, Aabb::union)
}

/// The box around `part`'s faces that show, in the scene: the corners of those faces are its
/// points that show (decision 0015). A node with no hidden point gives its own box, one with
/// every point hidden nothing, and one with some hidden the box of the rest.
fn shown_box(part: &Part) -> Aabb {
    let (store, t) = (part.store(), part.transform());
    let node_box = |n: u32| match store.node_state(n).points {
        Cover::None => t.bounds(store.bounds(n)),
        Cover::All => Aabb::EMPTY,
        Cover::Some => {
            let node = store.node(n);
            let mut b = Aabb::EMPTY;
            for (l, &p) in node.pos.iter().enumerate() {
                if !p.is_dead() && !node.hidden(l) {
                    b.grow(t.point(p));
                }
            }
            b
        }
    };
    (0..store.node_count() as u32)
        .map(node_box)
        .fold(Aabb::EMPTY, Aabb::union)
}

/// The box around every part that shows.
pub(crate) fn scene_box(doc: &Document) -> Aabb {
    let showing = doc.showing();
    parts_box(doc, |_, part| part.shown(showing))
}

/// The corners of the box around every part: what framing keeps in the picture.
pub(crate) fn corners(doc: &Document) -> Vec<[f32; 3]> {
    box_corners(scene_box(doc))
}

/// The centre of every part, if there is one.
pub(crate) fn center(doc: &Document) -> Option<[f32; 3]> {
    let b = scene_box(doc);
    (b != Aabb::EMPTY).then(|| ((b.lo + b.hi) * 0.5).to_array())
}

/// A preset view framing every part of `doc` for an image `aspect` wide per high.
pub(crate) fn fitted(preset: Preset, doc: &Document, aspect: f32) -> Camera {
    Camera::fitted(preset, &corners(doc), aspect)
}

/// The box around part `name`.
pub(crate) fn part_box(doc: &Document, name: &str) -> Aabb {
    parts_box(doc, |n, _| n == name)
}

/// The box around the points that show of every part that shows whose mask `inside` accepts
/// (0 where a part has no mask).
pub(crate) fn mask_box(doc: &Document, inside: impl Fn(f32) -> bool) -> Aabb {
    let mut b = Aabb::EMPTY;
    for (_, part) in doc.shown() {
        let (store, t) = (part.store(), part.transform());
        let mask = store.attr_id(gs_protocol::MASK);
        for n in 0..store.node_count() as u32 {
            let node = store.node(n);
            let values = mask.and_then(|a| match node.attr(a) {
                Some(gs_core::AttrVec::F32(v)) => Some(v),
                _ => None,
            });
            for (i, &p) in node.pos.iter().enumerate() {
                let m = values.and_then(|v| v.get(i)).copied().unwrap_or(0.0);
                if !p.is_dead() && !node.hidden(i) && inside(m) {
                    b.grow(t.point(p));
                }
            }
        }
    }
    b
}

/// The nearest surface point of any part that shows along a ray, as `pick.ray` finds it
/// (`pick::first_hit`).
pub(crate) fn hit(doc: &Document, (origin, dir): (Vec3, Vec3)) -> Option<[f32; 3]> {
    let parts: Vec<_> = (doc.shown()).map(|(n, p)| (n.as_str(), p)).collect();
    let (_, hit, _, part) = crate::pick::first_hit(&parts, origin, dir)?;
    Some(part.transform().point(hit.pos).to_array())
}
