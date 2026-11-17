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

fn media_candidates() -> Result<Vec<PathBuf>, String> {
    let pet = crate::character::selected();
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let mut dirs = vec![assets::default_media_for(pet)?];
    if let Some(exe) = exe {
        dirs.extend([
            exe.join("media"),
            exe.join("../media"),
            exe.join("../../media"),
            exe.join("../..").join(pet).join("media"),
        ]);
    }
    dirs.extend([
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(crate::character::selected())
            .join("media"),
        PathBuf::from("media"),
    ]);
    Ok(dirs)
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
    let mut media_override = None;
    let mut prepare = false;
    let mut force = false;
    let mut scale = None;
    let mut pack_output = None;
    let mut pack_effort = 3;
    let mut check = false;
    let mut benchmark = None;
    let mut legacy_atlas = false;
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
            "--pack-assets" => {
                pack_output = Some(PathBuf::from(
                    args.next()
                        .ok_or("--pack-assets needs an output media directory")?,
                ))
            }
            "--pack-effort" => {
                pack_effort = args
                    .next()
                    .ok_or("--pack-effort needs 0..6")?
                    .parse::<i32>()
                    .map_err(|_| "invalid pack effort")?
            }
            "--check-assets" => check = true,
            "--legacy-atlas" => legacy_atlas = true,
            "--download-assets" => download_only = true,
            "--benchmark-playback" => {
                benchmark = Some(
                    args.next()
                        .ok_or("--benchmark-playback needs seconds (1..300)")?
                        .parse::<u64>()
                        .map_err(|_| "invalid benchmark duration")?,
                )
            }
            "--force" => force = true,
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
                    "{} [--media-dir DIR] [--download-assets | --prepare-assets [--force] | --pack-assets OUTPUT_MEDIA [--pack-effort 0..6] | --check-assets | --benchmark-playback SECONDS [--legacy-atlas]]\n{} --scale 50..150 (save and apply to a running pet)",
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
    if force && !prepare {
        return Err("--force requires --prepare-assets".into());
    }
    if legacy_atlas && benchmark.is_none() {
        return Err("--legacy-atlas is only for --benchmark-playback".into());
    }
    if u8::from(prepare)
        + u8::from(pack_output.is_some())
        + u8::from(check)
        + u8::from(benchmark.is_some())
        + u8::from(download_only)
        > 1
    {
        return Err("prepare, pack, check and benchmark are mutually exclusive".into());
    }
    let media = match media_override {
        Some(media) => media,
        None => media_candidates()?
            .into_iter()
            .find(|d| {
                d.join("anim").is_dir()
                    || d.join("packed/library.json").is_file()
                    || d.join("resources.json").is_file()
            })
            .unwrap_or(assets::default_media()?),
    };
    let media = assets::ensure(&media)?;
    if download_only {
        println!("动画资源已就绪：{}", media.display());
        return Ok(());
    }
    if prepare {
        return gfx::prepare_assets(&media, force);
    }
    if let Some(output) = pack_output {
        return gfx::pack_assets(&media, &output, pack_effort);
    }
    if check {
        return gfx::check_assets(&media);
    }
    if let Some(seconds) = benchmark {
        return gfx::benchmark_playback(&media, seconds, legacy_atlas);
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
