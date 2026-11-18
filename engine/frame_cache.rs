//! Derived, seekable lossless frames. Directory names are stable; identity is in the header.
use super::{COUNT, Entry, FRAME_H, FRAME_W, Frame, names, raw};
use crate::settings::{self, DiskCacheMode};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{FileExt as _, MetadataExt};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 8] = b"PETFRAME";
const FORMAT: u32 = 1;
// Change this identity when cropping/repair semantics change; paths stay stable.
const PROCESSOR_ID: &[u8] = b"pet lossless cropped frames; zstd=1; repairs=1\0";
const HEADER: u64 = 80;
const RECORD: usize = 60;
const COMMON: &[&str] = &[
    "idle",
    "typing",
    "music",
    "dance",
    "walk-left",
    "dragged",
    "happy",
    "copy",
];

pub struct Status {
    pub mode: DiskCacheMode,
    pub directory: PathBuf,
    pub clips: usize,
    pub bytes: u64,
}
pub fn status(pet: &str) -> Result<Status, String> {
    let directory = settings::frame_cache_dir_for(pet);
    let mut clips = 0;
    let mut bytes = 0;
    match std::fs::read_dir(&directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.path().extension().is_some_and(|v| v == "frames") {
                    let metadata = entry.metadata().map_err(|e| e.to_string())?;
                    if metadata.is_file() {
                        clips += 1;
                        bytes += metadata.len();
                    }
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    Ok(Status {
        mode: settings::disk_cache_for(pet)?,
        directory,
        clips,
        bytes,
    })
}
fn eligible(pet: &str, name: &str, mode: DiskCacheMode) -> bool {
    match mode {
        DiskCacheMode::None => false,
        DiskCacheMode::Common => COMMON
            .iter()
            .any(|pose| crate::character::animation_for(pet, pose) == Some(name)),
        DiskCacheMode::All => true,
    }
}
fn lock(directory: &Path) -> Result<File, String> {
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".lock"))
        .map_err(|e| e.to_string())?;
    file.lock_exclusive().map_err(|e| e.to_string())?;
    Ok(file)
}
fn path_for(directory: &Path, name: &str) -> PathBuf {
    directory.join(format!("{name}.frames"))
}
fn fingerprint(media: &Path, pet: &str, name: &str) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    hash.update(PROCESSOR_ID);
    hash.update(pet.as_bytes());
    match std::fs::read(media.join("ASSETS.json")) {
        Ok(bytes) => hash.update(Sha256::digest(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => hash.update([0; 32]),
        Err(e) => return Err(e.to_string()),
    }
    let path = media.join("anim").join(format!("{name}.webp"));
    let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
#[derive(PartialEq, Eq)]
enum WriteReason {
    Prepare,
    Missing,
    Corrupt,
}
fn ensure(
    media: &Path,
    pet: &str,
    name: &str,
    key: &[u8; 32],
    reason: WriteReason,
) -> Result<bool, String> {
    let directory = settings::frame_cache_dir_for(pet);
    let _lock = lock(&directory)?;
    let path = path_for(&directory, name);
    if reason != WriteReason::Prepare && !eligible(pet, name, settings::disk_cache_for(pet)?) {
        return Ok(false);
    }
    if reason != WriteReason::Corrupt && Reader::open(&path, key).is_ok() {
        return Ok(true);
    }
    let clip = raw::Clip::load_for(media, name, pet)?;
    let mut output = tempfile::Builder::new()
        .prefix(".frame-")
        .tempfile_in(&directory)
        .map_err(|e| e.to_string())?;
    output.write_all(MAGIC).map_err(|e| e.to_string())?;
    output
        .write_all(&FORMAT.to_le_bytes())
        .map_err(|e| e.to_string())?;
    output
        .write_all(&(COUNT as u32).to_le_bytes())
        .map_err(|e| e.to_string())?;
    output.write_all(key).map_err(|e| e.to_string())?;
    let mut offset = HEADER + (COUNT * RECORD) as u64;
    let mut table = Vec::with_capacity(COUNT * RECORD);
    for (entry, encoded) in clip.entries.iter().zip(&clip.frames) {
        for value in [entry.x, entry.y, entry.width, entry.height] {
            table.extend_from_slice(&value.to_le_bytes());
        }
        table.extend_from_slice(&offset.to_le_bytes());
        table.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        table.extend_from_slice(&Sha256::digest(encoded));
        offset += encoded.len() as u64;
    }
    output
        .write_all(&Sha256::digest(&table))
        .map_err(|e| e.to_string())?;
    output.write_all(&table).map_err(|e| e.to_string())?;
    for encoded in &clip.frames {
        output.write_all(encoded).map_err(|e| e.to_string())?;
    }
    if fingerprint(media, pet, name)? != *key {
        return Err("素材在生成缓存期间发生变化，请重试。".into());
    }
    output.as_file().sync_all().map_err(|e| e.to_string())?;
    output.persist(&path).map_err(|e| e.to_string())?;
    File::open(&directory)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    drop(clip);
    raw::release_unused_decode_memory();
    Ok(true)
}

/// Prepare before publishing the setting; incomplete generation never enables a new mode.
pub fn configure(
    pet: &str,
    mode: DiskCacheMode,
    explicit_media: Option<&Path>,
    progress: Option<&crate::assets::ProgressSender>,
) -> Result<Status, String> {
    let mut selected = HashSet::new();
    if mode != DiskCacheMode::None {
        let media =
            crate::assets::prepare_media_with_progress_for(pet, explicit_media, progress)?.path;
        let clips: Vec<_> = names(&media)?
            .into_iter()
            .filter(|name| eligible(pet, name, mode))
            .collect();
        let total = clips.len();
        for (i, name) in clips.into_iter().enumerate() {
            if let Some(tx) = progress {
                let _ = tx.send(crate::assets::Progress::Frames { done: i, total });
            } else if i == 0 || i % 8 == 0 {
                eprintln!("{pet}：准备磁盘缓存 {i}/{total}…");
            }
            let key = fingerprint(&media, pet, &name)?;
            ensure(&media, pet, &name, &key, WriteReason::Prepare)?;
            selected.insert(format!("{name}.frames"));
        }
        if let Some(tx) = progress {
            let _ = tx.send(crate::assets::Progress::Frames { done: total, total });
        }
    }
    let directory = settings::frame_cache_dir_for(pet);
    // The same lock serializes pruning with atomic frame generation.
    let _lock = lock(&directory)?;
    settings::set_disk_cache_for(pet, mode)?;
    for entry in std::fs::read_dir(&directory).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ((name.ends_with(".frames") && !selected.contains(&name)) || name.starts_with(".frame-"))
            && entry.file_type().map_err(|e| e.to_string())?.is_file()
        {
            std::fs::remove_file(entry.path())
                .map_err(|e| format!("模式已保存，但清理 {} 失败：{e}", entry.path().display()))?;
        }
    }
    status(pet)
}

struct Record {
    entry: Entry,
    offset: u64,
    len: usize,
    hash: [u8; 32],
}
struct Reader {
    file: File,
    records: Vec<Record>,
    encoded: Vec<u8>,
}
impl Reader {
    fn open(path: &Path, key: &[u8; 32]) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let mut header = [0u8; HEADER as usize];
        file.read_exact_at(&mut header, 0)
            .map_err(|e| e.to_string())?;
        if &header[..8] != MAGIC
            || u32::from_le_bytes(header[8..12].try_into().unwrap()) != FORMAT
            || u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize != COUNT
            || &header[16..48] != key
        {
            return Err("磁盘缓存头已失效".into());
        }
        let mut table = vec![0; COUNT * RECORD];
        file.read_exact_at(&mut table, HEADER)
            .map_err(|e| e.to_string())?;
        if Sha256::digest(&table).as_slice() != &header[48..80] {
            return Err("磁盘缓存索引校验失败".into());
        }
        let mut offset = HEADER + table.len() as u64;
        let mut records = Vec::with_capacity(COUNT);
        let max = zstd::zstd_safe::compress_bound((FRAME_W * FRAME_H * 4) as usize);
        for bytes in table.as_chunks::<RECORD>().0 {
            let get = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
            let entry = Entry {
                x: get(0),
                y: get(4),
                width: get(8),
                height: get(12),
            };
            let at = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
            let len = get(24) as usize;
            if entry.width == 0
                || entry.height == 0
                || entry.width > FRAME_W
                || entry.height > FRAME_H
                || entry.x > FRAME_W - entry.width
                || entry.y > FRAME_H - entry.height
                || at != offset
                || len == 0
                || len > max
            {
                return Err("磁盘缓存帧索引无效".into());
            }
            offset += len as u64;
            records.push(Record {
                entry,
                offset: at,
                len,
                hash: bytes[28..60].try_into().unwrap(),
            });
        }
        if offset != file.metadata().map_err(|e| e.to_string())?.len() {
            return Err("磁盘缓存文件长度无效".into());
        }
        Ok(Self {
            file,
            records,
            encoded: Vec::new(),
        })
    }
    fn frame(
        &mut self,
        name: &str,
        index: u32,
        decoder: &mut zstd::bulk::Decompressor<'static>,
    ) -> Result<Frame, String> {
        let record = self
            .records
            .get(index as usize)
            .ok_or("frame index out of range")?;
        self.encoded.resize(record.len, 0);
        self.file
            .read_exact_at(&mut self.encoded, record.offset)
            .map_err(|e| e.to_string())?;
        if Sha256::digest(&self.encoded).as_slice() != record.hash {
            return Err("磁盘缓存帧校验失败".into());
        }
        let expected = (record.entry.width * record.entry.height * 4) as usize;
        let pixels = decoder
            .decompress(&self.encoded, expected)
            .map_err(|e| e.to_string())?;
        if pixels.len() != expected {
            return Err("磁盘缓存帧像素长度无效".into());
        }
        Ok(Frame {
            name: name.into(),
            index,
            entry: record.entry.clone(),
            pixels,
        })
    }
    fn bytes(&self) -> usize {
        self.encoded.capacity() + self.records.capacity() * std::mem::size_of::<Record>()
    }
}

type StampPart = (u64, i64, i64, u64, u64);
#[derive(PartialEq, Eq)]
struct Stamp {
    source: StampPart,
    marker: Option<StampPart>,
}
fn stamp_part(meta: std::fs::Metadata) -> StampPart {
    (
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ino(),
        meta.dev(),
    )
}
fn stamp(media: &Path, name: &str) -> Result<Stamp, String> {
    let source = std::fs::metadata(media.join("anim").join(format!("{name}.webp")))
        .map_err(|e| e.to_string())?;
    let marker = match std::fs::metadata(media.join("ASSETS.json")) {
        Ok(meta) => Some(stamp_part(meta)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    Ok(Stamp {
        source: stamp_part(source),
        marker,
    })
}
pub(super) struct Store {
    media: PathBuf,
    pet: String,
    mode: DiskCacheMode,
    current: Option<(String, [u8; 32], Reader)>,
    keys: HashMap<String, (Stamp, [u8; 32])>,
    failed: HashSet<String>,
}
impl Store {
    pub(super) fn new(media: PathBuf, pet: &str, mode: DiskCacheMode) -> Self {
        Self {
            media,
            pet: pet.into(),
            mode,
            current: None,
            keys: HashMap::new(),
            failed: HashSet::new(),
        }
    }
    pub(super) fn set_mode(&mut self, mode: DiskCacheMode) -> bool {
        if mode == self.mode {
            return false;
        }
        self.mode = mode;
        self.current = None;
        self.keys.clear();
        self.failed.clear();
        true
    }
    pub(super) fn bytes(&self) -> usize {
        self.current
            .as_ref()
            .map_or(0, |(_, _, reader)| reader.bytes())
    }
    pub(super) fn frame(
        &mut self,
        name: &str,
        index: u32,
        decoder: &mut zstd::bulk::Decompressor<'static>,
    ) -> Result<Option<Frame>, String> {
        if !eligible(&self.pet, name, self.mode) || self.failed.contains(name) {
            self.current = None;
            return Ok(None);
        }
        let result = self.read(name, index, decoder);
        if result.is_err() {
            self.failed.insert(name.into());
        }
        if !matches!(&result, Ok(Some(_))) {
            self.current = None;
        }
        result
    }
    fn read(
        &mut self,
        name: &str,
        index: u32,
        decoder: &mut zstd::bulk::Decompressor<'static>,
    ) -> Result<Option<Frame>, String> {
        let directory = settings::frame_cache_dir_for(&self.pet);
        if self
            .current
            .as_ref()
            .is_none_or(|(current, _, _)| current != name)
        {
            let next = stamp(&self.media, name)?;
            let key = match self.keys.get(name) {
                Some((old, key)) if *old == next => *key,
                _ => {
                    let key = fingerprint(&self.media, &self.pet, name)?;
                    self.keys.insert(name.into(), (next, key));
                    key
                }
            };
            let path = path_for(&directory, name);
            let reader = match Reader::open(&path, &key) {
                Ok(reader) => reader,
                Err(_) => {
                    // Recheck the saved mode under the writer lock, so a
                    // stale UI poll cannot undo another process's cleanup.
                    if !ensure(&self.media, &self.pet, name, &key, WriteReason::Missing)? {
                        return Ok(None);
                    }
                    Reader::open(&path, &key)?
                }
            };
            self.current = Some((name.into(), key, reader));
        }
        let (_, key, reader) = self.current.as_mut().unwrap();
        match reader.frame(name, index, decoder) {
            Ok(frame) => Ok(Some(frame)),
            Err(error) => {
                eprintln!("animation {name}: {error}；重建磁盘缓存");
                if !ensure(&self.media, &self.pet, name, key, WriteReason::Corrupt)? {
                    return Ok(None);
                }
                *reader = Reader::open(&path_for(&directory, name), key)?;
                reader.frame(name, index, decoder).map(Some)
            }
        }
    }
}
