//! The cube bins weld (gs-geo) and the mirror map search near points with (`math::Bins`): a
//! differential check, every box of cubes gives the points a scan of every point finds there.

use gs_core::math::Bins;

use crate::common::mix;

/// A cube from -3 to 3 along each axis, so cubes hold several points and some none.
fn cube(h: u64) -> [i64; 3] {
    [0, 21, 42].map(|s| i64::from(((h >> s) % 7) as u8) - 3)
}

#[test]
fn bins_give_the_points_in_a_box_of_cubes_that_a_scan_of_every_point_finds() {
    let cubes: Vec<[i64; 3]> = (0..300).map(|i| cube(mix(i))).collect();
    let bins = Bins::new(cubes.len(), |i| cubes[i]);
    for q in 0..200 {
        let (a, b) = (cube(mix(1000 + q)), cube(mix(2000 + q)));
        let lo = [0, 1, 2].map(|k| a[k].min(b[k]));
        let hi = [0, 1, 2].map(|k| a[k].max(b[k]));
        let mut got: Vec<u32> = bins.within(lo, hi).collect();
        got.sort_unstable();
        let inside = |c: &[i64; 3]| (0..3).all(|k| (lo[k]..=hi[k]).contains(&c[k]));
        let want: Vec<u32> = (0..cubes.len() as u32)
            .filter(|&i| inside(&cubes[i as usize]))
            .collect();
        assert_eq!(got, want, "cubes {lo:?} to {hi:?}");
    }
}
