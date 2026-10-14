//! The shared material model on the CPU (research 11, phases 2 and 3): one physically based
//! BRDF, the environments that light it, and the integrals a MatCap bake and the tests need.
//! Deterministic everywhere: `libm` for transcendental functions, fixed sample sets, and
//! `gs-par`'s fixed-chunk parallelism.
//!
//! - [`material`]: the parameters (`OpenPBR` names) and the BRDF they define;
//! - [`brdf`]: the lobes it is built from (GGX, Smith, Schlick with F82, Lambert);
//! - [`dfg`]: the GGX albedo table behind energy compensation and albedo scaling;
//! - [`environment`]: studios and HDR images as metered radiance cubes, with [`cube`] storage
//!   and [`irradiance`] (direct integral and SH9);
//! - [`lobe`]: filtered importance sampling of a lobe against an environment, and the
//!   GGX-prefiltered radiance a live lit mode will read;
//! - [`burley`]: Burley's diffusion profile for subsurface scattering;
//! - [`light`]: Lit mode's disc lights and the key light taken out of an environment;
//! - [`lit`]: Lit mode on the CPU, the reference its shader is checked against.
//!
//! References: Filament's documentation (BRDF, DFG table, compensation), Karis 2013 (split sum),
//! glTF 2.0 Appendix B, `OpenPBR` 1.1 (parameters, layering, F82), Heitz 2014 (Smith), Colbert
//! and Krivanek 2007 (filtered importance sampling), Ramamoorthi and Hanrahan 2001 (SH
//! irradiance), Christensen and Burley 2015 (diffusion profile).

pub mod brdf;
pub mod burley;
pub mod cube;
pub mod dfg;
pub mod environment;
pub mod hdr;
pub mod irradiance;
pub mod light;
pub mod lit;
pub mod lobe;
pub mod material;

pub use environment::{Environment, Studio};
pub use material::{Brdf, Color, Material};

use crate::error::RenderError;

/// Linear-light RGB.
pub use gs_core::material::Rgb;

/// `c` as the bits of an opaque RGBA16F texel, for a GPU upload. A value past the largest half
/// float (65504) is held at it: stored as infinity, it made every pixel that sampled it NaN in
/// the view transform, and so black (review 0005, m4).
pub(crate) fn half_rgba(c: Rgb) -> [u16; 4] {
    let most = f32::from(half::f16::MAX);
    [c[0], c[1], c[2], 1.0].map(|v| half::f16::from_f32(v.min(most)).to_bits())
}

/// Reads a TOML file into `T` (gs-core's reader). An error starts with the key it is about,
/// for example `material.specular_ior: invalid type: string "x", expected f32`.
pub(crate) fn parse_toml<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, RenderError> {
    gs_core::material::parse_toml(text).map_err(RenderError::Recipe)
}
