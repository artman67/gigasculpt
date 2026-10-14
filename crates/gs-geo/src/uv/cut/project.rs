//! A chart on its mean plane (UV2.1): what the check reads before UV3, and where UV3 starts.

use gs_protocol::face_triangles;

use crate::math::{cross, dot, sub};
use crate::uv::{Chart, Surface};

impl Chart {
    /// Its wedges on its mean plane: along its area-weighted mean normal, with the longer
    /// principal axis of its wedges on u, wound face up and centred on 0.
    pub fn project(&self, surface: Surface) -> Vec<[f32; 2]> {
        let at = |w: usize| surface.points[self.points[w] as usize].map(f64::from);
        let n = self.normal(surface);
        let axis = (0..3).fold(
            0,
            |best, i| if n[i].abs() < n[best].abs() { i } else { best },
        );
        let mut least = [0.0; 3];
        least[axis] = 1.0;
        let e1 = unit(cross(n, least));
        let e2 = cross(n, e1);
        let flat: Vec<[f64; 2]> = (0..self.points.len())
            .map(|w| [dot(at(w), e1), dot(at(w), e2)])
            .collect();
        upright(&flat)
    }

    /// Its area-weighted mean unit normal, up when it has no area.
    fn normal(&self, surface: Surface) -> [f64; 3] {
        let mut sum = [0.0; 3];
        for &f in &self.faces {
            let face = surface.faces[f as usize];
            let p = |k: usize| surface.points[face[k] as usize];
            for t in face_triangles(face, p) {
                let q = t.map(|k| p(k).map(f64::from));
                let c = cross(sub(q[1], q[0]), sub(q[2], q[0]));
                sum = std::array::from_fn(|i| sum[i] + c[i]);
            }
        }
        if dot(sum, sum) > 0.0 {
            unit(sum)
        } else {
            [0.0, 1.0, 0.0]
        }
    }
}

/// `flat` centred on 0 and turned so the longer principal axis of its points lies on u: the
/// projection's last step, and the flattener's.
pub(crate) fn upright(flat: &[[f64; 2]]) -> Vec<[f32; 2]> {
    let count = flat.len().max(1) as f64;
    let mean = [0, 1].map(|i| flat.iter().map(|p| p[i]).sum::<f64>() / count);
    let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
    for p in flat {
        let (x, y) = (p[0] - mean[0], p[1] - mean[1]);
        (a, b, c) = (a + x * x, b + x * y, c + y * y);
    }
    let half = (a - c) / 2.0;
    let large = f64::midpoint(a, c) + (half * half + b * b).sqrt();
    let major = if a >= c {
        [large - c, b]
    } else {
        [b, large - a]
    };
    let length = (major[0] * major[0] + major[1] * major[1]).sqrt();
    let u = if length > 0.0 {
        major.map(|x| x / length)
    } else {
        [1.0, 0.0]
    };
    (flat.iter())
        .map(|p| {
            let (x, y) = (p[0] - mean[0], p[1] - mean[1]);
            [(x * u[0] + y * u[1]) as f32, (y * u[0] - x * u[1]) as f32]
        })
        .collect()
}

/// `v` at length 1.
fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|x| x / length)
}
