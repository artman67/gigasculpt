//! Whether two faces run through the same corners in the same cycle (`math::same_cycle`), which
//! the mirror map (gs-core) and weld's duplicate faces (gs-geo) both ask: a metamorphic check,
//! every turn of a face is the same cycle, and its reversal and its neighbours are not.

use gs_core::NO;
use gs_core::math::{arity, same_cycle};

/// `f` started at its corner `r`.
fn turned(f: [u32; 4], r: usize) -> [u32; 4] {
    let k = arity(f);
    let mut g = f;
    for (i, c) in g.iter_mut().enumerate().take(k) {
        *c = f[(i + r) % k];
    }
    g
}

/// `f` with its winding reversed.
fn reversed(f: [u32; 4]) -> [u32; 4] {
    let mut g = f;
    g[..arity(f)].reverse();
    g
}

#[test]
fn a_face_is_its_own_cycle_from_any_corner_but_not_reversed() {
    for f in [[3, 7, 1, 9], [4, 2, 8, NO]] {
        for r in 0..arity(f) {
            assert!(same_cycle(f, turned(f, r)), "{f:?} from corner {r}");
            assert!(
                !same_cycle(f, turned(reversed(f), r)),
                "{f:?} reversed, from {r}"
            );
        }
        let mut other = f;
        other[1] = 5;
        assert!(!same_cycle(f, other), "{f:?} against {other:?}");
    }
    let (triangle, quad) = ([4, 2, 8, NO], [4, 2, 8, 6]);
    assert!(
        !same_cycle(triangle, quad),
        "a triangle is not the quad it begins"
    );
    assert!(
        !same_cycle(quad, triangle),
        "a quad is not the triangle it begins with"
    );
}
