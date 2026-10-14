//! The window and its event loop.

use std::time::Duration;

use sdl3::EventPump;
use sdl3::video::Window;

use crate::clock::Clock;
use crate::input::{EventMapper, Input, Modifiers};
use crate::surface::{self, SurfaceHandle};

/// Shown in task bars and used by compositors to recognise the app.
const APP_ID: &str = "gigasculpt";

/// How to open the window.
#[derive(Clone, Debug)]
pub struct WindowOptions {
    /// Window title.
    pub title: String,
    /// Size in window points (physical pixels divided by the pixel density).
    pub width: u32,
    pub height: u32,
    /// SDL video driver to use (`wayland`, `x11`, `offscreen`), or SDL's own choice.
    pub driver: Option<String>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "GigaSculpt".into(),
            width: 1280,
            height: 800,
            driver: None,
        }
    }
}

/// What one call to [`Platform::pump`] found.
#[derive(Clone, Debug)]
pub struct Pumped {
    /// Input in arrival order.
    pub inputs: Vec<Input>,
    /// When the queue was read, on [`Platform::now_ns`]'s clock.
    pub received_ns: u64,
    /// Modifier keys held now.
    pub modifiers: Modifiers,
    /// Physical pixels per window point now (1.25 on a monitor scaled to 125%).
    pub density: f32,
    /// The display's content scale for the window now, beyond `density`: what the system asks
    /// content to be scaled by (SDL's display scale over the pixel density). 1 where the
    /// density already carries it, as on Wayland; 1.25 on an X11 desktop set to 120 dpi.
    pub content_scale: f32,
}

/// The SDL window and its input. Lives on one thread, like SDL itself.
pub struct Platform {
    sdl: sdl3::Sdl,
    video: sdl3::VideoSubsystem,
    pump: EventPump,
    window: &'static Window,
    mapper: EventMapper,
    clock: Clock,
    text_input: bool,
}

impl Platform {
    /// Starts SDL and opens the window. The window lasts until the process ends (see
    /// [`SurfaceHandle`]), so open one per process.
    pub fn open(options: &WindowOptions) -> Result<Self, String> {
        // Hints must be set before SDL starts the video subsystem. With the pen hints off,
        // SDL makes no mouse or touch events out of pen input; where a platform makes them
        // anyway, the event mapper drops them by their mouse id (`SDL_PEN_MOUSEID`).
        sdl3::hint::set("SDL_PEN_MOUSE_EVENTS", "0");
        sdl3::hint::set("SDL_PEN_TOUCH_EVENTS", "0");
        sdl3::hint::set("SDL_APP_ID", APP_ID);
        if let Some(driver) = &options.driver {
            sdl3::hint::set("SDL_VIDEO_DRIVER", driver);
        }
        let sdl = sdl3::init().map_err(|e| e.to_string())?;
        let video = sdl.video().map_err(|e| e.to_string())?;
        let window = video
            .window(&options.title, options.width, options.height)
            .high_pixel_density()
            .resizable()
            .build()
            .map_err(|e| e.to_string())?;
        let mut pump = sdl.event_pump().map_err(|e| e.to_string())?;
        let events = sdl.event().map_err(|e| e.to_string())?;
        let (density, mut mapper) = (window.pixel_density(), EventMapper::default());
        let clock = Clock::calibrate(&events, &mut pump, window.id(), |e| mapper.feed(e, density))?;
        Ok(Self {
            sdl,
            video,
            pump,
            window: surface::keep(window),
            mapper,
            clock,
            text_input: false,
        })
    }

    /// For creating the GPU surface, on this thread.
    pub fn surface_handle(&self) -> SurfaceHandle {
        SurfaceHandle::new()
    }

    /// The SDL video driver in use, for example `wayland`.
    pub fn driver(&self) -> String {
        self.video.current_video_driver().to_string()
    }

    /// Size in physical pixels.
    pub fn size_in_pixels(&self) -> (u32, u32) {
        self.window.size_in_pixels()
    }

    /// Keeps the window at least `[width, height]` window points, telling the system only when
    /// that changes. Where the system will not hold it, the window stays free to shrink.
    pub fn set_minimum_size(&self, [width, height]: [u32; 2]) {
        if self.window.minimum_size() == (width, height) {
            return;
        }
        // The window lives in an Arc: a clone sets the one window.
        self.window.clone().set_minimum_size(width, height).ok();
    }

    /// Nanoseconds on SDL's event clock.
    pub fn now_ns(&self) -> u64 {
        self.clock.now_ns()
    }

    /// Turns text input (and the input method) on while a text field has focus.
    pub fn set_text_input(&mut self, on: bool) {
        if on == self.text_input {
            return;
        }
        let text = self.video.text_input();
        if on {
            text.start(self.window);
        } else {
            text.stop(self.window);
        }
        self.text_input = on;
    }

    /// True while SDL delivers typed text.
    pub fn text_input_active(&self) -> bool {
        self.video.text_input().is_active(self.window)
    }

    /// Waits up to `wait` for the first event, then reads every queued event.
    pub fn pump(&mut self, wait: Duration) -> Pumped {
        let density = self.window.pixel_density();
        if let Some(first) = self.pump.wait_event_timeout(wait) {
            self.mapper.feed(&first, density);
        }
        while let Some(event) = self.pump.poll_event() {
            self.mapper.feed(&event, density);
        }
        Pumped {
            inputs: self.mapper.take(),
            received_ns: self.clock.now_ns(),
            modifiers: self.sdl.keyboard().mod_state().into(),
            density,
            content_scale: self.window.display_scale() / density,
        }
    }
}
