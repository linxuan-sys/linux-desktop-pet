//! Versioned, per-character resource downloads, separate from executable releases.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Manifest,
    Download { received: u64, total: u64 },
    Verify,
}
pub type ProgressSender = Sender<Progress>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSource {
    Explicit,
    Repository,
    Managed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaLocation {
    pub path: PathBuf,
    pub source: MediaSource,
}

const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct Source {
    version: u32,
    repository: String,
    tag: String,
}
impl Default for Source {
    fn default() -> Self {
        Self {
            version: 1,
            repository: option_env!("PET_ASSET_REPOSITORY")
                .unwrap_or("tingfeng347/linux-desktop-pet")
                .into(),
            tag: option_env!("PET_ASSET_TAG").unwrap_or("assets-v1").into(),
        }
    }
}
fn component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
}
impl Source {
    fn validate(&self) -> Result<(), String> {
        let repo: Vec<_> = self.repository.split('/').collect();
        if self.version != 1
            || repo.len() != 2
            || !repo.iter().all(|p| component(p))
            || !component(&self.tag)
        {
            return Err("invalid resource repository or version tag".into());
        }
        Ok(())
    }
    fn base(&self) -> String {
        std::env::var("PET_ASSET_BASE_URL").unwrap_or_else(|_| {
            format!(
                "https://github.com/{}/releases/download/{}",
                self.repository, self.tag
            )
        })
    }
}

#[derive(Serialize, Deserialize)]
struct ResourceFile {
    bytes: u64,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    pet: String,
    tag: String,
    archive_bytes: u64,
    archive_sha256: String,
    files: BTreeMap<String, ResourceFile>,
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
fn asset_path(name: &str) -> bool {
    if name == "ASSETS.json" {
        return true;
    }
    name.strip_prefix("anim/").is_some_and(|file| {
        file.ends_with(".webp")
            && file.len() > 5
            && !file.contains(['/', '\\', '\0'])
            && file != "."
            && file != ".."
    })
}
impl Manifest {
    fn validate_for(&self, source: &Source, pet: &str) -> Result<(), String> {
        if self.version != 1
            || self.pet != pet
            || self.tag != source.tag
            || self.archive_bytes == 0
            || self.archive_bytes > MAX_ARCHIVE
            || !valid_hash(&self.archive_sha256)
            || self.files.is_empty()
            || self.files.len() > 2000
        {
            return Err("invalid resource manifest".into());
        }
        let mut total = 0u64;
        let mut clips = 0;
        for (name, file) in &self.files {
            if !asset_path(name) || file.bytes == 0 || !valid_hash(&file.sha256) {
                return Err("invalid resource file entry".into());
            }
            total = total
                .checked_add(file.bytes)
                .ok_or("resource size overflow")?;
            if name.starts_with("anim/") {
                clips += 1;
            }
        }
        if total > MAX_ARCHIVE || clips == 0 {
            return Err("resource manifest has invalid unpacked size".into());
        }
        Ok(())
    }
    fn available(&self, dir: &Path) -> bool {
        self.files.iter().all(|(name, entry)| {
            dir.join(name)
                .metadata()
                .is_ok_and(|m| m.is_file() && m.len() == entry.bytes)
        })
    }
}

pub fn pet_data(pet: &str) -> Result<PathBuf, String> {
    Ok(crate::settings::configured_assets_root()?.join(pet))
}

fn has_webp(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("webp"))
        })
    })
}

fn usable_local_media(path: &Path) -> bool {
    path.join("packed/library.json").is_file() || has_webp(&path.join("anim"))
}

fn canonical_media(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn push_ancestor_candidates(candidates: &mut Vec<PathBuf>, start: &Path, pet: &str) {
    for ancestor in start.ancestors() {
        candidates.push(ancestor.join(pet).join("media"));
    }
}

fn repository_candidates(pet: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(directory) = exe.parent()
    {
        push_ancestor_candidates(&mut candidates, directory, pet);
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(workspace) = manifest_dir.parent() {
        candidates.push(workspace.join(pet).join("media"));
    }
    if let Ok(current) = std::env::current_dir() {
        push_ancestor_candidates(&mut candidates, &current, pet);
    }
    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(canonical_media(candidate)));
    candidates
}

fn managed_media_for(pet: &str) -> Option<MediaLocation> {
    let source = Source::default();
    source.validate().ok()?;
    let path = pet_data(pet).ok()?.join("assets").join(&source.tag);
    let manifest = std::fs::read(path.join(".ready.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())?;
    (manifest.validate_for(&source, pet).is_ok() && manifest.available(&path)).then_some({
        MediaLocation {
            path,
            source: MediaSource::Managed,
        }
    })
}

/// Resolve local media without touching the network. Explicit paths are strict:
/// an invalid override is an error and never falls back to a different source.
pub fn resolve_media_for(
    pet: &str,
    explicit: Option<&Path>,
) -> Result<Option<MediaLocation>, String> {
    if let Some(path) = explicit {
        return if usable_local_media(path) {
            Ok(Some(MediaLocation {
                path: canonical_media(path),
                source: MediaSource::Explicit,
            }))
        } else {
            Err(format!("素材目录不可用：{}", path.display()))
        };
    }
    if let Some(path) = repository_candidates(pet)
        .into_iter()
        .find(|candidate| usable_local_media(candidate))
    {
        return Ok(Some(MediaLocation {
            path: canonical_media(&path),
            source: MediaSource::Repository,
        }));
    }
    Ok(managed_media_for(pet))
}

/// Prefer explicit and repository media, then reuse the verified managed cache,
/// and download exactly once when no local copy exists.
pub fn prepare_media_for(pet: &str, explicit: Option<&Path>) -> Result<MediaLocation, String> {
    if let Some(media) = resolve_media_for(pet, explicit)? {
        return Ok(media);
    }
    let path = download_for(pet)?;
    Ok(MediaLocation {
        path,
        source: MediaSource::Managed,
    })
}

fn render_cli_progress(progress: Receiver<Progress>) {
    let started = Instant::now();
    let mut last_render = Instant::now() - Duration::from_secs(1);
    let mut received = 0;
    let mut total = 0;
    let mut verifying = false;
    let mut displayed = false;

    for update in progress {
        match update {
            Progress::Manifest => {
                eprint!("\r\x1b[2K  正在获取动画资源清单…");
                flush_stderr();
                displayed = true;
                last_render = Instant::now();
            }
            Progress::Download {
                received: next,
                total: size,
            } => {
                received = next;
                total = size;
                verifying = false;
                displayed |= total > 0;
            }
            Progress::Verify => {
                verifying = true;
                eprint!("\r\x1b[2K  下载完成，正在逐文件校验…");
                flush_stderr();
                displayed = true;
                last_render = Instant::now();
            }
        }
        if !verifying && last_render.elapsed() >= Duration::from_millis(100) {
            render_cli_bar(received, total, started.elapsed());
            last_render = Instant::now();
        }
    }
    if !verifying && total > 0 {
        render_cli_bar(received, total, started.elapsed());
    }
    if displayed {
        eprintln!();
    }
}

fn render_cli_bar(received: u64, total: u64, elapsed: Duration) {
    let ratio = (received as f64 / total.max(1) as f64).clamp(0.0, 1.0);
    let width = 24usize;
    let filled = (ratio * width as f64).round() as usize;
    let bar = format!(
        "{}{}",
        "━".repeat(filled),
        "─".repeat(width.saturating_sub(filled))
    );
    let seconds = elapsed.as_secs_f64().max(0.001);
    let rate = received as f64 / 1048576.0 / seconds;
    let remaining = total.saturating_sub(received) as f64 / 1048576.0 / rate.max(0.001);
    eprint!(
        "\r\x1b[2K  [{bar}] {:>6.1}/{:>6.1} MiB  {:>3.0}%  {:>5.1} MiB/s  ETA {}",
        received as f64 / 1048576.0,
        total as f64 / 1048576.0,
        ratio * 100.0,
        rate,
        format_eta(remaining)
    );
    flush_stderr();
}

fn format_eta(seconds: f64) -> String {
    let seconds = seconds.ceil().clamp(0.0, 99.0 * 60.0 + 59.0) as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn flush_stderr() {
    let _ = io::stderr().flush();
}

pub fn download_for(pet: &str) -> Result<PathBuf, String> {
    if io::stderr().is_terminal() {
        let pet = pet.to_owned();
        let (sender, progress) = channel();
        let worker = std::thread::spawn(move || download_managed(&pet, Some(&sender)));
        render_cli_progress(progress);
        return worker
            .join()
            .map_err(|_| "resource download worker panicked".to_string())?;
    }
    download_managed(pet, None)
}

#[cfg(feature = "unified")]
pub fn download_with_progress_for(pet: &str, progress: ProgressSender) -> Result<PathBuf, String> {
    download_managed(pet, Some(&progress))
}

fn download_managed(pet: &str, progress: Option<&ProgressSender>) -> Result<PathBuf, String> {
    let source = Source::default();
    source.validate()?;
    let base = source.base();
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return Err("resource URL must use HTTP(S)".into());
    }
    let store = pet_data(pet)?.join("assets").join(&source.tag);
    download(&source, &base, &store, pet, progress)
}

fn download(
    source: &Source,
    base: &str,
    store: &Path,
    pet: &str,
    progress: Option<&ProgressSender>,
) -> Result<PathBuf, String> {
    source.validate()?;
    let parent = store.parent().ok_or("invalid resource cache path")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join("download.lock"))
        .map_err(|e| e.to_string())?;
    fs2::FileExt::lock_exclusive(&lock).map_err(|e| e.to_string())?;
    if std::fs::read_to_string(store.join(".origin")).is_ok_and(|s| s == base)
        && let Ok(bytes) = std::fs::read(store.join(".ready.json"))
        && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
        && manifest.validate_for(source, pet).is_ok()
        && manifest.available(store)
    {
        return Ok(store.to_path_buf());
    }
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(900)))
        .timeout_per_read(Some(Duration::from_secs(30)))
        .build()
        .new_agent();
    let name = pet;
    if let Some(progress) = progress {
        let _ = progress.send(Progress::Manifest);
    }
    if progress.is_none() {
        eprintln!("正在获取 {name} 的动画资源清单…");
    }
    let mut response = agent
        .get(format!("{base}/{name}.manifest.json"))
        .call()
        .map_err(|e| format!("资源清单下载失败: {e}；请确认 {base} 资源 Release 已发布并可访问"))?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("resource manifest is too large".into());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    manifest.validate_for(source, pet)?;
    if let Some(progress) = progress {
        let _ = progress.send(Progress::Download {
            received: 0,
            total: manifest.archive_bytes,
        });
    }
    let mut response = agent
        .get(format!("{base}/{name}-assets.tar.gz"))
        .call()
        .map_err(|e| format!("动画下载失败: {e}"))?;
    let mut archive = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 65536];
    let mut report = Instant::now();
    if progress.is_none() {
        eprintln!(
            "正在下载 {name} 动画，共 {:.1} MiB（只需下载一次）",
            manifest.archive_bytes as f64 / 1048576.0
        );
    }
    let mut body = response.body_mut().as_reader();
    loop {
        let n = body.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > manifest.archive_bytes {
            return Err("download exceeds declared resource size".into());
        }
        hash.update(&buffer[..n]);
        archive.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        if let Some(progress) = progress {
            let _ = progress.send(Progress::Download {
                received: count,
                total: manifest.archive_bytes,
            });
        }
        if progress.is_none() && report.elapsed() >= Duration::from_secs(2) {
            eprintln!(
                "{name}: {:.0}%",
                count as f64 / manifest.archive_bytes as f64 * 100.0
            );
            report = Instant::now();
        }
    }
    if count != manifest.archive_bytes
        || format!("{:x}", hash.finalize()) != manifest.archive_sha256.to_ascii_lowercase()
    {
        return Err("resource archive size or SHA-256 mismatch; retry the download".into());
    }
    archive.as_file().sync_all().map_err(|e| e.to_string())?;
    let incoming = tempfile::Builder::new()
        .prefix(".assets-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    if let Some(progress) = progress {
        let _ = progress.send(Progress::Verify);
    }
    extract(archive.path(), &manifest, incoming.path())?;
    for (name, content) in [
        (".ready.json", bytes.as_slice()),
        (".origin", base.as_bytes()),
    ] {
        let mut file = File::create(incoming.path().join(name)).map_err(|e| e.to_string())?;
        file.write_all(content).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    if store.exists() {
        std::fs::remove_dir_all(store).map_err(|e| e.to_string())?;
    }
    std::fs::rename(incoming.path(), store).map_err(|e| e.to_string())?;
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    if progress.is_none() {
        eprintln!("{name} 动画资源已就绪。");
    }
    Ok(store.to_path_buf())
}

fn extract(archive: &Path, manifest: &Manifest, output: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(File::open(archive).map_err(|e| e.to_string())?);
    let mut tar = tar::Archive::new(decoder);
    let mut seen = HashSet::new();
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        if entry.header().entry_type().is_dir() && path == Path::new("anim") {
            continue;
        }
        let name = path.to_str().ok_or("non-UTF8 resource path")?;
        if !entry.header().entry_type().is_file()
            || !asset_path(name)
            || !seen.insert(name.to_owned())
        {
            return Err("unsafe or duplicate resource archive entry".into());
        }
        let expected = manifest
            .files
            .get(name)
            .ok_or("unlisted resource archive entry")?;
        if entry.size() != expected.bytes {
            return Err("resource entry size mismatch".into());
        }
        let dest = output.join(&path);
        std::fs::create_dir_all(dest.parent().ok_or("invalid resource destination")?)
            .map_err(|e| e.to_string())?;
        let mut file = File::create(dest).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let n = entry.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
        if format!("{:x}", hash.finalize()) != expected.sha256.to_ascii_lowercase() {
            return Err("resource file SHA-256 mismatch".into());
        }
        file.sync_all().map_err(|e| e.to_string())?;
    }
    if seen.len() != manifest.files.len() {
        return Err("resource archive is incomplete".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    const PET: &str = "kimi-girl";

    #[test]
    fn cli_eta_is_bounded_and_readable() {
        assert_eq!(format_eta(9.1), "00:10");
        assert_eq!(format_eta(125.0), "02:05");
        assert_eq!(format_eta(10_000.0), "99:59");
    }

    #[test]
    fn explicit_media_must_be_usable_and_wins_without_downloading() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("media");
        std::fs::create_dir_all(media.join("anim")).unwrap();
        assert!(resolve_media_for(PET, Some(&media)).is_err());

        std::fs::write(media.join("anim/待机.webp"), b"webp").unwrap();
        let resolved = resolve_media_for(PET, Some(&media)).unwrap().unwrap();
        assert_eq!(resolved.source, MediaSource::Explicit);
        assert_eq!(resolved.path, media.canonicalize().unwrap());

        std::fs::remove_dir_all(media.join("anim")).unwrap();
        std::fs::create_dir_all(media.join("packed")).unwrap();
        std::fs::write(media.join("packed/library.json"), b"{}").unwrap();
        assert_eq!(
            resolve_media_for(PET, Some(&media))
                .unwrap()
                .unwrap()
                .source,
            MediaSource::Explicit
        );
    }

    fn bundle() -> (Source, Manifest, Vec<u8>) {
        let source = Source::default();
        let mut files = BTreeMap::new();
        let mut tar = tar::Builder::new(Vec::new());
        for (name, data) in [
            ("anim/待机.webp", b"original WebP bytes".as_slice()),
            ("ASSETS.json", br#"{"status":"cleaned"}"#.as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, name, data).unwrap();
            files.insert(
                name.into(),
                ResourceFile {
                    bytes: data.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(data)),
                },
            );
        }
        let data = tar.into_inner().unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&data).unwrap();
        let archive = gzip.finish().unwrap();
        let manifest = Manifest {
            version: 1,
            pet: PET.into(),
            tag: source.tag.clone(),
            archive_bytes: archive.len() as u64,
            archive_sha256: format!("{:x}", Sha256::digest(&archive)),
            files,
        };
        (source, manifest, archive)
    }
    fn server(manifest: &Manifest, archive: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let manifest = serde_json::to_vec(manifest).unwrap();
        let thread = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                use std::io::BufRead;
                let mut reader = std::io::BufReader::new(&mut socket);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                        break;
                    }
                }
                let body = if request.lines().next().unwrap().contains(".manifest.json") {
                    &manifest
                } else {
                    &archive
                };
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                socket.write_all(body).unwrap();
            }
        });
        (base, thread)
    }
    #[test]
    fn download_preserves_original_bytes_and_cleaned_marker_and_then_works_offline() {
        let (source, manifest, archive) = bundle();
        let (base, server) = server(&manifest, archive);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("assets-v1");
        assert_eq!(
            download(&source, &base, &target, PET, None).unwrap(),
            target
        );
        server.join().unwrap();
        assert_eq!(
            std::fs::read(target.join("anim/待机.webp")).unwrap(),
            b"original WebP bytes"
        );
        assert_eq!(
            std::fs::read(target.join("ASSETS.json")).unwrap(),
            br#"{"status":"cleaned"}"#
        );
        // Server is gone. Any accidental network request makes this fail.
        assert_eq!(
            download(&source, &base, &target, PET, None).unwrap(),
            target
        );
    }
    #[test]
    fn checksum_failure_never_installs_partial_resources() {
        let (source, manifest, mut archive) = bundle();
        let last = archive.len() - 1;
        archive[last] ^= 1;
        let (base, server) = server(&manifest, archive);
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("assets-v1");
        assert!(
            download(&source, &base, &target, PET, None)
                .unwrap_err()
                .contains("SHA-256")
        );
        server.join().unwrap();
        assert!(!target.exists());
    }
    #[test]
    fn rejects_paths_other_characters_and_wrong_resource_versions() {
        let (source, mut manifest, _) = bundle();
        assert!(manifest.validate_for(&source, PET).is_ok());
        manifest.pet = "another-pet".into();
        assert!(manifest.validate_for(&source, PET).is_err());
        manifest.pet = PET.into();
        manifest.tag = "different".into();
        assert!(manifest.validate_for(&source, PET).is_err());
        for path in [
            "../escape.webp",
            "anim/../../escape.webp",
            "/tmp/x.webp",
            "anim/x/y.webp",
            "anim/x\\y.webp",
        ] {
            assert!(!asset_path(path));
        }
    }
    #[test]
    fn extraction_rejects_symlinks_and_missing_or_corrupt_files() {
        let (_, mut manifest, archive) = bundle();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("assets.tar.gz");
        std::fs::write(&path, archive).unwrap();
        manifest.files.get_mut("anim/待机.webp").unwrap().sha256 = "0".repeat(64);
        assert!(extract(&path, &manifest, dir.path()).is_err());
        let mut tar = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("/tmp/outside").unwrap();
        header.set_cksum();
        tar.append_data(&mut header, "anim/待机.webp", std::io::empty())
            .unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&tar.into_inner().unwrap()).unwrap();
        std::fs::write(&path, gzip.finish().unwrap()).unwrap();
        assert!(
            extract(&path, &manifest, dir.path())
                .unwrap_err()
                .contains("unsafe")
        );
    }
}
