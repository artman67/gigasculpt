//! Color math on the CPU, through `libm` so it is deterministic: the sRGB transfer functions
//! and the view transforms of the display step (research 11, section 6). `shaders/display.wgsl`
//! is the GPU twin of the view transforms; the display tests hold the two together.

/// The sRGB transfer functions (0..1), both ways: gs-core's, the one copy on the CPU.
pub(crate) use gs_core::columns::{linear_to_srgb, srgb_to_linear};

/// A 16-bit sRGB sample to linear light.
pub(crate) fn srgb16_to_linear(v: u16) -> f32 {
    srgb_to_linear(f32::from(v) / 65535.0)
}

type Rgb = [f32; 3];

fn max3(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2])
}

fn min3(c: Rgb) -> f32 {
    c[0].min(c[1]).min(c[2])
}

/// Khronos PBR Neutral's constants: Fresnel at normal incidence of IOR 1.5, where highlight
/// compression starts, and how fast compressed highlights desaturate.
const F90: f32 = 0.04;
const KS: f32 = 0.8 - F90;
const KD: f32 = 0.15;

/// Khronos PBR Neutral (2024): scene-linear Rec. 709 to display-linear 0..1. Below the
/// shoulder it only subtracts a small offset, so base colors show as set under unit light;
/// above it, highlights compress toward white without a hue shift.
/// Spec: <https://github.com/KhronosGroup/ToneMapping/blob/main/PBR_Neutral/README.md>.
pub(crate) fn pbr_neutral(rgb: Rgb) -> Rgb {
    let c = rgb.map(|v| v.max(0.0));
    let x = min3(c);
    let offset = if x < 2.0 * F90 {
        x - x * x / (4.0 * F90)
    } else {
        F90
    };
    let c = c.map(|v| v - offset);
    let peak = max3(c);
    if peak < KS {
        return c;
    }
    let d = 1.0 - KS;
    let new_peak = 1.0 - d * d / (peak + d - KS);
    let g = 1.0 / (KD * (peak - new_peak) + 1.0);
    c.map(|v| v * (new_peak / peak) * g + new_peak * (1.0 - g))
}

/// Row-major 3x3 matrix times a color.
fn mul(m: &[Rgb; 3], c: Rgb) -> Rgb {
    m.map(|row| row[0] * c[0] + row[1] * c[1] + row[2] * c[2])
}

/// Rec. 709 to Rec. 2020 primaries and back (ITU-R BT.2087), linear light. `AgX`'s matrices are
/// built for Rec. 2020.
const REC709_TO_REC2020: [Rgb; 3] = [
    [0.627_404, 0.329_282, 0.043_313_6],
    [0.069_097, 0.919_54, 0.011_361_2],
    [0.016_391_6, 0.088_013_2, 0.895_595],
];
const REC2020_TO_REC709: [Rgb; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_9],
    [-0.124_550_5, 1.132_899_9, -0.008_349_4],
    [-0.018_150_8, -0.100_578_9, 1.118_729_7],
];

/// `AgX`'s inset and outset matrices (rows), from Filament's `ToneMapper.cpp` (Apache-2.0,
/// Copyright 2021 The Android Open Source Project), which took them from Blender's Rec. 2020
/// `AgX`. Filament lists them by column; these are the same numbers by row, and the outset is
/// the inverse of Filament's `AgXOutsetMatrixInv`, computed once.
const AGX_INSET: [Rgb; 3] = [
    [0.856_627_15, 0.095_121_24, 0.048_251_606],
    [0.137_318_97, 0.761_242, 0.101_439_04],
    [0.111_898_21, 0.076_799_42, 0.811_302_4],
];
const AGX_OUTSET: [Rgb; 3] = [
    [1.127_100_6, -0.110_606_64, -0.016_493_939],
    [-0.141_329_76, 1.157_823_7, -0.016_493_939],
    [-0.141_329_76, -0.110_606_64, 1.251_936_4],
];
/// log2 of 2^-10 and 2^6.5 times middle gray: the range `AgX` encodes.
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

/// `AgX`'s sigmoid: the iolite engine's 6th-order fit of the default contrast curve, as three.js
/// uses it. Filament uses the 7th-order fit from the same source, which has the lower mean error
/// but ends at 0.982 instead of 1, so white never reached full white; decision 0019 records the
/// change.
fn agx_contrast(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x
        - 0.00232
}

/// `AgX` as Filament and three.js implement it (no look): scene-linear Rec. 709 to
/// display-linear 0..1. Brights go toward white the way film does. It has no exact inverse.
pub(crate) fn agx(rgb: Rgb) -> Rgb {
    let v = mul(&AGX_INSET, mul(&REC709_TO_REC2020, rgb.map(|c| c.max(0.0))));
    let v = v.map(|c| {
        let ev = (libm::log2f(c.max(1e-10)) - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
        agx_contrast(ev.clamp(0.0, 1.0))
    });
    let v = mul(&AGX_OUTSET, v).map(|c| libm::powf(c.max(0.0), 2.2));
    mul(&REC2020_TO_REC709, v).map(|c| c.clamp(0.0, 1.0))
}
