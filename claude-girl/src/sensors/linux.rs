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
    use super::is_cli_focus;

    #[test]
    fn recognizes_the_ai_clis() {
        assert!(is_cli_focus("claude: Claude Code", "kitty"));
        assert!(is_cli_focus("kitty", "claude"));
        assert!(is_cli_focus("OC | Greeting", "kitty"), "opencode titles its terminal 'OC | …'");
        assert!(is_cli_focus("refactor", "Dev.sandbox-opencode"));
        // Codex thread titles may mention Codex; native app ids also match
        assert!(is_cli_focus("Codex - any thread", "kitty"));
        assert!(is_cli_focus("any title", "codex"));
        assert!(!is_cli_focus("zsh in adarsh", "kitty"));
        assert!(!is_cli_focus("logos", "dev.duxlabs.files"));
    }
}

/// Focused-window rule: title/app identifies the AI CLI. The claude CLI
/// sets 'Claude Code' in titles; opencode titles terminals 'OC | …'; Codex
/// keeps the thread name in the title, so only threads that mention Codex
/// are recognized (a native Codex window matches on app id).
fn is_cli_focus(title: &str, app: &str) -> bool {
    title.contains("Claude Code")
        || title.contains("OpenCode")
        || title.starts_with("OC |")
        || title.contains("Codex")
        || title.contains("codex")
        || app == "claude"
        || app.contains("opencode")
        || app.contains("codex")
}

pub fn poll_system_state(focus_cache: &SharedCache) -> DerivedState {
    let cli_count = count_paths("/proc", "claude") as u32
        + count_paths("/proc", "opencode") as u32
        + count_paths("/proc", "codex") as u32;
    let spotify_playing = mpris::read().playing;
    let focused = focus_cache.lock().unwrap().window.clone();
    let is_cli_focused = is_cli_focus(&focused.title, &focused.app);

    let (battery_pct, is_charging) = battery::read()
        .map(|b| (u32::from(b.percent), b.charging))
        .unwrap_or((100, true));

    DerivedState {
        mode: if is_cli_focused {
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
