//! The window's icon: the picture built into gs-platform is one SDL's window icon takes.

use gs_platform::Icon;
use sdl3::pixels::PixelFormat;

/// 256 pixels square of 8-bit RGBA, transparent in its corner (outside the rounded square) and
/// opaque at its center, and an SDL surface of that size in RGBA order.
#[test]
fn the_built_in_icon_decodes_to_the_surface_the_window_takes() {
    let mut icon = Icon::built_in().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((icon.width, icon.height), (256, 256));
    assert_eq!(
        icon.rgba.len(),
        256 * 256 * 4,
        "four bytes a pixel, no padding"
    );
    let alpha = |x: usize, y: usize| icon.rgba[(y * 256 + x) * 4 + 3];
    let (corner, center) = (alpha(0, 0), alpha(128, 128));
    assert_eq!(
        (corner, center),
        (0, 255),
        "outside and inside the rounded square"
    );
    let surface = icon.surface().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(surface.size(), (256, 256));
    assert_eq!(surface.pixel_format(), PixelFormat::RGBA32);
}
