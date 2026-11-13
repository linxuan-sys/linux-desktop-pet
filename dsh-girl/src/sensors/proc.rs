// /proc-based AI CLI detection + shared config loading.
// pet detection scans /proc/*/comm for AI CLI processes; the loader reads
// the shared $XDG_CONFIG_HOME/desktop-pet/desktop-pet.json.

use serde_json::Value;
use std::path::PathBuf;

/// Counts processes whose /proc/<pid>/comm matches `comm`.
/// Tolerates pids appearing/disappearing mid-scan (skips unreadable entries).
pub fn count_paths(dir: &str, comm: &str) -> usize {
    let mut count = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(entry.path().join("comm")) {
            if text.trim() == comm {
                count += 1;
            }
        }
    }
    count
}

/// The pet's click commands.
pub struct Config {
    /// Command run by a stationary click, per button: [left, middle, right].
    /// "" = tease the pet only, spawn nothing.
    pub click: [String; 3],
}

impl Config {
    fn default_click(pet: &str) -> [String; 3] {
        let right = match pet {
            "claude-girl" => "kitty claude",
            "dsh-girl" => "kitty dsh-tui",
            "codex-girl" => "kitty codex",
            _ => "",
        };
        [String::new(), "kitty".to_owned(), right.to_owned()]
    }

    pub fn parse(raw: &Value, pet: &str) -> Config {
        let fallback = Self::default_click(pet);
        let click_cfg = raw
            .get("pets")
            .and_then(|pets| pets.get(pet))
            .and_then(|section| section.get("click"));
        let pick = |key: &str, index: usize| -> String {
            click_cfg
                .and_then(|c| c.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| fallback[index].clone())
        };
        let click = [pick("left", 0), pick("middle", 1), pick("right", 2)];
        Config { click }
    }

    /// One file for all three pets:
    /// `$XDG_CONFIG_HOME/desktop-pet/desktop-pet.json`.
    /// Missing or malformed file falls back to the built-in defaults.
    pub fn load() -> Config {
        let xdg = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(h).join(".config"))
                    .unwrap_or_default()
            });
        let path = xdg.join("desktop-pet").join("desktop-pet.json");
        let raw = std::fs::read_to_string(path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        Config::parse(&raw, env!("CARGO_PKG_NAME"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_one_claude_among_decoys() {
        let dir = std::env::temp_dir().join("dsh-girl-proc-test");
        let _ = std::fs::remove_dir_all(&dir);
        for (pid, comm) in [
            ("100", "claude"),
            ("101", "kitty"),
            ("102", "claude"),
        ] {
            std::fs::create_dir_all(dir.join(pid)).unwrap();
            std::fs::write(dir.join(pid).join("comm"), format!("{comm}\n")).unwrap();
        }
        std::fs::write(dir.join("acpi"), "\n").unwrap();
        assert_eq!(count_paths(dir.to_str().unwrap(), "claude"), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_dir_is_zero() {
        assert_eq!(count_paths("/nonexistent-dsh-girl-test", "claude"), 0);
    }

    #[test]
    fn defaults_are_per_pet() {
        let claude = Config::parse(&serde_json::json!({}), "claude-girl");
        assert_eq!(claude.click, ["", "kitty", "kitty claude"]);
        let dsh = Config::parse(&serde_json::json!({}), "dsh-girl");
        assert_eq!(dsh.click, ["", "kitty", "kitty dsh-tui"]);
        let codex = Config::parse(&serde_json::json!({}), "codex-girl");
        assert_eq!(codex.click, ["", "kitty", "kitty codex"]);
    }

    #[test]
    fn one_section_overrides_only_that_pet() {
        let raw = serde_json::json!({
            "pets": {
                "claude-girl": {
                    "click": { "middle": "foot", "right": "kitty claude --resume" }
                }
            }
        });
        let claude = Config::parse(&raw, "claude-girl");
        assert_eq!(claude.click, ["", "foot", "kitty claude --resume"]);
        let dsh = Config::parse(&raw, "dsh-girl");
        assert_eq!(dsh.click, ["", "kitty", "kitty dsh-tui"]);
    }
}
