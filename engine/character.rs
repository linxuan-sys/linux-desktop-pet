use std::sync::OnceLock;

pub const PETS: &[&str] = &[
    "claude-girl",
    "dsh-girl",
    "codex-girl",
    "grok-girl",
    "qwen-girl",
    "gemini-girl",
    "glm-girl",
    "kimi-girl",
];
static SELECTED: OnceLock<&'static str> = OnceLock::new();

pub fn select(value: &str) -> Result<&'static str, String> {
    let value = value.strip_suffix("-girl").unwrap_or(value);
    let name = PETS
        .iter()
        .copied()
        .find(|name| name.strip_suffix("-girl") == Some(value))
        .ok_or_else(|| {
            format!(
                "unknown pet {value}; choose claude, dsh, codex, grok, qwen, gemini, glm or kimi"
            )
        })?;
    if let Some(current) = SELECTED.get() {
        return (*current == name)
            .then_some(name)
            .ok_or_else(|| "pet already selected".into());
    }
    SELECTED.set(name).map_err(|_| "pet already selected")?;
    Ok(name)
}

pub fn selected() -> &'static str {
    SELECTED.get().copied().unwrap_or(env!("CARGO_PKG_NAME"))
}

#[path = "poses.rs"]
mod poses;
pub use poses::animation_for;
pub fn pose_anim(pose: &str) -> Option<&'static str> {
    animation_for(selected(), pose)
}

pub const DEFAULT_PREMULTIPLY: bool = true;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_resolve_without_changing_identity() {
        let selected = *SELECTED.get_or_init(|| "kimi-girl");
        assert_eq!(
            select(selected.strip_suffix("-girl").unwrap()).unwrap(),
            selected
        );
        assert_eq!(select(selected).unwrap(), selected);
        assert!(select("not-a-pet").is_err());
    }
}
