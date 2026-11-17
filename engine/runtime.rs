//! Main loop: sensors/input remain responsive while animation time waits for assets.
use crate::assets;
use crate::{gfx, host, pet, sensors, settings};
use chrono::Timelike;
use gfx::{ART_SHIFT, Canvas, Sheet, WINDOW_H, WINDOW_W};
use host::AppEvent;
use pet::{Pet, happy_skip_ms};
use sensors::state::DerivedState;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
const SYSTEM_POLL_MS: u64 = 2_500;
const TICK_MS: u64 = 16;
enum Signal {
    State(DerivedState),
    Clipboard,
}

pub fn main() {
    let result = crate::character::select(env!("CARGO_PKG_NAME"))
        .and_then(|_| entry(&std::env::args().skip(1).collect::<Vec<_>>()));
    if let Err(e) = result {
        eprintln!("{}: {e}", crate::character::selected());
        std::process::exit(1);
    }
}
pub fn entry(arguments: &[String]) -> Result<(), String> {
    #[cfg(feature = "unified")]
    set_process_name();

    let mut media_override = None;
    let mut prepare = false;
    let mut scale = None;
    let mut check = false;
    let mut benchmark = None;
    let mut download_only = false;
    let mut args = arguments.iter().cloned();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--media-dir" => {
                media_override = Some(PathBuf::from(
                    args.next().ok_or("--media-dir needs a path")?,
                ))
            }
            "--prepare-assets" => prepare = true,
            "--check-assets" => check = true,
            "--download-assets" => download_only = true,
            "--benchmark-playback" => {
                benchmark = Some(
                    args.next()
                        .ok_or("--benchmark-playback needs seconds (1..300)")?
                        .parse::<u64>()
                        .map_err(|_| "invalid benchmark duration")?,
                )
            }
            "--scale" => {
                scale = Some(
                    args.next()
                        .ok_or("--scale needs a percentage (50..150)")?
                        .parse::<f64>()
                        .map_err(|_| "invalid percentage")?
                        / 100.0,
                )
            }
            "--help" | "-h" => {
                println!(
                    "{} [--media-dir DIR] [--download-assets | --prepare-assets | --check-assets | --benchmark-playback SECONDS]\n{} --scale 50..150 (save and apply to a running pet)",
                    crate::character::selected(),
                    crate::character::selected()
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}; see --help")),
        }
    }
    if let Some(scale) = scale {
        settings::set_scale(scale)?;
        println!("size saved: {:.0}%", scale * 100.0);
        return Ok(());
    }
    if u8::from(prepare) + u8::from(check) + u8::from(benchmark.is_some()) + u8::from(download_only)
        > 1
    {
        return Err("prepare, check, benchmark and download are mutually exclusive".into());
    }
    let media =
        assets::prepare_media_for(crate::character::selected(), media_override.as_deref())?.path;
    if download_only {
        println!("动画资源已就绪：{}", media.display());
        return Ok(());
    }
    if prepare || check {
        return gfx::check_assets(&media);
    }
    if let Some(seconds) = benchmark {
        return gfx::benchmark_playback(&media, seconds);
    }
    #[cfg(feature = "unified")]
    let _instance = crate::control::InstanceGuard::acquire()?;
    let watcher = settings::Watcher::new()?;
    let mut sheet = Sheet::load(&media, watcher.current.cache_mib)?;
    sheet.set_premultiply(watcher.current.premultiply);
    let (tx, rx) = channel();
    spawn_sensor_threads(&tx);
    run(watcher, sheet, rx)
}

#[cfg(feature = "unified")]
fn process_name_for(pet: &str) -> String {
    let short = pet.strip_suffix("-girl").unwrap_or(pet);
    format!("pet-{short}")
}

#[cfg(feature = "unified")]
fn set_process_name() {
    let name = process_name_for(crate::character::selected());
    let Ok(name) = std::ffi::CString::new(name) else {
        return;
    };
    if unsafe { libc::prctl(libc::PR_SET_NAME, name.as_ptr(), 0, 0, 0) } != 0 {
        eprintln!("set process name: {}", std::io::Error::last_os_error());
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

fn run(
    mut config: settings::Watcher,
    mut sheet: Sheet,
    rx: Receiver<Signal>,
) -> Result<(), String> {
    let mut host = host::open(&config.current)?;
    eprintln!(
        "{}: host ready; cache budget {} MiB",
        crate::character::selected(),
        config.current.cache_mib
    );
    let mut pet = Pet::new();
    pet.apply(&DerivedState {
        mode: "awake".into(),
        spotify_playing: false,
        hour: chrono::Local::now().hour(),
        battery_pct: 100,
        is_charging: true,
    });
    let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
    let start = Instant::now();
    let mut previous = start;
    let mut config_at = start;
    let mut animation_time = Duration::ZERO;
    let mut playing = true;
    let mut dirty = false;
    let mut last_spawn: Option<Instant> = None;
    let mut children = Vec::<std::process::Child>::new();
    // Decode only the first requested pose; do not load unused source sheets.
    loop {
        let current = Instant::now();
        // Dragging no longer freezes the clock: the drag itself drives a
        // clip, so the animation must keep advancing.
        if playing {
            animation_time += current.duration_since(previous);
        }
        previous = current;
        let now = animation_time.as_millis() as u64;
        while let Ok(signal) = rx.try_recv() {
            match signal {
                Signal::State(s) => pet.apply(&s),
                Signal::Clipboard => pet.on_copy(now),
            }
        }
        if config_at.elapsed() >= Duration::from_millis(500) {
            config_at = current;
            let old = config.current.clone();
            if config.poll() {
                if old.output != config.current.output {
                    host = host::open(&config.current)?;
                    pet.on_drag_end();
                } else if old.scale != config.current.scale {
                    host.set_scale(config.current.scale);
                    pet.on_drag_end();
                    if let Err(e) = host.placement().save() {
                        eprintln!("save placement: {e}");
                    }
                }
                sheet.set_budget(config.current.cache_mib);
                sheet.set_premultiply(config.current.premultiply);
                sheet.invalidate();
                dirty = true;
            }
        }
        let mut reopen = false;
        let was_dragging = host.dragging();
        for event in host.poll_events() {
            match event {
                AppEvent::Click(button) => {
                    pet.on_click(now);
                    let command = &config.current.click[button.index()];
                    if !command.is_empty()
                        && last_spawn.is_none_or(|t| t.elapsed() >= Duration::from_millis(1200))
                    {
                        last_spawn = Some(current);
                        let mut parts = command.split_whitespace();
                        if let Some(bin) = parts.next() {
                            match std::process::Command::new(bin).args(parts).spawn() {
                                Ok(child) => children.push(child),
                                Err(e) => eprintln!("click command {bin}: {e}"),
                            }
                        }
                    }
                }
                AppEvent::Dragged => {
                    if let Err(e) = host.placement().save() {
                        eprintln!("save placement: {e}");
                    }
                }
                AppEvent::GeometryChanged => {
                    dirty = true;
                }
                AppEvent::Closed => reopen = true,
                AppEvent::Disconnected => return Err("Wayland connection lost".into()),
            }
        }
        // AppEvent::Dragged only fires on release, so drive the drag clip
        // from the pointer-down edge instead.
        let now_dragging = host.dragging();
        if now_dragging != was_dragging {
            if now_dragging {
                pet.on_drag();
            } else {
                pet.on_drag_end();
            }
            dirty = true;
        }
        if reopen {
            host = host::open(&config.current)?;
            pet.on_drag_end();
            dirty = true;
        }
        children.retain_mut(|c| matches!(c.try_wait(), Ok(None)));
        let (x, y) = host.position();
        pet.sync_pos(x, y);
        let max_x = host.max_x();
        pet.set_max_x(max_x);
        if let Some(pose) = pet.tick(now) {
            playing = sheet.ready(pose);
            if playing {
                if pet.walking() && !host.dragging() {
                    let (x, y) = pet.pos();
                    host.move_to(x, y)?;
                }
                let shift = if max_x > 0 {
                    (ART_SHIFT * (2 * pet.pos().0 - max_x) / max_x).clamp(-ART_SHIFT, ART_SHIFT)
                } else {
                    ART_SHIFT
                };
                let skip = if pose == "happy" { happy_skip_ms() } else { 0 };
                dirty |= sheet.draw(
                    pose,
                    now.saturating_sub(pet.clip_start()) + skip,
                    pose == "walk-right",
                    shift,
                    &mut canvas,
                );
            }
        }
        if dirty && host.present(&canvas)? {
            dirty = false;
        }
        std::thread::sleep(Duration::from_millis(TICK_MS));
    }
}

#[cfg(all(test, feature = "unified"))]
mod tests {
    use super::*;

    #[test]
    fn process_name_uses_the_short_character_name() {
        assert_eq!(process_name_for("claude-girl"), "pet-claude");
        assert_eq!(process_name_for("dsh-girl"), "pet-dsh");
        assert_eq!(process_name_for("codex-girl"), "pet-codex");
    }
}
