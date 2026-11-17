//! User preferences and persisted placement are separate, atomically written files.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Root containing one data directory per character. Keeping this global
    /// lets downloads and the runtime resolve the same location.
    pub assets_root: PathBuf,
    pub scale: f64,
    pub cache_mib: usize,
    pub output: Option<String>,
    pub click: [String; 3],
    /// Premultiply alpha before writing to the shared buffer. Wayland's
    /// Argb8888 is a premultiplied format, so `true` is the correct default;
    /// `false` keeps the authored colours exactly and is useful when the
    /// artwork's antialiased edges carry stray hues.
    pub premultiply: bool,
}
impl Settings {
    pub fn parse(raw: &Value) -> Result<Self, String> {
        Self::parse_for(raw, crate::character::selected())
    }
    pub fn parse_for(raw: &Value, pet: &str) -> Result<Self, String> {
        if !raw.is_object() || raw.get("pets").is_some_and(|v| !v.is_object()) {
            return Err("configuration root and pets must be objects".into());
        }
        if raw["pets"].get(pet).is_some_and(|v| !v.is_object()) {
            return Err("pet configuration must be an object".into());
        }
        let assets_root = parse_assets_root(raw.get("assets_root"))?;
        let section = &raw["pets"][pet];
        let scale = match section.get("scale") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|v| v.is_finite() && (0.5..=1.5).contains(v))
                .ok_or("scale must be a number between 0.5 and 1.5")?,
        };
        let cache_mib = match section.get("cache_mib") {
            None => 32,
            Some(v) => v
                .as_u64()
                .filter(|v| (16..=512).contains(v))
                .ok_or("cache_mib must be an integer between 16 and 512")?
                as usize,
        };
        let output = match section.get("output") {
            None | Some(Value::Null) => None,
            Some(Value::String(v)) if !v.is_empty() => Some(v.clone()),
            _ => return Err("output must be a nonempty output name or null".into()),
        };
        let premultiply = match section.get("premultiply") {
            None => crate::character::DEFAULT_PREMULTIPLY,
            Some(v) => v.as_bool().ok_or("premultiply must be true or false")?,
        };
        if let Some(click) = section.get("click") {
            if !click.is_object() {
                return Err("click must be an object".into());
            }
            for key in ["left", "middle", "right"] {
                if click.get(key).is_some_and(|v| !v.is_string()) {
                    return Err(format!("click.{key} must be a string"));
                }
            }
        }
        Ok(Self {
            assets_root,
            scale,
            cache_mib,
            output,
            premultiply,
            click: crate::sensors::proc::Config::parse(raw, pet).click,
        })
    }
}
fn xdg(key: &str, fallback: &str) -> PathBuf {
    std::env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
}
pub fn default_assets_root() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        return PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest);
    }
    PathBuf::from(value)
}

fn parse_assets_root(value: Option<&Value>) -> Result<PathBuf, String> {
    match value {
        None | Some(Value::Null) => Ok(default_assets_root()),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            let path = expand_home(value.trim());
            if !path.is_absolute() {
                return Err("assets_root must be an absolute path or start with ~/".into());
            }
            Ok(path)
        }
        _ => Err("assets_root must be a nonempty absolute path string or null".into()),
    }
}

pub fn configured_assets_root() -> Result<PathBuf, String> {
    let path = config_path();
    let raw: Value = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("{}: {error}", path.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Value::Null,
        Err(error) => return Err(error.to_string()),
    };
    if !raw.is_object() && !raw.is_null() {
        return Err("configuration root must be an object".into());
    }
    parse_assets_root(raw.get("assets_root"))
}

#[cfg(feature = "unified")]
pub fn resolve_assets_root_input(value: &str) -> Result<PathBuf, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("素材目录不能为空".into());
    }
    let path = expand_home(value);
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("无法创建素材目录 {}: {error}", path.display()))?;
    path.canonicalize().map_err(|error| error.to_string())
}

#[cfg(feature = "unified")]
pub fn set_assets_root(value: &str, reset: bool) -> Result<PathBuf, String> {
    if reset {
        update_for(crate::character::selected(), |raw| {
            raw["assets_root"] = Value::Null
        })?;
        return Ok(default_assets_root());
    }
    let root = resolve_assets_root_input(value)?;
    let saved = root.to_string_lossy().into_owned();
    update_for(crate::character::selected(), |raw| {
        raw["assets_root"] = Value::String(saved)
    })?;
    Ok(root)
}

pub fn config_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("pet/pet.json")
}
pub fn state_path() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
        .join("pet")
        .join(format!("{}.json", crate::character::selected()))
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("missing parent")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    serde_json::to_writer_pretty(tmp.as_file_mut(), value).map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn update(mutate: impl FnOnce(&mut Value)) -> Result<(), String> {
    update_for(crate::character::selected(), mutate)
}

pub fn update_for(pet: &str, mutate: impl FnOnce(&mut Value)) -> Result<(), String> {
    let path = config_path();
    let parent = path.parent().ok_or("missing config directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".pet.lock"))
        .map_err(|error| error.to_string())?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|error| error.to_string())?;
    let mut raw: Value = match std::fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.to_string()),
    };
    if !raw.is_object() {
        return Err("configuration root must be an object".into());
    }
    if raw.get("pets").is_some_and(|v| !v.is_object()) {
        return Err("pets must be an object".into());
    }
    if raw["pets"].get(pet).is_some_and(|v| !v.is_object()) {
        return Err("pet configuration must be an object".into());
    }
    mutate(&mut raw);
    Settings::parse_for(&raw, pet)?;
    atomic_json(&path, &raw)
}

pub fn set_scale(scale: f64) -> Result<(), String> {
    update(|raw| raw["pets"][crate::character::selected()]["scale"] = serde_json::json!(scale))
}

pub struct Watcher {
    path: PathBuf,
    previous: Option<Vec<u8>>,
    pub current: Settings,
}
impl Watcher {
    pub fn new() -> Result<Self, String> {
        let path = config_path();
        let previous = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let raw = match &previous {
            Some(bytes) => {
                serde_json::from_slice(bytes).map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => serde_json::json!({}),
        };
        let current = Settings::parse(&raw)?;
        Ok(Self {
            path,
            previous,
            current,
        })
    }
    pub fn poll(&mut self) -> bool {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                eprintln!("config read: {e}");
                return false;
            }
        };
        if self.previous == bytes {
            return false;
        }
        self.previous = bytes;
        let result = self
            .previous
            .as_ref()
            .map_or(Ok(serde_json::json!({})), |b| {
                serde_json::from_slice(b).map_err(|e| e.to_string())
            })
            .and_then(|v| Settings::parse(&v));
        match result {
            Ok(next) => {
                let changed = self.current != next;
                self.current = next;
                changed
            }
            Err(e) => {
                eprintln!("config rejected (keeping last valid settings): {e}");
                false
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub right: i32,
    pub bottom: i32,
}
impl Placement {
    pub fn load() -> Option<Self> {
        let path = state_path();
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Placement>(&bytes) {
                Ok(p) if p.right >= 0 && p.bottom >= 0 => Some(p),
                _ => {
                    eprintln!("{}: invalid placement; using default", path.display());
                    None
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                None
            }
        }
    }
    pub fn save(&self) -> Result<(), String> {
        atomic_json(&state_path(), self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn premultiply_defaults_follow_the_character_and_accept_override() {
        let d = Settings::parse(&serde_json::json!({})).unwrap();
        assert_eq!(d.premultiply, crate::character::DEFAULT_PREMULTIPLY);
        let v = serde_json::json!({
            "pets": { crate::character::selected(): { "premultiply": false } }
        });
        assert!(!Settings::parse(&v).unwrap().premultiply);
        let bad = serde_json::json!({
            "pets": { crate::character::selected(): { "premultiply": "no" } }
        });
        assert!(Settings::parse(&bad).is_err());
    }

    #[test]
    fn reject_bad_scale_and_preserve_click_defaults() {
        let mut v = serde_json::json!({});
        v["pets"][crate::character::selected()]["scale"] = serde_json::json!(0.0);
        assert!(Settings::parse(&v).is_err());
        v["pets"][crate::character::selected()]["scale"] = serde_json::json!(0.75);
        let s = Settings::parse(&v).unwrap();
        assert_eq!(s.scale, 0.75);
        assert_eq!(s.click[0], "");
    }

    #[test]
    fn assets_root_accepts_absolute_paths_and_rejects_relative_ones() {
        let root = default_assets_root();
        let configured = serde_json::json!({
            "assets_root": root,
            "pets": {}
        });
        assert_eq!(
            Settings::parse_for(&configured, crate::character::selected())
                .unwrap()
                .assets_root,
            root
        );
        let relative = serde_json::json!({
            "assets_root": "relative/path",
            "pets": {}
        });
        assert!(Settings::parse_for(&relative, crate::character::selected()).is_err());
    }

    #[test]
    fn corrupt_hot_reload_retains_last_valid_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut w = Watcher {
            path: path.clone(),
            previous: None,
            current: Settings::parse(&serde_json::json!({})).unwrap(),
        };
        std::fs::write(&path, b"{").unwrap();
        assert!(!w.poll());
        assert_eq!(w.current.scale, 1.0);
        let mut v = serde_json::json!({});
        v["pets"][crate::character::selected()]["scale"] = serde_json::json!(0.75);
        atomic_json(&path, &v).unwrap();
        assert!(w.poll());
        assert_eq!(w.current.scale, 0.75);
    }
}
