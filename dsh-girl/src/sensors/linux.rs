// Linux sensor impls: /proc-based CLI detection, real config loading,
// compositor focus via the niri event-stream.

use super::battery;
use super::focus::SharedCache;
use super::mpris;
use super::state::DerivedState;
use super::proc::count_paths;
use chrono::Timelike;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_terminals() {
        assert!(is_terminal_focus("kitty"));
        assert!(is_terminal_focus("com.mitchellh.ghostty"));
        assert!(is_terminal_focus("org.gnome.Terminal"));
        assert!(is_terminal_focus("org.kde.konsole"));
        assert!(is_terminal_focus("Alacritty"));
        assert!(is_terminal_focus("org.wezfurlong.wezterm"));
        assert!(is_terminal_focus("foot"));
        assert!(is_terminal_focus("st-256color"));
        assert!(!is_terminal_focus("org.gnome.Nautilus"));
        assert!(!is_terminal_focus("wechat"));
    }
}

/// No generic Wayland "terminal" app id; "term" catches most, plus the
/// terminals whose ids do not contain it.
fn is_terminal_focus(app: &str) -> bool {
    let app = app.to_ascii_lowercase();
    const TERMINALS: &[&str] = &[
        "term", "kitty", "ghostty", "foot", "alacritty", "konsole", "tilix",
        "guake", "yakuake", "tilda", "sakura", "rxvt", "ptyxis", "blackbox",
        "kgx", "gnome.console", "contour", "tabby", "hyper", "dev.warp.warp",
    ];
    app == "st"
        || app == "st-256color"
        || app == "rio"
        || app == "warp"
        || TERMINALS.iter().any(|t| app.contains(t))
}

pub fn poll_system_state(focus_cache: &SharedCache) -> DerivedState {
    let cli_count = count_paths("/proc", "claude") as u32
        + count_paths("/proc", "opencode") as u32
        + count_paths("/proc", "codex") as u32;
    let spotify_playing = mpris::read().playing;
    let focused = focus_cache.lock().unwrap().window.clone();
    let is_terminal = is_terminal_focus(&focused.app);

    let (battery_pct, is_charging) = battery::read()
        .map(|b| (u32::from(b.percent), b.charging))
        .unwrap_or((100, true));

    DerivedState {
        mode: if is_terminal {
            "action".to_string()
        } else if cli_count > 0 {
            "awake".to_string()
        } else {
            "asleep".to_string()
        },
        spotify_playing,
        hour: chrono::Local::now().hour(),
        battery_pct,
        is_charging,
    }
}

#[cfg(test)]
mod live_probe {
    use super::*;
    use crate::sensors::focus;

    /// Prints the derived state twice a second for 5s against the live
    /// session; shows what the pet's main loop actually receives.
    #[test]
    #[ignore = "live: prints poll_system_state against the real session"]
    fn probe_derived_state() {
        let cache = focus::new_cache();
        let c2 = cache.clone();
        std::thread::spawn(move || {
            focus::run_forever(focus::detect, &c2, std::time::Duration::from_secs(3));
        });
        for i in 0..10 {
            let st = poll_system_state(&cache);
            eprintln!("probe[{}] mode={}", i, st.mode);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}
