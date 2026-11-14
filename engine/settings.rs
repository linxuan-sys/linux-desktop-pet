//! User preferences and persisted placement are separate, atomically written files.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub scale: f64,
    pub cache_mib: usize,
    pub output: Option<String>,
    pub click: [String; 3],
}
impl Settings {
    pub fn parse(raw: &Value) -> Result<Self, String> {
        if !raw.is_object() || raw.get("pets").is_some_and(|v| !v.is_object()) {
            return Err("configuration root and pets must be objects".into());
        }
        if raw["pets"]
            .get(env!("CARGO_PKG_NAME"))
            .is_some_and(|v| !v.is_object())
        {
            return Err("pet configuration must be an object".into());
        }
        let section = &raw["pets"][env!("CARGO_PKG_NAME")];
        let scale = match section.get("scale") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|v| v.is_finite() && (0.5..=1.5).contains(v))
                .ok_or("scale must be a number between 0.5 and 1.5")?,
        };
        let cache_mib = match section.get("cache_mib") {
            None => 128,
            Some(v) => v
                .as_u64()
                .filter(|v| (128..=512).contains(v))
                .ok_or("cache_mib must be an integer between 128 and 512")?
                as usize,
        };
        let output = match section.get("output") {
            None | Some(Value::Null) => None,
            Some(Value::String(v)) if !v.is_empty() => Some(v.clone()),
            _ => return Err("output must be a nonempty output name or null".into()),
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
            scale,
            cache_mib,
            output,
            click: crate::sensors::proc::Config::parse(raw, env!("CARGO_PKG_NAME")).click,
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
pub fn config_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("desktop-pet/desktop-pet.json")
}
pub fn state_path() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
        .join("desktop-pet")
        .join(format!("{}.json", env!("CARGO_PKG_NAME")))
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
pub fn set_scale(scale: f64) -> Result<(), String> {
    let path = config_path();
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
    if raw["pets"]
        .get(env!("CARGO_PKG_NAME"))
        .is_some_and(|v| !v.is_object())
    {
        return Err("pet configuration must be an object".into());
    }
    raw["pets"][env!("CARGO_PKG_NAME")]["scale"] = serde_json::json!(scale);
    Settings::parse(&raw)?;
    atomic_json(&path, &raw)
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
    fn reject_bad_scale_and_preserve_click_defaults() {
        let mut v = serde_json::json!({});
        v["pets"][env!("CARGO_PKG_NAME")]["scale"] = serde_json::json!(0.0);
        assert!(Settings::parse(&v).is_err());
        v["pets"][env!("CARGO_PKG_NAME")]["scale"] = serde_json::json!(0.75);
        let s = Settings::parse(&v).unwrap();
        assert_eq!(s.scale, 0.75);
        assert_eq!(s.click[0], "");
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
        v["pets"][env!("CARGO_PKG_NAME")]["scale"] = serde_json::json!(0.75);
        atomic_json(&path, &v).unwrap();
        assert!(w.poll());
        assert_eq!(w.current.scale, 0.75);
    }
}
