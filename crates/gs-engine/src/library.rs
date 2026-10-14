//! What the window's Library and Alpha palette show that only the engine holds: the shipped
//! alphas, each as a thumbnail (decision 0025, section 13), and what hovering one says.

use gs_core::brush::AlphaSpec;

/// Every shipped alpha: its library name and its title, in name order.
pub fn alphas() -> Vec<(&'static str, &'static str)> {
    (crate::alpha::shipped().iter())
        .map(|a| (a.name, a.recipe.name.as_str()))
        .collect()
}

/// What hovering alpha `name`'s picture says: its title, size in pixels and bits per pixel, as
/// `alpha.list` gives them ("Square, 1024 x 1024, 16-bit"). `None` for an alpha the library does
/// not have.
pub fn alpha_hover(name: &str) -> Option<String> {
    let a = crate::alpha::find(name).ok()?;
    let (title, side, bits) = (&a.recipe.name, a.recipe.size, crate::alpha::BITS);
    Some(format!("{title}, {side} x {side}, {bits}-bit"))
}

/// Alpha `name` as a `size`-pixel square of grey values, 0 to 255, top row first: each pixel
/// the value at its centre, as `alpha.preview` shows it on its own. `None` for an alpha the
/// library does not have or a size of 0.
pub fn alpha_thumbnail(name: &str, size: u32) -> Option<Vec<u8>> {
    let spec = AlphaSpec::plain(name);
    let pyramid = (size > 0).then(|| crate::alpha::pyramid(&spec).ok())??;
    let level = pyramid.level_for(1.0, 4.0 / size as f32, spec.tiles);
    let at = |i: u32| 2.0 * (i as f32 + 0.5) / size as f32 - 1.0;
    let grey = |x, y| (pyramid.sample(level, at(x), -at(y), spec.tiles) * 255.0).round() as u8;
    Some(
        (0..size)
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .map(|(x, y)| grey(x, y))
            .collect(),
    )
}
