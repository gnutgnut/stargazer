// ── Config ──────────────────────────────────────────────────────────────────
pub const LAYER_COUNT: usize = 5;
pub const STARS_PER_LAYER: [usize; LAYER_COUNT] = [8000, 4000, 2500, 1500, 500];
pub const MAX_STARS: usize = {
    let mut sum = 0;
    let mut i = 0;
    while i < LAYER_COUNT { sum += STARS_PER_LAYER[i]; i += 1; }
    sum
};
// Ceiling for upward scaling — allocate room for 3x the default count.
// Extra stars are spawned as 1px dots when the system has headroom.
pub const STAR_CEILING: usize = MAX_STARS * 3;
const LANDMARK_STARS: usize = 20;
pub const FP_SHIFT: i32 = 16;
pub const FP_ONE: i32 = 1 << FP_SHIFT;
pub const TWINKLE_PERIOD: u32 = 8;
pub const TARGET_FPS: f32 = 60.0;
pub const FRAME_DT: f32 = 1.0 / TARGET_FPS;

// Adaptive star count
pub const BUDGET_MS: f32 = 15.0;
pub const GROW_MS: f32 = 12.0;
pub const SHED_STEP: usize = 2000;
pub const GROW_STEP: usize = 500;
pub const MIN_STARS: usize = 2000;
pub const ADJUST_INTERVAL: u32 = 15;

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
pub struct Rng(pub u32);

impl Rng {
    #[inline(always)]
    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    #[inline(always)]
    pub fn range(&mut self, max: u32) -> u32 {
        self.next() % max
    }
}

// ── Color helpers ───────────────────────────────────────────────────────────
#[inline(always)]
pub fn dim_color(c: u32, factor: u32) -> u32 {
    let r = (((c >> 16) & 0xFF) * factor) >> 8;
    let g = (((c >>  8) & 0xFF) * factor) >> 8;
    let b = (( c        & 0xFF) * factor) >> 8;
    0xFF000000 | (r << 16) | (g << 8) | b
}

#[inline(always)]
pub fn fp_from_float(v: f32) -> i32 {
    (v * FP_ONE as f32) as i32
}

/// Convert ARGB pixel buffer to RGBA byte buffer (for canvas ImageData).
#[allow(dead_code)]
pub fn argb_to_rgba(argb: &[u32], rgba: &mut [u8]) {
    for (i, &px) in argb.iter().enumerate() {
        let off = i * 4;
        rgba[off]     = ((px >> 16) & 0xFF) as u8; // R
        rgba[off + 1] = ((px >>  8) & 0xFF) as u8; // G
        rgba[off + 2] = ( px        & 0xFF) as u8; // B
        rgba[off + 3] = 255;                        // A (always opaque)
    }
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
pub struct Starfield {
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
    pub count: usize,
    pub active: usize,
    active_groups: [usize; 5],
    pub groups: [usize; 5],
    max_x: i32,
    max_y: i32,
    pub width: usize,
    pub height: usize,
    rng: Rng,
    frame: u32,
    landmark: LandmarkColors,
}

impl Starfield {
    pub fn new(w: usize, h: usize) -> Self {
        let mut sf = Starfield {
            x: vec![0i32; STAR_CEILING],
            y: vec![0i32; STAR_CEILING],
            speed_x: vec![0i32; STAR_CEILING],
            speed_y: vec![0i32; STAR_CEILING],
            color: vec![0u32; STAR_CEILING],
            base_color: vec![0u32; STAR_CEILING],
            color_edge: vec![0u32; STAR_CEILING],
            color_corner: vec![0u32; STAR_CEILING],
            size: vec![0u8; STAR_CEILING],
            base_bright: vec![0u8; STAR_CEILING],
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

        let start = idx.saturating_sub(LANDMARK_STARS);
        for i in start..idx {
            sf.size[i] = 3;
            sf.base_bright[i] = 255;
            sf.color[i] = 0xFFFFFFFF;
            sf.base_color[i] = 0xFFFFFFFF;
        }

        sf.count = idx;
        sf.sort_by_size();
        sf.active = sf.count;
        sf.recompute_active_groups();
        sf
    }

    fn sort_by_size(&mut self) {
        let n = self.count;
        let mut perm: Vec<usize> = (0..n).collect();
        perm.sort_by_key(|&i| self.size[i]);

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

        self.groups = [n; 5];
        let mut current_size = 255u8;
        for i in 0..n {
            let s = self.size[i];
            if s != current_size {
                let from = if current_size == 255 { 0 } else { current_size as usize + 1 };
                for g in from..=s as usize {
                    if g < 5 { self.groups[g] = i; }
                }
                current_size = s;
            }
        }
        self.groups[4] = n;
        for g in (0..4).rev() {
            if self.groups[g] > self.groups[g + 1] {
                self.groups[g] = self.groups[g + 1];
            }
        }
    }

    pub fn recompute_active_groups(&mut self) {
        let a = self.active;
        for g in 0..5 {
            self.active_groups[g] = self.groups[g].min(a);
        }
        self.active_groups[4] = a.min(self.groups[4]);
    }

    pub fn adjust_count(&mut self, avg_frame_ms: f32) {
        if avg_frame_ms > BUDGET_MS && self.active > MIN_STARS {
            let overshoot = ((avg_frame_ms - BUDGET_MS) / BUDGET_MS * SHED_STEP as f32) as usize;
            let step = overshoot.max(SHED_STEP);
            self.active = self.active.saturating_sub(step).max(MIN_STARS);
            self.recompute_active_groups();
        } else if avg_frame_ms < GROW_MS && self.active < STAR_CEILING {
            // Grow: if we need more stars than currently spawned, spawn them
            let new_active = (self.active + GROW_STEP).min(STAR_CEILING);
            if new_active > self.count {
                self.spawn_extra(new_active - self.count);
            }
            self.active = new_active;
            self.recompute_active_groups();
        }
    }

    /// Spawn extra 1px stars at the end of the size-0 group.
    /// Inserts before the size-1 boundary and shifts groups up.
    fn spawn_extra(&mut self, n: usize) {
        let w = self.width;
        let h = self.height;
        // Use the far-layer color for extra stars (dim background dots)
        let base_color = LAYERS[0].color;
        let insert_at = self.count; // append at end (all size-0 are at the front)

        for j in 0..n {
            let idx = insert_at + j;
            if idx >= STAR_CEILING { break; }
            self.x[idx] = (self.rng.range(w as u32) as i32) << FP_SHIFT;
            self.y[idx] = (self.rng.range(h as u32) as i32) << FP_SHIFT;
            // Random speed in the range of layers 0-2
            let spd = fp_from_float(0.15 + (self.rng.range(100) as f32) * 0.01);
            self.speed_x[idx] = spd.max(1);
            self.speed_y[idx] = 0;
            let bright = 30 + self.rng.range(70) as u8;
            self.base_bright[idx] = bright;
            let c = dim_color(base_color, bright as u32);
            self.color[idx] = c;
            self.base_color[idx] = c;
            self.color_edge[idx] = 0;
            self.color_corner[idx] = 0;
            self.size[idx] = 0; // always 1px
        }
        let added = n.min(STAR_CEILING - self.count);
        self.count += added;
        // Extra stars are size-0, appended after the sorted block.
        // Update group[1..4] sentinel to include them (they stay at old positions),
        // and group[4] = new count.
        self.groups[4] = self.count;
        // groups[1] stays the same — extra 1px stars go after groups[1] boundary
        // but since they're appended at the end and are size-0, we need to
        // expand groups[1] to include them. Actually, groups[0]..groups[1] is
        // the size-0 range. We need groups[1] to shift right.
        // Simplest: just re-sort. This only runs when adapting (every ~0.25s).
        self.sort_by_size();
    }

    pub fn update(&mut self, dt: f32) {
        let count = self.active;
        let mx = self.max_x;
        let my = self.max_y;
        let dt_scale = dt / FRAME_DT;
        let dt_fp8 = (dt_scale * 256.0) as i32;

        let x = &mut self.x[..count];
        let sx = &self.speed_x[..count];
        for i in 0..count {
            let mut v = x[i] + (sx[i] >> 4) * (dt_fp8 >> 4);
            if v >= mx { v -= mx; }
            if v < 0   { v += mx; }
            x[i] = v;
        }

        let y = &mut self.y[..count];
        let sy = &self.speed_y[..count];
        for i in 0..count {
            let mut v = y[i] + (sy[i] >> 4) * (dt_fp8 >> 4);
            if v >= my { v -= my; }
            if v < 0   { v += my; }
            y[i] = v;
        }

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

    #[inline(never)]
    pub fn render(&self, pixels: &mut [u32]) {
        let w = self.width;
        let h = self.height;
        let groups = self.active_groups;

        for i in groups[0]..groups[1] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            unsafe { *pixels.get_unchecked_mut(py * w + px) = self.color[i]; }
        }

        for i in groups[1]..groups[2] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            let c = self.color[i];
            unsafe {
                *pixels.get_unchecked_mut(py * w + px) = c;
                if px + 1 < w { *pixels.get_unchecked_mut(py * w + px + 1) = c; }
                if py + 1 < h {
                    *pixels.get_unchecked_mut((py + 1) * w + px) = c;
                    if px + 1 < w { *pixels.get_unchecked_mut((py + 1) * w + px + 1) = c; }
                }
            }
        }

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
                        let pc = if dx == 1 && dy == 1 { c }
                                 else if dx != 1 && dy != 1 { dc_corner }
                                 else { dc_edge };
                        *pixels.get_unchecked_mut(sy * w + sx) = pc;
                    }
                }
            }
        }

        for i in groups[3]..groups[4] {
            let px = (self.x[i] >> FP_SHIFT) as usize;
            let py = (self.y[i] >> FP_SHIFT) as usize;
            if px >= w || py >= h { continue; }
            unsafe {
                *pixels.get_unchecked_mut(py * w + px) = 0xFFFFFFFF;
                for d in 0..3usize {
                    let fc = self.landmark.cross[d];
                    let d1 = d + 1;
                    if px + d1 < w { *pixels.get_unchecked_mut(py * w + px + d1) = fc; }
                    if px >= d1    { *pixels.get_unchecked_mut(py * w + px - d1) = fc; }
                    if py + d1 < h { *pixels.get_unchecked_mut((py + d1) * w + px) = fc; }
                    if py >= d1    { *pixels.get_unchecked_mut((py - d1) * w + px) = fc; }
                }
                let glow = self.landmark.glow;
                for dy in 0..3usize {
                    let sy = (py + dy).wrapping_sub(1);
                    if sy >= h { continue; }
                    for dx in 0..3usize {
                        let sx = (px + dx).wrapping_sub(1);
                        if sx >= w { continue; }
                        if dx == 1 && dy == 1 { continue; }
                        *pixels.get_unchecked_mut(sy * w + sx) = glow;
                    }
                }
            }
        }
    }
}

// ── Bitmap HUD ──────────────────────────────────────────────────────────────
pub const FONT3X5: [[u8; 5]; 10] = [
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

pub fn draw_num(pixels: &mut [u32], w: usize, x: usize, y: usize, val: u32, color: u32, scale: usize) -> usize {
    let mut digits = [0u8; 6];
    let mut n = val;
    let mut len = 0;
    if n == 0 { digits[0] = 0; len = 1; }
    else {
        while n > 0 && len < 6 { digits[len] = (n % 10) as u8; n /= 10; len += 1; }
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
                            if px < w && idx < pixels.len() { pixels[idx] = color; }
                        }
                    }
                }
            }
        }
    }
    len * 4 * scale
}

pub fn clear_rect(pixels: &mut [u32], w: usize, x: usize, y: usize, rw: usize, rh: usize) {
    for row in y..y + rh {
        if row >= pixels.len() / w { break; }
        let start = row * w + x;
        let end = (start + rw).min(pixels.len());
        for idx in start..end { pixels[idx] = 0; }
    }
}

const HUD_ENTRIES: usize = 3;

pub struct Hud {
    prev_vals: [u32; HUD_ENTRIES],
    fade: [u8; HUD_ENTRIES],
    base_colors: [u32; HUD_ENTRIES],
}

impl Hud {
    pub fn new() -> Self {
        Self {
            prev_vals: [u32::MAX; HUD_ENTRIES],
            fade: [255; HUD_ENTRIES],
            base_colors: [0xFF44FF44, 0xFF44AAAA, 0xFFFF4444],
        }
    }

    pub fn draw(&mut self, pixels: &mut [u32], w: usize, vals: [u32; HUD_ENTRIES]) {
        let y: usize = 10;
        let scale: usize = 2;
        let gap: usize = 3 * scale;
        let hud_h = 5 * scale + 4;
        let hud_w = 3 * 6 * 4 * scale + 2 * gap + 8;
        clear_rect(pixels, w, 8, if y > 1 { y - 1 } else { 0 }, hud_w, hud_h);

        let mut x = 10;
        for entry in 0..HUD_ENTRIES {
            let val = vals[entry];
            if val != self.prev_vals[entry] {
                self.fade[entry] = 255;
                self.prev_vals[entry] = val;
            } else {
                let floor: u8 = if entry == 2 && val == 0 { 15 } else { 50 };
                if self.fade[entry] > floor {
                    self.fade[entry] = self.fade[entry].saturating_sub(4);
                }
            }
            let color = dim_color(self.base_colors[entry], self.fade[entry] as u32);
            x += draw_num(pixels, w, x, y, val, color, scale) + gap;
        }
    }
}
