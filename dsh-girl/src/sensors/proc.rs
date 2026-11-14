// Shared config loading.
// The loader reads $XDG_CONFIG_HOME/desktop-pet/desktop-pet.json.

use serde_json::Value;
use std::path::PathBuf;

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
