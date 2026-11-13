// Native pet entry: sensors feed the pose engine, the pose engine feeds the
// window host. Sensor threads push Signals over one channel; the main loop
// owns the host, pet and clock.

pub mod host;
mod gfx;
pub mod pet;
mod sensors;

use gfx::{blit_scale, Canvas, Sheet, ART_SHIFT, PAD};
use host::AppEvent;
use pet::{Pet, HAPPY_SKIP_MS};
use sensors::proc::Config;
use sensors::state::DerivedState;
use chrono::Timelike;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

const SYSTEM_POLL_MS: u64 = 2_500;
const TICK_MS: u64 = 33;

/// First candidate dir that contains the animation library.
fn find_media_dir(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|d| d.join("anim").is_dir()).cloned()
}

fn media_candidates() -> Vec<PathBuf> {
    let xdg_data = match std::env::var("XDG_DATA_HOME") {
        Ok(v) if !v.is_empty() => PathBuf::from(v).join("dsh-girl/media"),
        _ => PathBuf::from(
            std::env::var("HOME").map(|h| h + "/.local/share/dsh-girl/media").unwrap_or_default(),
        ),
    };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    [
        Some(xdg_data),
        exe_dir.as_ref().map(|d| d.join("media")),
        exe_dir.as_ref().map(|d| d.join("..").join("media")),
        // `cargo build` tree: target/release/../.. is the project root
        exe_dir.as_ref().map(|d| d.join("../..").join("media")),
        Some(PathBuf::from("media")),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// What the sensor threads push toward the main loop.
enum Signal {
    State(DerivedState),
    Clipboard,
}

fn main() {
    let cfg = Config::load();
    let media = find_media_dir(&media_candidates()).unwrap_or_else(|| {
        eprintln!("dsh-girl: no media/anim directory next to the exe, in XDG data, or cwd");
        std::process::exit(1);
    });
    let mut sheet = Sheet::load(&media);
    if sheet.is_empty() {
        eprintln!("dsh-girl: no .webp animations in {}", media.join("anim").display());
        std::process::exit(1);
    }
    // pin the click reaction now so the first click does not decode
    let _ = sheet.frame_at("happy", 0, false);
    let (tx, rx) = channel::<Signal>();
    spawn_sensor_threads(&tx);
    if let Err(e) = run(cfg, &mut sheet, rx) {
        eprintln!("dsh-girl: host failed: {e}");
        std::process::exit(1);
    }
}

fn spawn_sensor_threads(tx: &Sender<Signal>) {
    // system state + focus event stream (2.5s cadence)
    let tx_state = tx.clone();
    let system_ms = Duration::from_millis(SYSTEM_POLL_MS);
    std::thread::spawn(move || {
        let cache = sensors::focus::new_cache();
        let c2 = cache.clone();
        std::thread::spawn(move || {
            sensors::focus::run_forever(sensors::focus::detect, &c2, Duration::from_secs(3));
        });
        loop {
            let state = sensors::poll_system_state(&cache);
            if tx_state.send(Signal::State(state)).is_err() {
                break;
            }
            std::thread::sleep(system_ms);
        }
    });

    // clipboard watcher: Wayland's copy reaction source
    let tx_clip = tx.clone();
    std::thread::spawn(move || {
        sensors::clipboard::watch(move |kind| {
            if matches!(
                kind,
                sensors::clipboard::ClipboardKind::Text | sensors::clipboard::ClipboardKind::Image
            ) {
                let _ = tx_clip.send(Signal::Clipboard);
            }
        });
    });
}

fn run(cfg: Config, sheet: &mut Sheet, rx: Receiver<Signal>) -> Result<(), String> {
    let mut host = host::open()?;
    eprintln!("dsh-girl: host ready");
    let mut pet = Pet::new();
    let (fw, fh) = host.size();

    let start = Instant::now();
    // clicks that spawn something debounce each other; a click burst must
    // not open a terminal swarm
    let mut last_spawn_ms: u64 = 0;

    // sensors haven't answered yet; match upstream's power-on defaults
    let mut last_state = DerivedState {
        mode: "awake".into(),
        spotify_playing: false,
        hour: chrono::Local::now().hour() as u32,
        battery_pct: 100,
        is_charging: true,
    };
    pet.apply(&last_state);

    loop {
        // 1: drain sensor signals — latest state wins, reactions accumulate
        loop {
            match rx.try_recv() {
                Ok(Signal::State(st)) => {
                    last_state = st;
                    pet.apply(&last_state);
                }
                Ok(Signal::Clipboard) => pet.on_copy(start.elapsed().as_millis() as u64),
                Err(_) => break,
            }
        }

        // 2: host input
        for ev in host.poll_events() {
            match ev {
                AppEvent::Click(btn) => {
                    pet.on_click(start.elapsed().as_millis() as u64);
                    // per-button command from config; "" (default left) is
                    // a pure tease that spawns nothing
                    let now_ms = start.elapsed().as_millis() as u64;
                    let cmd = &cfg.click[btn.index()];
                    if !cmd.is_empty() && now_ms.saturating_sub(last_spawn_ms) >= 1200 {
                        last_spawn_ms = now_ms;
                        spawn_command(cmd);
                    }
                }
                AppEvent::Dragged => pet.on_drag(),
            }
        }

        // walk + pose this frame (position lives in the host's margins)
        let now = start.elapsed().as_millis() as u64;
        let (px, py) = host.position();
        pet.sync_pos(px, py);
        let max_x = host.max_x();
        pet.set_max_x(max_x);
        let pose = pet.tick(now);
        if pet.walking() {
            let (x, y) = pet.pos();
            let _ = host.move_to(x, y);
        }

        // 3: draw the current animation frame
        if let Some(name) = pose {
            let clip_start = pet.clip_start();
            // the click hop starts at its 3.5s mark
            let skip = if name == "happy" { HAPPY_SKIP_MS } else { 0 };
            // the right-run art is the left-run art mirrored
            let mirror = name == "walk-right";
            if let Some(frame) = sheet.frame_at(name, now.saturating_sub(clip_start) + skip, mirror) {
                let mut canvas = Canvas::new(fw, fh);
                // The art is padded on both sides. Slide it toward the nearer
                // screen edge: +ART_SHIFT at the right perch, -ART_SHIFT at the
                // left wall, linear in between, so both gaps match.
                let art_shift = if max_x > 0 {
                    (ART_SHIFT * (2 * pet.pos().0 - max_x) / max_x)
                        .clamp(-ART_SHIFT, ART_SHIFT)
                } else {
                    ART_SHIFT
                };
                blit_scale(&mut canvas, &frame, PAD as i32 + art_shift, PAD as i32, 1);
                host.present(&canvas);
            }
        }

        std::thread::sleep(Duration::from_millis(TICK_MS));
    }
}

fn spawn_command(command: &str) {
    let mut parts = command.split_whitespace();
    let Some(bin) = parts.next() else { return };
    let _ = std::process::Command::new(bin).args(&mut parts).spawn();
}
