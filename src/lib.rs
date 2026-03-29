#[cfg(feature = "web")]
mod starfield;

#[cfg(feature = "web")]
use starfield::*;
#[cfg(feature = "web")]
use wasm_bindgen::prelude::*;
#[cfg(feature = "web")]
use wasm_bindgen::Clamped;
#[cfg(feature = "web")]
use web_sys::{CanvasRenderingContext2d, ImageData};

#[cfg(feature = "web")]
const WIDTH: usize = 1280;
#[cfg(feature = "web")]
const HEIGHT: usize = 720;

#[cfg(feature = "web")]
#[wasm_bindgen]
pub struct StargazerWeb {
    sf: Starfield,
    hud: Hud,
    pixels: Vec<u32>,
    rgba: Vec<u8>,
    fps: u32,
    frame_count: u32,
    last_fps_time: f64,
}

#[cfg(feature = "web")]
#[wasm_bindgen]
impl StargazerWeb {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            sf: Starfield::new(WIDTH, HEIGHT),
            hud: Hud::new(),
            pixels: vec![0u32; WIDTH * HEIGHT],
            rgba: vec![0u8; WIDTH * HEIGHT * 4],
            fps: 0,
            frame_count: 0,
            last_fps_time: 0.0,
        }
    }

    pub fn width(&self) -> u32 { WIDTH as u32 }
    pub fn height(&self) -> u32 { HEIGHT as u32 }
    pub fn stars(&self) -> u32 { self.sf.active as u32 }

    /// Call once per frame. `time_ms` is performance.now() from JS.
    /// `dt` is seconds since last frame.
    pub fn frame(&mut self, dt: f32, time_ms: f64, ctx: &CanvasRenderingContext2d) {
        let dt = dt.min(0.05);

        self.sf.update(dt);
        self.pixels.fill(0);
        self.sf.render(&mut self.pixels);

        // FPS
        self.frame_count += 1;
        if time_ms - self.last_fps_time >= 1000.0 {
            self.fps = self.frame_count;
            self.frame_count = 0;
            self.last_fps_time = time_ms;
        }
        self.hud.draw(&mut self.pixels, WIDTH, [self.fps, self.sf.active as u32, 0]);

        // Convert ARGB → RGBA for canvas
        argb_to_rgba(&self.pixels, &mut self.rgba);

        // Put to canvas
        let data = ImageData::new_with_u8_clamped_array_and_sh(
            Clamped(&self.rgba),
            WIDTH as u32,
            HEIGHT as u32,
        )
        .unwrap();
        ctx.put_image_data(&data, 0.0, 0.0).unwrap();
    }
}
