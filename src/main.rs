use minifb::{Key, Window, WindowOptions};
use std::time::Instant;

// ── Config ──────────────────────────────────────────────────────────────────
const LAYER_COUNT: usize = 5;
const STARS_PER_LAYER: [usize; LAYER_COUNT] = [8000, 4000, 2500, 1500, 500];
const MAX_STARS: usize = 8000 + 4000 + 2500 + 1500 + 500;
const LANDMARK_STARS: usize = 20;
const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;
const TWINKLE_PERIOD: u32 = 8;

// ── Layer definitions ───────────────────────────────────────────────────────
struct LayerDef {
    speed_x: f32,
    drift_y: f32,
    color: u32,
    max_size: u8,
    bright_lo: u8,
    bright_hi: u8,
}

const LAYERS: [LayerDef; LAYER_COUNT] = [
    LayerDef { speed_x: 0.25, drift_y: 0.0,  color: 0xFF4466AA, max_size: 0, bright_lo: 40,  bright_hi: 90  },
    LayerDef { speed_x: 0.5,  drift_y: 0.02, color: 0xFF7788BB, max_size: 0, bright_lo: 70,  bright_hi: 130 },
    LayerDef { speed_x: 1.0,  drift_y: 0.05, color: 0xFFAABBCC, max_size: 0, bright_lo: 100, bright_hi: 180 },
    LayerDef { speed_x: 2.0,  drift_y: 0.08, color: 0xFFDDCCBB, max_size: 1, bright_lo: 150, bright_hi: 220 },
    LayerDef { speed_x: 4.0,  drift_y: 0.12, color: 0xFFFFEEDD, max_size: 2, bright_lo: 200, bright_hi: 255 },
];

// ── Fast PRNG ───────────────────────────────────────────────────────────────
struct Rng(u32);

impl Rng {
    #[inline(always)]
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    #[inline(always)]
    fn range(&mut self, max: u32) -> u32 {
        self.next() % max
    }
}

// ── Color helpers ───────────────────────────────────────────────────────────
#[inline(always)]
fn dim_color(c: u32, factor: u32) -> u32 {
    let r = ((c >> 16) & 0xFF) * factor / 255;
    let g = ((c >>  8) & 0xFF) * factor / 255;
    let b = ((c      ) & 0xFF) * factor / 255;
    0xFF000000 | (r << 16) | (g << 8) | b
}

#[inline(always)]
fn fp_from_float(v: f32) -> i32 {
    (v * FP_ONE as f32) as i32
}

// ── SoA Starfield ───────────────────────────────────────────────────────────
struct Starfield {
    x: Vec<i32>,
    y: Vec<i32>,
    speed_x: Vec<i32>,
    speed_y: Vec<i32>,
    color: Vec<u32>,
    base_color: Vec<u32>,
    size: Vec<u8>,
    base_bright: Vec<u8>,
    count: usize,
    max_x: i32,
    max_y: i32,
    width: usize,
    height: usize,
    rng: Rng,
    frame: u32,
}

impl Starfield {
    fn new(w: usize, h: usize) -> Self {
        let mut sf = Starfield {
            x: vec![0i32; MAX_STARS],
            y: vec![0i32; MAX_STARS],
            speed_x: vec![0i32; MAX_STARS],
            speed_y: vec![0i32; MAX_STARS],
            color: vec![0u32; MAX_STARS],
            base_color: vec![0u32; MAX_STARS],
            size: vec![0u8; MAX_STARS],
            base_bright: vec![0u8; MAX_STARS],
            count: 0,
            max_x: (w as i32) << FP_SHIFT,
            max_y: (h as i32) << FP_SHIFT,
            width: w,
            height: h,
            rng: Rng(0xDEADBEEF),
            frame: 0,
        };

        let mut idx = 0usize;
        for l in 0..LAYER_COUNT {
            let ld = &LAYERS[l];
            let spd_x = fp_from_float(ld.speed_x);
            let spd_y = fp_from_float(ld.drift_y);

            for _ in 0..STARS_PER_LAYER[l] {
                sf.x[idx] = (sf.rng.range(w as u32) as i32) << FP_SHIFT;
                sf.y[idx] = (sf.rng.range(h as u32) as i32) << FP_SHIFT;

                // Speed variation ±12.5%
                let vary_range = (spd_x / 4).unsigned_abs().max(1);
                let vary = sf.rng.range(vary_range) as i32;
                sf.speed_x[idx] = (spd_x - spd_x / 8 + vary).max(1);

                // Vertical drift
                if spd_y > 0 {
                    let vy = sf.rng.range((spd_y as u32).wrapping_mul(2).max(1)) as i32;
                    sf.speed_y[idx] = vy - spd_y;
                }

                // Brightness
                let bright_range = (ld.bright_hi - ld.bright_lo + 1) as u32;
                let bright = ld.bright_lo + sf.rng.range(bright_range) as u8;
                sf.base_bright[idx] = bright;
                let c = dim_color(ld.color, bright as u32);
                sf.color[idx] = c;
                sf.base_color[idx] = c;

                // Size
                sf.size[idx] = if ld.max_size > 0 {
                    sf.rng.range(ld.max_size as u32 + 1) as u8
                } else {
                    0
                };

                idx += 1;
            }
        }

        // Landmark stars
        let start = idx.saturating_sub(LANDMARK_STARS);
        for i in start..idx {
            sf.size[i] = 3;
            sf.base_bright[i] = 255;
            sf.color[i] = 0xFFFFFFFF;
            sf.base_color[i] = 0xFFFFFFFF;
        }

        sf.count = idx;
        sf
    }

    fn update(&mut self) {
        let count = self.count;
        let mx = self.max_x;
        let my = self.max_y;

        // Bulk X update — tight loop, auto-vectorizable
        for i in 0..count {
            self.x[i] += self.speed_x[i];
            if self.x[i] >= mx { self.x[i] -= mx; }
            if self.x[i] < 0   { self.x[i] += mx; }
        }

        // Bulk Y update
        for i in 0..count {
            self.y[i] += self.speed_y[i];
            if self.y[i] >= my { self.y[i] -= my; }
            if self.y[i] < 0   { self.y[i] += my; }
        }

        // Twinkle: modulate a subset every N frames
        self.frame += 1;
        if self.frame % TWINKLE_PERIOD == 0 {
            for i in (0..count).step_by(7) {
                let r = self.rng.next();
                let delta = (r & 0x3F) as i32 - 32;
                let bright = (self.base_bright[i] as i32 + delta).clamp(20, 255) as u32;

                let c = self.base_color[i];
                let cr = (c >> 16) & 0xFF;
                let cg = (c >>  8) & 0xFF;
                let cb =  c        & 0xFF;
                let maxc = cr.max(cg).max(cb);
                if maxc > 0 {
                    let nr = (cr * bright / maxc).min(255);
                    let ng = (cg * bright / maxc).min(255);
                    let nb = (cb * bright / maxc).min(255);
                    self.color[i] = 0xFF000000 | (nr << 16) | (ng << 8) | nb;
                }
            }
        }
    }

    #[inline(never)] // keep this as a clear compilation unit for the optimizer
    fn render(&self, pixels: &mut [u32]) {
        let w = self.width;
        let h = self.height;
        let count = self.count;

        for i in 0..count {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            let c = self.color[i];

            if px >= w || py >= h { continue; }

            unsafe {
                match self.size[i] {
                    0 => {
                        // 1x1 — single pixel, unchecked for speed
                        *pixels.get_unchecked_mut(py * w + px) = c;
                    }
                    1 => {
                        // 2x2
                        *pixels.get_unchecked_mut(py * w + px) = c;
                        if px + 1 < w {
                            *pixels.get_unchecked_mut(py * w + px + 1) = c;
                        }
                        if py + 1 < h {
                            *pixels.get_unchecked_mut((py + 1) * w + px) = c;
                            if px + 1 < w {
                                *pixels.get_unchecked_mut((py + 1) * w + px + 1) = c;
                            }
                        }
                    }
                    2 => {
                        // 3x3 with dimmed edges/corners
                        let dc_edge = dim_color(c, 200);
                        let dc_corner = dim_color(c, 140);
                        for dy in 0..3usize {
                            let sy = (py + dy).wrapping_sub(1);
                            if sy >= h { continue; }
                            for dx in 0..3usize {
                                let sx = (px + dx).wrapping_sub(1);
                                if sx >= w { continue; }
                                let is_corner = dx != 1 && dy != 1;
                                let is_center = dx == 1 && dy == 1;
                                let pc = if is_center { c }
                                         else if is_corner { dc_corner }
                                         else { dc_edge };
                                *pixels.get_unchecked_mut(sy * w + sx) = pc;
                            }
                        }
                    }
                    3 => {
                        // Landmark: bright cross + glow
                        *pixels.get_unchecked_mut(py * w + px) = 0xFFFFFFFF;
                        for d in 1..=3usize {
                            let fade = (255 - d * 60) as u32;
                            let fc = dim_color(0xFFFFEEDD, fade);
                            if px + d < w { *pixels.get_unchecked_mut(py * w + px + d) = fc; }
                            if px >= d    { *pixels.get_unchecked_mut(py * w + px - d) = fc; }
                            if py + d < h { *pixels.get_unchecked_mut((py + d) * w + px) = fc; }
                            if py >= d    { *pixels.get_unchecked_mut((py - d) * w + px) = fc; }
                        }
                        for dy in 0..3usize {
                            let sy = (py + dy).wrapping_sub(1);
                            if sy >= h { continue; }
                            for dx in 0..3usize {
                                let sx = (px + dx).wrapping_sub(1);
                                if sx >= w { continue; }
                                if dx == 1 && dy == 1 { continue; }
                                let glow = dim_color(0xFFFFEEDD, 180);
                                let pidx = sy * w + sx;
                                if *pixels.get_unchecked(pidx) < glow {
                                    *pixels.get_unchecked_mut(pidx) = glow;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// ── Bitmap FPS display ──────────────────────────────────────────────────────
const FONT3X5: [[u8; 5]; 10] = [
    [0x7,0x5,0x5,0x5,0x7],
    [0x2,0x6,0x2,0x2,0x7],
    [0x7,0x1,0x7,0x4,0x7],
    [0x7,0x1,0x7,0x1,0x7],
    [0x5,0x5,0x7,0x1,0x1],
    [0x7,0x4,0x7,0x1,0x7],
    [0x7,0x4,0x7,0x5,0x7],
    [0x7,0x1,0x1,0x1,0x1],
    [0x7,0x5,0x7,0x5,0x7],
    [0x7,0x5,0x7,0x1,0x7],
];

fn draw_fps(pixels: &mut [u32], w: usize, x: usize, y: usize, fps: u32, scale: usize) {
    let color = 0xFF44FF44u32;
    let s = format!("{fps}");
    for (ci, ch) in s.chars().enumerate() {
        let digit = match ch.to_digit(10) {
            Some(d) => d as usize,
            None => continue,
        };
        let ox = x + ci * 4 * scale;
        for row in 0..5usize {
            let bits = FONT3X5[digit][row];
            for col in 0..3usize {
                if bits & (4 >> col) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = ox + col * scale + sx;
                            let py = y + row * scale + sy;
                            let idx = py * w + px;
                            if px < w && idx < pixels.len() {
                                pixels[idx] = color;
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── Main ────────────────────────────────────────────────────────────────────
fn main() {
    let width: usize = 1280;
    let height: usize = 720;

    let mut window = Window::new(
        "Stargazer",
        width,
        height,
        WindowOptions {
            borderless: true,
            resize: false,
            scale: minifb::Scale::X1,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .expect("Failed to create window");

    window.set_target_fps(60);

    let mut pixels = vec![0u32; width * height];
    let mut sf = Starfield::new(width, height);

    let mut fps: u32 = 0;
    let mut frame_count: u32 = 0;
    let mut fps_timer = Instant::now();

    eprintln!("Stargazer: {}x{}, {} stars", width, height, sf.count);

    while window.is_open() && !window.is_key_down(Key::Escape) && !window.is_key_down(Key::Q) {
        sf.update();

        // Clear
        pixels.fill(0);

        // Render stars into pixel buffer
        sf.render(&mut pixels);

        // FPS counter
        frame_count += 1;
        let elapsed = fps_timer.elapsed();
        if elapsed.as_secs() >= 1 {
            fps = frame_count;
            frame_count = 0;
            fps_timer = Instant::now();
        }
        draw_fps(&mut pixels, width, 10, 10, fps, 2);

        // Upload buffer and present
        window.update_with_buffer(&pixels, width, height)
            .expect("Failed to update window");
    }
}
