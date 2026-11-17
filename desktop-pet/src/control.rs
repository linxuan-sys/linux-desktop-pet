use crate::character;
use fs2::FileExt;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const LOCK_WAIT_ENV: &str = "DESKTOP_PET_LOCK_WAIT";
const FOREGROUND_ENV: &str = "PET_RUN_FOREGROUND";

fn directory_for(pet: &str) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("pet")
        .join(pet)
}

fn pid_path() -> PathBuf {
    pid_path_for(character::selected())
}

fn pid_path_for(pet: &str) -> PathBuf {
    directory_for(pet).join("instance.pid")
}

fn lock_path_for(pet: &str) -> PathBuf {
    directory_for(pet).join("instance.lock")
}

fn read_pid_for(pet: &str) -> Option<u32> {
    let pid = std::fs::read_to_string(pid_path_for(pet))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<_> = cmdline
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part))
        .collect();
    (args.windows(2).any(|pair| {
        pair[0] == "run" && (pair[1] == pet || pair[1] == pet.strip_suffix("-girl").unwrap_or(pet))
    }) && args.first().is_some_and(|arg| {
        Path::new(arg.as_ref())
            .file_name()
            .is_some_and(|name| name == "pet")
    }))
    .then_some(pid)
}

#[cfg(test)]
pub fn running() -> bool {
    read_pid_for(character::selected()).is_some()
}

pub fn pet_running(pet: &str) -> bool {
    read_pid_for(pet).is_some()
}

pub fn start_for(pet: &str) -> Result<String, String> {
    if let Some(pid) = read_pid_for(pet) {
        return Ok(format!("已在运行 · PID {pid}"));
    }
    let _lock = instance_lock(pet, false)?;
    if let Some(pid) = read_pid_for(pet) {
        return Ok(format!("已在运行 · PID {pid}"));
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        })
        .join("pet");
    std::fs::create_dir_all(&state).map_err(|error| error.to_string())?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(state.join(format!("{pet}.log")))
        .map_err(|error| error.to_string())?;
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command
        .arg("run")
        .arg(pet)
        .env(LOCK_WAIT_ENV, "1")
        .env(FOREGROUND_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            log.try_clone().map_err(|error| error.to_string())?,
        ))
        .stderr(Stdio::from(log));
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|error| error.to_string())?;
    if let Err(error) = write_pid_for(pet, child.id()) {
        let _ = unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        return Err(error);
    }
    Ok(format!("已启动 · PID {}", child.id()))
}

fn write_pid_for(pet: &str, pid: u32) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(pid_path_for(pet))
        .map_err(|error| error.to_string())?;
    writeln!(file, "{pid}").map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

fn instance_lock(pet: &str, wait: bool) -> Result<File, String> {
    let path = lock_path_for(pet);
    std::fs::create_dir_all(path.parent().ok_or("invalid runtime directory")?)
        .map_err(|error| error.to_string())?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    if wait {
        file.lock_exclusive().map_err(|error| error.to_string())?;
    } else {
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == ErrorKind::WouldBlock {
                format!("{pet} is already running or starting")
            } else {
                error.to_string()
            }
        })?;
    }
    Ok(file)
}

pub fn stop() -> Result<(), String> {
    stop_for(character::selected())
}

pub fn stop_for(pet: &str) -> Result<(), String> {
    let Some(pid) = read_pid_for(pet) else {
        let _ = std::fs::remove_file(pid_path_for(pet));
        return Ok(());
    };
    if unsafe { libc::kill(pid as i32, libc::SIGTERM) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub struct InstanceGuard {
    path: PathBuf,
    pid: u32,
    _lock: File,
}
impl InstanceGuard {
    pub fn acquire() -> Result<Self, String> {
        let path = pid_path();
        let pet = character::selected();
        let wait = std::env::var_os(LOCK_WAIT_ENV).is_some();
        let lock = instance_lock(pet, wait)?;
        let pid = std::process::id();
        if read_pid_for(pet).is_some_and(|running| running != pid) {
            return Err(format!("{pet} is already running"));
        }
        write_pid_for(pet, pid)?;
        Ok(Self {
            path,
            pid,
            _lock: lock,
        })
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
            == Some(self.pid)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_stale_pid_is_not_running() {
        let path = pid_path();
        let _ = std::fs::remove_file(path);
        assert!(!running());
    }
}
