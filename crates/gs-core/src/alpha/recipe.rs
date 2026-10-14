//! Alpha recipes: our own pictures, written as data (`assets/alphas/*.toml`) and drawn by this
//! generator (decision 0018). A recipe is a list of layers, each a radial profile, a shape from
//! a distance function, or tileable noise; each layer after the first combines with the ones
//! before it. Drawing uses plain arithmetic in a fixed order, integer hashing for noise and no
//! platform maths, so the same recipe always gives the same pixels at any thread count.

use serde::Deserialize;

use super::noise;
use super::picture::{MOST, Picture, quantize};
use crate::curve::Curve;

/// A recipe file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// What the user reads, for example `Soft Round`.
    pub name: String,
    /// The catalog item it builds, for example `alpha.starter-tips`.
    pub catalog_id: String,
    /// Width and height in pixels.
    #[serde(default = "full")]
    pub size: u32,
    /// The layers, first to last.
    #[serde(rename = "layer")]
    pub layers: Vec<Layer>,
}

fn full() -> u32 {
    1024
}

/// How a layer combines with the layers before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    #[default]
    Multiply,
    Add,
    Max,
    Min,
}

/// One layer. Distances are in half-widths of the picture: its center is 0 and the middle of
/// each edge is 1, so a circle of radius 1 touches the edges.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum Layer {
    /// A curve over the distance from the center: 0 the center, 1 the edge (and beyond).
    Profile {
        points: Curve,
        #[serde(default)]
        op: Op,
    },
    /// A disc: 1 inside `radius`, 0 outside, with a ramp `edge` wide across the rim.
    Circle {
        #[serde(default = "one")]
        radius: f32,
        #[serde(default = "thin")]
        edge: f32,
        #[serde(default)]
        op: Op,
    },
    /// A square `half` half-widths from its center to each side, corners rounded by `round`.
    Box {
        half: f32,
        #[serde(default)]
        round: f32,
        #[serde(default = "thin")]
        edge: f32,
        #[serde(default)]
        op: Op,
    },
    /// A band `width` wide centered at distance `radius`.
    Ring {
        radius: f32,
        width: f32,
        #[serde(default = "thin")]
        edge: f32,
        #[serde(default)]
        op: Op,
    },
    /// Tileable value noise with `scale` cells across, `octaves` each twice as fine: the layer
    /// is `1 - amount * (1 - noise)`, so `amount` 0 is plain white.
    Noise {
        #[serde(default = "cells")]
        scale: u32,
        #[serde(default = "single")]
        octaves: u32,
        #[serde(default)]
        seed: u32,
        #[serde(default = "one")]
        amount: f32,
        #[serde(default)]
        op: Op,
    },
    /// Squares `cells` across each way, 1 and 0 alternating with 1 at the top left, hard edged.
    Checker {
        #[serde(default = "eight")]
        cells: u32,
        #[serde(default)]
        op: Op,
    },
}

fn eight() -> u32 {
    8
}

fn one() -> f32 {
    1.0
}
fn thin() -> f32 {
    0.01
}
fn cells() -> u32 {
    32
}
fn single() -> u32 {
    1
}

impl Layer {
    fn op(&self) -> Op {
        match self {
            Layer::Profile { op, .. }
            | Layer::Circle { op, .. }
            | Layer::Box { op, .. }
            | Layer::Ring { op, .. }
            | Layer::Noise { op, .. }
            | Layer::Checker { op, .. } => *op,
        }
    }

    /// The layer's value at `(x, y)` (each -1 to 1), and at `(s, t)` (each 0 to 1).
    fn at(&self, x: f32, y: f32, s: f32, t: f32) -> f32 {
        let d = (x * x + y * y).sqrt();
        // 1 well inside a shape, 0 well outside, a straight ramp `edge` wide across its border.
        let shape = |sd: f32, edge: f32| (0.5 - sd / edge).clamp(0.0, 1.0);
        match self {
            Layer::Profile { points, .. } => points.eval(d),
            Layer::Circle { radius, edge, .. } => shape(d - radius, *edge),
            Layer::Box {
                half, round, edge, ..
            } => {
                let inner = half - round;
                let (qx, qy) = (x.abs() - inner, y.abs() - inner);
                let (ox, oy) = (qx.max(0.0), qy.max(0.0));
                let sd = (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - round;
                shape(sd, *edge)
            }
            Layer::Ring {
                radius,
                width,
                edge,
                ..
            } => shape((d - radius).abs() - width * 0.5, *edge),
            Layer::Noise {
                scale,
                octaves,
                seed,
                amount,
                ..
            } => 1.0 - amount * (1.0 - noise::value(s, t, *scale, *octaves, *seed)),
            Layer::Checker { cells, .. } => {
                let square = |x: f32| (x * *cells as f32) as u32;
                f32::from(u8::from((square(s) + square(t)).is_multiple_of(2)))
            }
        }
    }

    /// Why the layer's settings cannot be drawn, if they cannot.
    fn check(&self, i: usize) -> Result<(), String> {
        let bad = |what: &str| Err(format!("layer {i}: {what}"));
        match self {
            Layer::Profile { points, .. } if points.outside_fraction().is_some() => {
                bad("profile values must be from 0 to 1")
            }
            Layer::Circle { radius, edge, .. } if !(*radius >= 0.0 && *edge > 0.0) => {
                bad("radius must not be negative and edge must be above 0")
            }
            Layer::Box {
                half, round, edge, ..
            } if !(*half > 0.0 && (0.0..=*half).contains(round) && *edge > 0.0) => {
                bad("half must be above 0, round 0 to half, edge above 0")
            }
            Layer::Ring {
                radius,
                width,
                edge,
                ..
            } if !(*radius >= 0.0 && *width > 0.0 && *edge > 0.0) => {
                bad("radius must not be negative, width and edge above 0")
            }
            Layer::Noise {
                scale,
                octaves,
                amount,
                ..
            } if !((1..=1024).contains(scale)
                && (1..=8).contains(octaves)
                && (0.0..=1.0).contains(amount)) =>
            {
                bad("scale must be 1 to 1024, octaves 1 to 8, amount 0 to 1")
            }
            Layer::Checker { cells, .. } if !(1..=1024).contains(cells) => {
                bad("cells must be 1 to 1024")
            }
            _ => Ok(()),
        }
    }
}

impl Recipe {
    /// Read and check a recipe file.
    pub fn from_toml(text: &str) -> Result<Recipe, String> {
        let r: Recipe =
            toml::from_str(text).map_err(|e| format!("alpha recipe does not parse: {e}"))?;
        if !(2..=MOST).contains(&r.size) {
            return Err(format!("size must be 2 to {MOST}, got {}", r.size));
        }
        if r.layers.is_empty() {
            return Err("a recipe needs at least one layer".into());
        }
        for (i, layer) in r.layers.iter().enumerate() {
            layer.check(i)?;
        }
        Ok(r)
    }

    /// The picture this recipe draws: each pixel the layers at its center, combined in order
    /// and held to 0 to 1. Fails only for a recipe built in code with a size out of range.
    pub fn draw(&self) -> Result<Picture, String> {
        let n = self.size as usize;
        let mut values = vec![0u16; n * n];
        let mut rows: Vec<&mut [u16]> = values.chunks_mut(n).collect();
        gs_par::apply_each(&mut rows, |j, row| {
            let t = (j as f32 + 0.5) / n as f32;
            for (i, v) in row.iter_mut().enumerate() {
                let s = (i as f32 + 0.5) / n as f32;
                let (x, y) = (2.0 * s - 1.0, 2.0 * t - 1.0);
                let mut layers = self.layers.iter();
                let first = layers.next().map_or(0.0, |l| l.at(x, y, s, t));
                let value = layers.fold(first, |acc, l| {
                    let here = l.at(x, y, s, t);
                    match l.op() {
                        Op::Multiply => acc * here,
                        Op::Add => acc + here,
                        Op::Max => acc.max(here),
                        Op::Min => acc.min(here),
                    }
                });
                *v = quantize(value);
            }
        });
        Picture::new(self.size, self.size, values)
    }
}
