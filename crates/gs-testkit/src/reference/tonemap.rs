//! A slow f64 reference for gs-render's `AgX` view transform (decision 0019): three.js's
//! `AgXToneMapping` (`src/renderers/shaders/ShaderChunk/tonemapping_pars_fragment.glsl.js`,
//! MIT, Copyright 2010-2024 three.js authors), which follows Filament's operator and takes its
//! matrices from Blender's `AgX`. Written from that source's numbers and order of operations;
//! it shares no code with gs-render. three.js's matrices are GLSL `mat3`s built from columns, so
//! each `[_; 3]` below is a column.

/// Linear Rec. 709 (sRGB primaries) to linear Rec. 2020, and back, as three.js writes them
/// (ITU-R BT.2407, four decimals).
const SRGB_TO_REC2020: [[f64; 3]; 3] = [
    [0.6274, 0.0691, 0.0164],
    [0.3293, 0.9195, 0.0880],
    [0.0433, 0.0113, 0.8956],
];
const REC2020_TO_SRGB: [[f64; 3]; 3] = [
    [1.6605, -0.1246, -0.0182],
    [-0.5876, 1.1329, -0.1006],
    [-0.0728, -0.0083, 1.1187],
];
const INSET: [[f64; 3]; 3] = [
    [0.856627153315983, 0.137318972929847, 0.11189821299995],
    [0.0951212405381588, 0.761241990602591, 0.0767994186031903],
    [0.0482516061458583, 0.101439036467562, 0.811302368396859],
];
const OUTSET: [[f64; 3]; 3] = [
    [
        1.1271005818144368,
        -0.1413297634984383,
        -0.14132976349843826,
    ],
    [
        -0.11060664309660323,
        1.157823702216272,
        -0.11060664309660294,
    ],
    [
        -0.016493938717834573,
        -0.016493938717834257,
        1.2519364065950405,
    ],
];
/// log2 of 2^-10 and 2^6.5 times middle gray 0.18.
const MIN_EV: f64 = -12.47393;
const MAX_EV: f64 = 4.026069;

/// A matrix given by columns times a vector.
fn by_columns(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
}

/// three.js's sigmoid, the 6th-order fit of `AgX`'s default contrast curve (iolite engine's
/// minimal `AgX`, mean squared error 3.67e-6): its coefficients from x^6 down to the constant.
const CONTRAST: [f64; 7] = [15.5, -40.14, 31.96, -6.868, 0.4298, 0.1191, -0.00232];

/// three.js's `AgXToneMapping` at exposure 1, one GLSL statement per line: linear Rec. 709
/// light to display-linear 0..1. The sigmoid is evaluated by Horner's rule.
pub fn agx(rgb: [f64; 3]) -> [f64; 3] {
    let mut color = by_columns(&SRGB_TO_REC2020, rgb);
    color = by_columns(&INSET, color);
    color = color.map(|c| c.max(1e-10));
    color = color.map(libm::log2);
    color = color.map(|c| (c - MIN_EV) / (MAX_EV - MIN_EV));
    color = color.map(|c| c.clamp(0.0, 1.0));
    color = color.map(|x| CONTRAST.iter().fold(0.0, |sum, k| sum * x + k));
    color = by_columns(&OUTSET, color);
    color = color.map(|c| libm::pow(c.max(0.0), 2.2));
    color = by_columns(&REC2020_TO_SRGB, color);
    color.map(|c| c.clamp(0.0, 1.0))
}
