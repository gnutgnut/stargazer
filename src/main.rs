mod starfield;

use minifb::{Key, Window, WindowOptions};
use std::time::{Duration, Instant};
use std::thread;
use std::io::Write;
use std::fs::File;
use starfield::*;

const WIDTH: usize = 1280;
const HEIGHT: usize = 720;
const FRAME_DURATION: Duration = Duration::from_micros(16_667);

fn main() {
    let logging = std::env::args().any(|a| a == "--log");

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
    let mut hud = Hud::new();

    let mut fps: u32 = 0;
    let mut frame_count: u32 = 0;
    let mut drop_count: u32 = 0;
    let mut total_drops: u32 = 0;
    let mut reported_drops: u32 = 0;
    let mut fps_timer = Instant::now();
    let mut frame_start = Instant::now();
    let mut adjust_counter: u32 = 0;
    let mut work_ms_accum: f32 = 0.0;
    let mut frame_num: u64 = 0;
    let mut log: Option<File> = if logging {
        let mut f = File::create("stargazer.log").expect("Failed to create log file");
        writeln!(f, "frame,time_ms,dt_ms,active,fps,drops,adjusted").unwrap();
        Some(f)
    } else {
        None
    };
    let app_start = Instant::now();

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
        pixels.fill(0);
        sf.render(&mut pixels);
        hud.draw(&mut pixels, WIDTH, [fps, sf.active as u32, drop_count]);

        window.update_with_buffer(&pixels, WIDTH, HEIGHT)
            .expect("Failed to update window");

        let total_frame_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        work_ms_accum += total_frame_ms;

        frame_count += 1;
        if raw_dt > FRAME_DT * 1.3 {
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

        adjust_counter += 1;
        let mut adjusted = false;
        if adjust_counter >= ADJUST_INTERVAL {
            let avg_frame = work_ms_accum / adjust_counter as f32;
            let before = sf.active;
            adjusted = sf.adjust_count(avg_frame);
            if adjusted {
                eprintln!("adapt: avg={:.1}ms  {} -> {} stars  count={}  groups={:?}  active_groups={:?}",
                    avg_frame, before, sf.active, sf.count, sf.groups, sf.active_groups);
            }
            work_ms_accum = 0.0;
            adjust_counter = 0;
        }

        frame_num += 1;
        if let Some(ref mut f) = log {
            if frame_num <= 60000 {
                let _ = writeln!(f, "{},{:.2},{:.2},{},{},{},{}",
                    frame_num,
                    app_start.elapsed().as_secs_f32() * 1000.0,
                    raw_dt * 1000.0,
                    sf.active, fps, total_drops,
                    if adjusted { 1 } else { 0 },
                );
                if frame_num == 60000 {
                    let _ = writeln!(f, "# log capped at 60000 frames");
                }
            }
        }

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
