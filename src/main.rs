use minifb::{Key, Window, WindowOptions};
use std::time::{Duration, Instant};
use std::thread;

// ── Config ──────────────────────────────────────────────────────────────────
const LAYER_COUNT: usize = 5;
const STARS_PER_LAYER: [usize; LAYER_COUNT] = [8000, 4000, 2500, 1500, 500];
const MAX_STARS: usize = {
    let mut sum = 0;
    let mut i = 0;
    while i < LAYER_COUNT { sum += STARS_PER_LAYER[i]; i += 1; }
    sum
};
const LANDMARK_STARS: usize = 20;
const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;
const TWINKLE_PERIOD: u32 = 8;
const TARGET_FPS: f32 = 60.0;
const FRAME_DT: f32 = 1.0 / TARGET_FPS;
const FRAME_DURATION: Duration = Duration::from_micros(16_667);
const WIDTH: usize = 1280;
const HEIGHT: usize = 720;

// Adaptive star count: frame budget threshold for scaling
const BUDGET_MS: f32 = 14.0;  // if work takes >14ms, start shedding stars
const GROW_MS: f32 = 10.0;    // if work takes <10ms, add more stars back
const STAR_STEP: usize = 500; // add/remove this many per adjustment
const MIN_STARS: usize = 2000;
const ADJUST_INTERVAL: u32 = 30; // check every N frames (0.5s at 60fps)

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
    let r = (((c >> 16) & 0xFF) * factor) >> 8;
    let g = (((c >>  8) & 0xFF) * factor) >> 8;
    let b = (( c        & 0xFF) * factor) >> 8;
    0xFF000000 | (r << 16) | (g << 8) | b
}

#[inline(always)]
fn fp_from_float(v: f32) -> i32 {
    (v * FP_ONE as f32) as i32
}

// ── Precomputed landmark colors ─────────────────────────────────────────────
struct LandmarkColors {
    cross: [u32; 3],
    glow: u32,
}

fn precompute_landmark() -> LandmarkColors {
    LandmarkColors {
        cross: [
            dim_color(0xFFFFEEDD, 195),
            dim_color(0xFFFFEEDD, 135),
            dim_color(0xFFFFEEDD, 75),
        ],
        glow: dim_color(0xFFFFEEDD, 180),
    }
}

// ── SoA Starfield ───────────────────────────────────────────────────────────
// Stars are sorted by size group at init so the render loop can process
// each group in a tight branch-free pass. Group boundaries stored in `groups`.
//
// To add a new per-star field:
// 1. Add Vec<T> here  2. Init in new()  3. Set in init loop
// 4. Swap in sort_by_size()  5. Update  6. Render
struct Starfield {
    x: Vec<i32>,
    y: Vec<i32>,
    speed_x: Vec<i32>,
    speed_y: Vec<i32>,
    color: Vec<u32>,
    base_color: Vec<u32>,
    color_edge: Vec<u32>,
    color_corner: Vec<u32>,
    size: Vec<u8>,
    base_bright: Vec<u8>,
    count: usize,        // total allocated stars
    active: usize,       // currently rendered (adaptive, <= count)
    active_groups: [usize; 5], // group boundaries for active subset
    // Group boundaries for all stars: groups[s] is the start index of size s.
    // groups[4] == count (sentinel).
    groups: [usize; 5],
    max_x: i32,
    max_y: i32,
    width: usize,
    height: usize,
    rng: Rng,
    frame: u32,
    landmark: LandmarkColors,
    // Dirty pixel tracking: indices into the pixel buffer that were written
    // last frame. Cleared instead of full-buffer memset.
    dirty: Vec<u32>,
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
            color_edge: vec![0u32; MAX_STARS],
            color_corner: vec![0u32; MAX_STARS],
            size: vec![0u8; MAX_STARS],
            base_bright: vec![0u8; MAX_STARS],
            count: 0,
            active: 0,
            active_groups: [0; 5],
            groups: [0; 5],
            max_x: (w as i32) << FP_SHIFT,
            max_y: (h as i32) << FP_SHIFT,
            width: w,
            height: h,
            rng: Rng(0xDEADBEEF),
            frame: 0,
            landmark: precompute_landmark(),
            // Generous initial capacity: most stars are 1px so ~MAX_STARS writes,
            // plus multi-pixel stars add a few more. Avoids realloc during render.
            dirty: Vec::with_capacity(MAX_STARS + 5000),
        };

        let mut idx = 0usize;
        for l in 0..LAYER_COUNT {
            let ld = &LAYERS[l];
            let spd_x = fp_from_float(ld.speed_x);
            let spd_y = fp_from_float(ld.drift_y);

            for _ in 0..STARS_PER_LAYER[l] {
                sf.x[idx] = (sf.rng.range(w as u32) as i32) << FP_SHIFT;
                sf.y[idx] = (sf.rng.range(h as u32) as i32) << FP_SHIFT;

                let vary_range = (spd_x / 4).unsigned_abs().max(1);
                let vary = sf.rng.range(vary_range) as i32;
                sf.speed_x[idx] = (spd_x - spd_x / 8 + vary).max(1);

                if spd_y > 0 {
                    let vy = sf.rng.range((spd_y as u32).wrapping_mul(2).max(1)) as i32;
                    sf.speed_y[idx] = vy - spd_y;
                }

                let bright_range = (ld.bright_hi - ld.bright_lo + 1) as u32;
                let bright = ld.bright_lo + sf.rng.range(bright_range) as u8;
                sf.base_bright[idx] = bright;
                let c = dim_color(ld.color, bright as u32);
                sf.color[idx] = c;
                sf.base_color[idx] = c;
                sf.color_edge[idx] = dim_color(c, 200);
                sf.color_corner[idx] = dim_color(c, 120);

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

        // Sort all star arrays by size group so render can process each
        // group in a tight loop without branching on size[i].
        sf.sort_by_size();
        sf.active = sf.count;
        sf.recompute_active_groups();

        sf
    }

    /// Sort all SoA arrays by size, then compute group boundaries.
    fn sort_by_size(&mut self) {
        let n = self.count;
        // Build permutation indices sorted by size (stable sort preserves
        // layer ordering within each size group).
        let mut perm: Vec<usize> = (0..n).collect();
        perm.sort_by_key(|&i| self.size[i]);

        // Apply permutation to every SoA array via a temp copy.
        macro_rules! apply_perm {
            ($field:expr) => {{
                let old: Vec<_> = $field[..n].to_vec();
                for (new_i, &old_i) in perm.iter().enumerate() {
                    $field[new_i] = old[old_i];
                }
            }};
        }
        apply_perm!(self.x);
        apply_perm!(self.y);
        apply_perm!(self.speed_x);
        apply_perm!(self.speed_y);
        apply_perm!(self.color);
        apply_perm!(self.base_color);
        apply_perm!(self.color_edge);
        apply_perm!(self.color_corner);
        apply_perm!(self.size);
        apply_perm!(self.base_bright);

        // Compute group boundaries
        // groups[0] = start of size-0, groups[1] = start of size-1, etc.
        // groups[4] = count (end sentinel)
        self.groups = [n; 5]; // default all to end
        let mut current_size = 255u8;
        for i in 0..n {
            let s = self.size[i];
            if s != current_size {
                // Fill all group starts from current_size+1 to s
                let from = if current_size == 255 { 0 } else { current_size as usize + 1 };
                for g in from..=s as usize {
                    if g < 5 {
                        self.groups[g] = i;
                    }
                }
                current_size = s;
            }
        }
        // Sentinel
        if 4 < 5 {
            self.groups[4] = n;
        }
        // Fill any missing groups (sizes with 0 stars point to next group's start)
        for g in (0..4).rev() {
            if self.groups[g] > self.groups[g + 1] {
                self.groups[g] = self.groups[g + 1];
            }
        }
    }

    /// Recompute active_groups by clamping each group boundary to active count.
    /// Stars are sorted by size, so reducing active count sheds the largest
    /// (most expensive) stars first — landmarks, then 3x3, then 2x2.
    fn recompute_active_groups(&mut self) {
        let a = self.active;
        for g in 0..5 {
            self.active_groups[g] = self.groups[g].min(a);
        }
        // Sentinel
        self.active_groups[4] = a.min(self.groups[4]);
    }

    /// Adaptive star count: shed stars if over budget, grow if under.
    fn adjust_count(&mut self, work_ms: f32) {
        if work_ms > BUDGET_MS && self.active > MIN_STARS {
            self.active = self.active.saturating_sub(STAR_STEP).max(MIN_STARS);
            self.recompute_active_groups();
        } else if work_ms < GROW_MS && self.active < self.count {
            self.active = (self.active + STAR_STEP).min(self.count);
            self.recompute_active_groups();
        }
    }

    fn update(&mut self, dt: f32) {
        let count = self.active; // only update active stars
        let mx = self.max_x;
        let my = self.max_y;

        let dt_scale = dt / FRAME_DT;
        let dt_fp8 = (dt_scale * 256.0) as i32;

        // Bulk X update — local var helps autovectorizer keep value in register
        let x = &mut self.x[..count];
        let sx = &self.speed_x[..count];
        for i in 0..count {
            let mut v = x[i] + (sx[i] >> 4) * (dt_fp8 >> 4);
            if v >= mx { v -= mx; }
            if v < 0   { v += mx; }
            x[i] = v;
        }

        // Bulk Y update
        let y = &mut self.y[..count];
        let sy = &self.speed_y[..count];
        for i in 0..count {
            let mut v = y[i] + (sy[i] >> 4) * (dt_fp8 >> 4);
            if v >= my { v -= my; }
            if v < 0   { v += my; }
            y[i] = v;
        }

        // Twinkle
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
                    if self.size[i] >= 2 {
                        self.color_edge[i] = dim_color(self.color[i], 200);
                        self.color_corner[i] = dim_color(self.color[i], 120);
                    }
                }
            }
        }
    }

    /// Clear only the pixels we drew last frame (dirty-list clear).
    /// Much faster than `pixels.fill(0)` when star coverage is <3% of screen.
    fn clear_dirty(&mut self, pixels: &mut [u32]) {
        for &idx in &self.dirty {
            unsafe { *pixels.get_unchecked_mut(idx as usize) = 0; }
        }
        self.dirty.clear();
    }

    #[inline(never)]
    fn render(&mut self, pixels: &mut [u32]) {
        let w = self.width;
        let h = self.height;
        let groups = self.active_groups;

        // ── Size 0: single pixel ────────────────────────────────────────
        // Tightest possible loop — no branching on size, no multi-pixel logic.
        for i in groups[0]..groups[1] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            let idx = py * w + px;
            unsafe { *pixels.get_unchecked_mut(idx) = self.color[i]; }
            self.dirty.push(idx as u32);
        }

        // ── Size 1: 2x2 ────────────────────────────────────────────────
        for i in groups[1]..groups[2] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            let c = self.color[i];
            unsafe {
                let idx = py * w + px;
                *pixels.get_unchecked_mut(idx) = c;
                self.dirty.push(idx as u32);
                if px + 1 < w {
                    let idx = py * w + px + 1;
                    *pixels.get_unchecked_mut(idx) = c;
                    self.dirty.push(idx as u32);
                }
                if py + 1 < h {
                    let idx = (py + 1) * w + px;
                    *pixels.get_unchecked_mut(idx) = c;
                    self.dirty.push(idx as u32);
                    if px + 1 < w {
                        let idx = (py + 1) * w + px + 1;
                        *pixels.get_unchecked_mut(idx) = c;
                        self.dirty.push(idx as u32);
                    }
                }
            }
        }

        // ── Size 2: 3x3 with precomputed edge/corner ───────────────────
        for i in groups[2]..groups[3] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            let c = self.color[i];
            let dc_edge = self.color_edge[i];
            let dc_corner = self.color_corner[i];
            unsafe {
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
                        let idx = sy * w + sx;
                        *pixels.get_unchecked_mut(idx) = pc;
                        self.dirty.push(idx as u32);
                    }
                }
            }
        }

        // ── Size 3: landmark stars ──────────────────────────────────────
        for i in groups[3]..groups[4] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            unsafe {
                let idx = py * w + px;
                *pixels.get_unchecked_mut(idx) = 0xFFFFFFFF;
                self.dirty.push(idx as u32);
                for d in 0..3usize {
                    let fc = self.landmark.cross[d];
                    let d1 = d + 1;
                    if px + d1 < w {
                        let idx = py * w + px + d1;
                        *pixels.get_unchecked_mut(idx) = fc;
                        self.dirty.push(idx as u32);
                    }
                    if px >= d1 {
                        let idx = py * w + px - d1;
                        *pixels.get_unchecked_mut(idx) = fc;
                        self.dirty.push(idx as u32);
                    }
                    if py + d1 < h {
                        let idx = (py + d1) * w + px;
                        *pixels.get_unchecked_mut(idx) = fc;
                        self.dirty.push(idx as u32);
                    }
                    if py >= d1 {
                        let idx = (py - d1) * w + px;
                        *pixels.get_unchecked_mut(idx) = fc;
                        self.dirty.push(idx as u32);
                    }
                }
                let glow = self.landmark.glow;
                for dy in 0..3usize {
                    let sy = (py + dy).wrapping_sub(1);
                    if sy >= h { continue; }
                    for dx in 0..3usize {
                        let sx = (px + dx).wrapping_sub(1);
                        if sx >= w { continue; }
                        if dx == 1 && dy == 1 { continue; }
                        let idx = sy * w + sx;
                        *pixels.get_unchecked_mut(idx) = glow;
                        self.dirty.push(idx as u32);
                    }
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

/// Draw a number at (x,y) with given color and pixel scale. Returns width in pixels.
fn draw_num(pixels: &mut [u32], w: usize, x: usize, y: usize, val: u32, color: u32, scale: usize) -> usize {
    let mut digits = [0u8; 6];
    let mut n = val;
    let mut len = 0;
    if n == 0 {
        digits[0] = 0;
        len = 1;
    } else {
        while n > 0 && len < 6 {
            digits[len] = (n % 10) as u8;
            n /= 10;
            len += 1;
        }
        digits[..len].reverse();
    }
    for ci in 0..len {
        let digit = digits[ci] as usize;
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
    len * 4 * scale
}

/// Draw HUD: "FPS | STARS | DROPS"
fn draw_hud(pixels: &mut [u32], w: usize, fps: u32, stars: u32, drops: u32) {
    let y = 10;
    let scale = 2;
    let gap = 3 * scale; // gap between numbers
    let mut x = 10;
    // FPS in green
    x += draw_num(pixels, w, x, y, fps, 0xFF44FF44, scale) + gap;
    // Star count in dim cyan
    x += draw_num(pixels, w, x, y, stars, 0xFF44AAAA, scale) + gap;
    // Drop count in red (or dim if zero)
    let drop_color = if drops > 0 { 0xFFFF4444 } else { 0xFF444444 };
    draw_num(pixels, w, x, y, drops, drop_color, scale);
}

// ── Main ────────────────────────────────────────────────────────────────────
fn main() {
    let mut window = Window::new(
        "Stargazer",
        WIDTH,
        HEIGHT,
        WindowOptions {
            borderless: true,
            resize: false,
            scale: minifb::Scale::X1,
            scale_mode: minifb::ScaleMode::AspectRatioStretch,
            ..WindowOptions::default()
        },
    )
    .expect("Failed to create window");

    window.set_target_fps(0);

    let mut pixels = vec![0u32; WIDTH * HEIGHT];
    let mut sf = Starfield::new(WIDTH, HEIGHT);

    let mut fps: u32 = 0;
    let mut frame_count: u32 = 0;
    let mut drop_count: u32 = 0;
    let mut total_drops: u32 = 0;
    let mut reported_drops: u32 = 0;
    let mut fps_timer = Instant::now();
    let mut frame_start = Instant::now();
    let mut adjust_counter: u32 = 0;
    let mut work_ms_accum: f32 = 0.0;

    eprintln!(
        "Stargazer: {}x{}, {} stars (groups: 1px={}, 2x2={}, 3x3={}, landmark={})",
        WIDTH, HEIGHT, sf.count,
        sf.groups[1] - sf.groups[0],
        sf.groups[2] - sf.groups[1],
        sf.groups[3] - sf.groups[2],
        sf.groups[4] - sf.groups[3],
    );

    while window.is_open() && !window.is_key_down(Key::Escape) && !window.is_key_down(Key::Q) {
        let now = Instant::now();
        let raw_dt = now.duration_since(frame_start).as_secs_f32();
        let dt = raw_dt.min(0.05);
        frame_start = now;

        sf.update(dt);

        // Dirty-list clear: zero only the pixels we drew last frame
        sf.clear_dirty(&mut pixels);

        // Render stars into pixel buffer
        sf.render(&mut pixels);

        // Measure work time (before present, which includes the
        // buffer copy and any OS-level vsync wait)
        let work_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        work_ms_accum += work_ms;

        // FPS & drop counter
        frame_count += 1;
        if raw_dt > FRAME_DT * 1.5 {
            total_drops += 1;
        }
        let elapsed = fps_timer.elapsed();
        if elapsed.as_secs() >= 1 {
            fps = frame_count;
            drop_count = total_drops - reported_drops;
            reported_drops = total_drops;
            frame_count = 0;
            fps_timer = Instant::now();
        }

        // Adaptive star count: check every ADJUST_INTERVAL frames
        adjust_counter += 1;
        if adjust_counter >= ADJUST_INTERVAL {
            let avg_work = work_ms_accum / adjust_counter as f32;
            sf.adjust_count(avg_work);
            work_ms_accum = 0.0;
            adjust_counter = 0;
        }

        // HUD: FPS (green) | star count (cyan) | drops/sec (red)
        draw_hud(&mut pixels, WIDTH, fps, sf.active as u32, drop_count);

        window.update_with_buffer(&pixels, WIDTH, HEIGHT)
            .expect("Failed to update window");

        // Precise frame cap
        let work_time = frame_start.elapsed();
        if work_time < FRAME_DURATION {
            let sleep_time = FRAME_DURATION - work_time;
            if sleep_time > Duration::from_millis(2) {
                thread::sleep(sleep_time - Duration::from_millis(1));
            }
            while frame_start.elapsed() < FRAME_DURATION {
                std::hint::spin_loop();
            }
        }
    }
}
