//! Lit mode's direct lights (decision 0023): distant discs of uniform radiance fixed to the
//! view, the key light taken out of an environment, and the settings of the eight lights of
//! `light.palette`.
//!
//! - Diffuse: the exact irradiance of a uniform spherical cap, including where it dips below
//!   the horizon (Snyder's form factor as Lagarde and de Rousiers give it, "Moving Frostbite to
//!   PBR", 2014, section 4.7.2).
//! - Specular: Karis 2013's representative point (the point of the disc nearest the
//!   reflection ray, his equation 11) with his normalization `(a / a')^2`,
//!   `a' = saturate(a + sin s / 3)` (equations 10 and 14).
//!
//! Intensities are in the environment's metering: an environment is scaled so a white card
//! facing the camera reflects 1 (its irradiance is pi), and a light of intensity 1 lights a
//! white card facing it the same way.

use super::Rgb;
use super::cube::{Cube, Level, direction, texel_solid_angle, wrap};
use crate::math::{V3, dot, normalize_or, scale, sub};
use std::f32::consts::PI;

/// Radiance above which part of an environment is a source rather than surroundings: the
/// radiance of a uniform environment that lights the camera-facing card to reflect 1.
pub const THRESHOLD: f32 = 1.0;

/// Where lights 2 to 8 shine from unless set: the default studio's key soft box, up and left
/// of the camera (view space).
pub const DEFAULT_DIRECTION: V3 = [-0.45, 0.55, 0.70];

/// The angular radius of lights 2 to 8 unless set: the sun's, in degrees.
pub const DEFAULT_SIZE_DEG: f32 = 0.27;

/// Relative luminance of linear Rec. 709 light.
pub fn luminance(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// A distant disc of uniform radiance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DiscLight {
    /// Toward the disc's center, unit, in view space (x right, y up, z toward the viewer).
    pub direction: V3,
    /// Angular radius in radians, above 0.
    pub radius: f32,
    /// Irradiance on a surface facing the disc, linear RGB.
    pub illuminance: Rgb,
}

/// Irradiance from a uniform cap of radiance 1 whose center lies at cosine `cos_t` from the
/// normal, the cap's angular radius `s` given as `sin^2 s`: `pi sin^2 s cos t` while the whole
/// cap is above the horizon, Snyder's closed form while it straddles it, 0 below (Lagarde and
/// de Rousiers 2014, listing 7).
pub fn cap_irradiance(cos_t: f32, sin2_s: f32) -> f32 {
    if cos_t * cos_t > sin2_s {
        return PI * sin2_s * cos_t.max(0.0);
    }
    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
    let x = (1.0 / sin2_s - 1.0).max(0.0).sqrt();
    let y = (-x * cos_t / sin_t.max(1e-12)).clamp(-1.0, 1.0);
    let sin_t_root_y = sin_t * (1.0 - y * y).max(0.0).sqrt();
    let e = (cos_t * libm::acosf(y) - x * sin_t_root_y) * sin2_s
        + libm::atanf(sin_t_root_y / x.max(1e-12));
    e.max(0.0)
}

impl DiscLight {
    /// `sin s` and `sin^2 s` of the angular radius.
    pub fn sin_radius(&self) -> (f32, f32) {
        let s = libm::sinf(self.radius.clamp(1e-6, PI / 2.0));
        (s, s * s)
    }

    /// Irradiance on a surface with unit normal `n`.
    pub fn irradiance(&self, n: V3) -> Rgb {
        let (_, sin2) = self.sin_radius();
        let k = cap_irradiance(dot(n, self.direction), sin2) / (PI * sin2);
        self.illuminance.map(|e| e * k)
    }

    /// The direction from which the disc lights a specular lobe reflecting along unit `r`:
    /// the point of the disc nearest the ray (Karis 2013, equation 11, with the disc at
    /// distance 1 and of radius `sin s`).
    pub fn representative(&self, r: V3) -> V3 {
        let (sin, _) = self.sin_radius();
        let l = self.direction;
        // From the disc's center to the nearest point of the ray.
        let to_ray = sub(scale(r, dot(l, r)), l);
        let len = dot(to_ray, to_ray).sqrt();
        let t = if len > 0.0 { (sin / len).min(1.0) } else { 0.0 };
        let closest = std::array::from_fn(|k| l[k] + to_ray[k] * t);
        normalize_or(closest, l)
    }

    /// Karis 2013's energy normalization of a GGX lobe of `alpha` lit through the
    /// representative point: `(a / a')^2`, `a' = saturate(a + sin s / 3)`.
    pub fn normalization(&self, alpha: f32) -> f32 {
        let (sin, _) = self.sin_radius();
        let wider = (alpha + sin / 3.0).min(1.0);
        let k = alpha / wider;
        k * k
    }
}

/// One of the eight lights as set: each value left unset takes the light's default (for light
/// 1, what the environment's extraction found; for lights 2 to 8, white, intensity 1, from
/// [`DEFAULT_DIRECTION`], [`DEFAULT_SIZE_DEG`] wide).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightSetting {
    pub on: bool,
    /// Linear RGB, scaled to luminance 1 when used.
    pub color: Option<Rgb>,
    /// 1 lights a white card facing the light to reflect 1.
    pub intensity: Option<f32>,
    /// Toward the light, view space; normalized when used.
    pub direction: Option<V3>,
    /// Angular radius in degrees.
    pub size_deg: Option<f32>,
}

/// The lights of Lit mode: Ambient (the environment's light scaled) and lights 1 to 8.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lights {
    pub ambient: f32,
    pub each: [LightSetting; 8],
}

impl Lights {
    /// Ambient 1, light 1 (the environment's key) on with its found values, lights 2 to 8 off.
    pub const DEFAULT: Lights = {
        let off = LightSetting {
            on: false,
            color: None,
            intensity: None,
            direction: None,
            size_deg: None,
        };
        let mut each = [off; 8];
        each[0].on = true;
        Lights { ambient: 1.0, each }
    };
}

impl Default for Lights {
    fn default() -> Self {
        Lights::DEFAULT
    }
}

/// A light's values: color at luminance 1, intensity, unit direction, radius in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightValues {
    pub color: Rgb,
    pub intensity: f32,
    pub direction: V3,
    pub size_deg: f32,
}

impl LightValues {
    /// The values of `light` (light 1 is the environment's `key`).
    pub fn of(setting: &LightSetting, key: Option<&DiscLight>, first: bool) -> LightValues {
        let base = match (first, key) {
            (true, Some(k)) => {
                let lum = luminance(k.illuminance);
                LightValues {
                    color: if lum > 0.0 {
                        k.illuminance.map(|c| c / lum)
                    } else {
                        [1.0; 3]
                    },
                    intensity: lum / PI,
                    direction: k.direction,
                    size_deg: k.radius.to_degrees(),
                }
            }
            (true, None) => LightValues {
                intensity: 0.0,
                ..LightValues::default_light()
            },
            (false, _) => LightValues::default_light(),
        };
        let color = setting.color.map_or(base.color, |c| {
            let lum = luminance(c);
            if lum > 0.0 {
                c.map(|v| v / lum)
            } else {
                [0.0; 3]
            }
        });
        LightValues {
            color,
            intensity: setting.intensity.unwrap_or(base.intensity).max(0.0),
            direction: setting
                .direction
                .map_or(base.direction, |d| normalize_or(d, base.direction)),
            size_deg: setting.size_deg.unwrap_or(base.size_deg).clamp(0.01, 90.0),
        }
    }

    /// Lights 2 to 8 unless set.
    fn default_light() -> LightValues {
        LightValues {
            color: [1.0; 3],
            intensity: 1.0,
            direction: normalize_or(DEFAULT_DIRECTION, [0.0, 0.0, 1.0]),
            size_deg: DEFAULT_SIZE_DEG,
        }
    }

    /// The light as a disc.
    pub fn disc(&self) -> DiscLight {
        DiscLight {
            direction: self.direction,
            radius: self.size_deg.to_radians(),
            illuminance: self.color.map(|c| c * self.intensity * PI),
        }
    }
}

impl Lights {
    /// The discs of the lights that are on and give light, in order.
    pub fn discs(&self, key: Option<&DiscLight>) -> Vec<DiscLight> {
        (self.each.iter().enumerate())
            .filter(|(_, s)| s.on)
            .map(|(i, s)| LightValues::of(s, key, i == 0).disc())
            .filter(|d| luminance(d.illuminance) > 0.0)
            .collect()
    }
}

/// The key light taken out of `radiance` (decision 0023): of the connected regions of level 0
/// brighter than [`THRESHOLD`], the one that adds the most irradiance to a card facing the
/// camera. Its texels keep luminance 1 (their color kept) and the rest of their light becomes
/// a disc matched to it by its moments: direction the mean, angular radius from the mean
/// cosine (a uniform cap of radius `s` has a mean cosine of `(1 + cos s) / 2`), illuminance
/// the excess's own toward that direction. The environment plus the disc hold the light of
/// the original. Returns the rest of the environment (new mip levels, no metering) and the
/// disc, or `None` when nothing is brighter than the threshold.
pub fn extract(radiance: &Cube) -> Option<(Cube, DiscLight)> {
    let level = &radiance.levels[0];
    let texels = Texels {
        level,
        size: level[0].size,
    };
    let region = texels.regions();
    // Each region's irradiance on the camera-facing card; the largest, first in texel order.
    let mut card = vec![0.0f64; region.len()];
    for (t, &root) in region.iter().enumerate().filter(|(_, r)| **r != NONE) {
        let (d, omega, c) = texels.at(t);
        card[root] += f64::from(luminance(c) * omega * d[2].max(0.0));
    }
    let (chosen, best) =
        (card.iter().enumerate()).fold(
            (NONE, 0.0f64),
            |(bi, bv), (i, &v)| if v > bv { (i, v) } else { (bi, bv) },
        );
    if chosen == NONE || best <= 0.0 {
        return None;
    }
    // The excess of the chosen region, and the rest of the environment.
    let mut base: Level = level.clone();
    let (mut moment, mut total) = ([0.0f64; 3], 0.0f64);
    let mut excess: Vec<(V3, f32, Rgb)> = Vec::new();
    for t in (0..region.len()).filter(|&t| region[t] == chosen) {
        let (d, omega, c) = texels.at(t);
        let kept = c.map(|v| v / luminance(c));
        let over: Rgb = std::array::from_fn(|k| c[k] - kept[k]);
        let w = f64::from(luminance(over) * omega);
        for k in 0..3 {
            moment[k] += w * f64::from(d[k]);
        }
        total += w;
        excess.push((d, omega, over));
        let face_texels = texels.size * texels.size;
        base[t / face_texels].texels[t % face_texels] = kept;
    }
    let toward = normalize_or(moment.map(|m| m as f32), [0.0, 0.0, 1.0]);
    let mean_cos = (dot(moment, moment).sqrt() / total.max(1e-30)).clamp(-1.0, 1.0);
    let cos_s = (2.0 * mean_cos - 1.0).clamp(-1.0, 1.0) as f32;
    let illuminance = excess.iter().fold([0.0f32; 3], |sum, (d, omega, over)| {
        let k = dot(*d, toward).max(0.0) * omega;
        std::array::from_fn(|c| sum[c] + over[c] * k)
    });
    let disc = DiscLight {
        direction: toward,
        radius: libm::acosf(cos_s).max(1e-4),
        illuminance,
    };
    Some((Cube::from_base(base), disc))
}

/// No region: a texel at or below the threshold.
const NONE: usize = usize::MAX;

/// The texels of one cube level, numbered face by face, row by row.
struct Texels<'a> {
    level: &'a Level,
    size: usize,
}

impl Texels<'_> {
    /// Direction, solid angle and radiance of texel `t`.
    fn at(&self, t: usize) -> (V3, f32, Rgb) {
        let (f, rest) = (t / (self.size * self.size), t % (self.size * self.size));
        let (i, j) = (rest % self.size, rest / self.size);
        let n = self.size as f32;
        let d = direction(f, (i as f32 + 0.5) / n, (j as f32 + 0.5) / n);
        (
            d,
            texel_solid_angle(self.size, i, j),
            self.level[f].texels[rest],
        )
    }

    /// For every texel brighter than the threshold, the first texel of its connected region
    /// (connected across face edges); [`NONE`] for the others. Union-find in texel order.
    fn regions(&self) -> Vec<usize> {
        let size = self.size;
        let count = 6 * size * size;
        let bright = |t: usize| luminance(self.at(t).2) > THRESHOLD;
        let mut parent: Vec<usize> = (0..count).collect();
        let find = |parent: &mut Vec<usize>, mut x: usize| {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        };
        let cell = |k: usize| i32::try_from(k).unwrap_or(0);
        for t in (0..count).filter(|&t| bright(t)) {
            let (f, rest) = (t / (size * size), t % (size * size));
            let (i, j) = (cell(rest % size), cell(rest / size));
            // All four sides: across a face edge the neighbor may lie on any side.
            for (di, dj) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                let (g, x, y) = wrap(size, f, i + di, j + dj);
                let b = g * size * size + y * size + x;
                if bright(b) {
                    let (ra, rb) = (find(&mut parent, t), find(&mut parent, b));
                    parent[ra.max(rb)] = ra.min(rb);
                }
            }
        }
        (0..count)
            .map(|t| {
                if bright(t) {
                    find(&mut parent, t)
                } else {
                    NONE
                }
            })
            .collect()
    }
}
