//! The app's icon on its window: `assets/brand/icon-256.png`, built in. X11, Windows and the
//! macOS Dock show it while the app runs; on Wayland only compositors with the toplevel icon
//! protocol do, and the rest read the desktop file's icon instead.

use sdl3::pixels::PixelFormat;
use sdl3::surface::Surface;
use sdl3::video::Window;

/// Written by `assets/brand/make.py` from the icon's source.
const PNG: &[u8] = include_bytes!("../../../assets/brand/icon-256.png");

/// A picture as SDL takes a window icon: 8-bit RGBA rows, top row first, with no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Icon {
    /// The built-in icon, decoded.
    pub fn built_in() -> Result<Icon, String> {
        let text = |e: png::DecodingError| format!("the built-in icon: {e}");
        let mut reader = png::Decoder::new(std::io::Cursor::new(PNG))
            .read_info()
            .map_err(text)?;
        let mut rgba = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let frame = reader.next_frame(&mut rgba).map_err(text)?;
        if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
            return Err("the built-in icon is not 8-bit RGBA".into());
        }
        rgba.truncate(frame.buffer_size());
        Ok(Icon {
            width: frame.width,
            height: frame.height,
            rgba,
        })
    }

    /// The picture as the SDL surface `SDL_SetWindowIcon` takes, borrowing its pixels.
    pub fn surface(&mut self) -> Result<Surface<'_>, String> {
        let pitch = self.width * 4;
        Surface::from_data(
            &mut self.rgba,
            self.width,
            self.height,
            pitch,
            PixelFormat::RGBA32,
        )
        .map_err(|e| e.to_string())
    }
}

/// Gives `window` the built-in icon. Only a look: where the system shows no window icon (SDL's
/// offscreen driver has none) or the call fails, the window keeps the system's default.
pub(crate) fn show(window: &mut Window) {
    let Ok(mut icon) = Icon::built_in() else {
        return;
    };
    if let Ok(surface) = icon.surface() {
        window.set_icon(surface);
    }
}
