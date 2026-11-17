//! Release media: independently seekable, lossless cropped WebP frames.
//! Keep the displayed frame and at most one prefetched frame.
use super::*;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

const MAGIC: &[u8; 8] = b"PETFRM01";
const COUNT: usize = (COLS * ROWS) as usize;
const HEADER: usize = 16 + COUNT * 28;
const MAX_ENCODED: usize = (FRAME_W * FRAME_H * 8) as usize;
const HOT_CACHE_LIMIT: usize = 16 * 1024 * 1024;

#[path = "raw.rs"]
mod raw;

pub(super) fn check_original_assets(media: &Path) -> Result<(), String> {
    raw::check_assets(media)
}

/// Cache just the most frequently decoded clip, after its first full playback.
/// Zstd is larger than WebP on disk, but much faster for this bounded RAM cache.
struct HotCache {
    name: Option<String>,
    frames: HashMap<u32, Vec<u8>>,
    counts: HashMap<String, u64>,
    oversized: HashSet<String>,
    bytes: usize,
    published_bytes: Arc<AtomicUsize>,
}
impl HotCache {
    fn new(published_bytes: Arc<AtomicUsize>) -> Self {
        Self {
            name: None,
            frames: HashMap::new(),
            counts: HashMap::new(),
            oversized: HashSet::new(),
            bytes: 0,
            published_bytes,
        }
    }
    fn select(&mut self, name: &str) {
        let count = self.counts.entry(name.into()).or_default();
        *count += 1;
        let count = *count;
        if self.name.as_deref() != Some(name)
            && !self.oversized.contains(name)
            && count > COUNT as u64
            && self.name.as_ref().is_none_or(|n| count > self.counts[n])
        {
            self.frames.clear();
            self.name = Some(name.into());
            self.bytes = 0;
            self.published_bytes.store(0, Ordering::Relaxed);
        }
    }
    fn get(&self, name: &str, index: u32, entry: &Entry) -> Result<Option<Frame>, String> {
        if self.name.as_deref() != Some(name) {
            return Ok(None);
        }
        let Some(encoded) = self.frames.get(&index) else {
            return Ok(None);
        };
        let pixels = zstd::bulk::decompress(encoded, (entry.width * entry.height * 4) as usize)
            .map_err(|e| format!("RAM frame decode: {e}"))?;
        Ok(Some(Frame {
            name: name.into(),
            index,
            entry: entry.clone(),
            pixels,
        }))
    }
    fn insert(&mut self, frame: &Frame) -> Result<(), String> {
        if self.name.as_deref() != Some(&frame.name) || self.frames.contains_key(&frame.index) {
            return Ok(());
        }
        let encoded =
            zstd::bulk::compress(&frame.pixels, 1).map_err(|e| format!("RAM frame encode: {e}"))?;
        if self.bytes + encoded.len() > HOT_CACHE_LIMIT {
            self.oversized.insert(frame.name.clone());
            self.name = None;
            self.frames.clear();
            self.bytes = 0;
        } else {
            self.bytes += encoded.len();
            self.frames.insert(frame.index, encoded);
        }
        self.published_bytes.store(self.bytes, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct Library {
    version: u32,
    pet: String,
    clips: Vec<String>,
}
impl Library {
    fn load(media: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(media.join("packed/library.json")).map_err(|e| e.to_string())?;
        let library: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        if library.version != 1
            || library.pet != crate::character::selected()
            || library.clips.is_empty()
        {
            return Err("unsupported or mismatched packed library".into());
        }
        let mut seen = HashSet::new();
        for name in &library.clips {
            if name.is_empty()
                || name.contains(['/', '\\', '\0'])
                || name == "."
                || name == ".."
                || !seen.insert(name)
            {
                return Err("invalid or duplicate packed clip name".into());
            }
        }
        Ok(library)
    }
}

#[derive(Clone, Debug)]
struct Entry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    offset: u64,
    length: usize,
}
struct Reader {
    file: File,
    entries: Vec<Entry>,
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
impl Reader {
    fn open(path: &Path) -> Result<Self, String> {
        let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        let mut header = vec![0; HEADER];
        file.read_exact(&mut header)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if &header[..8] != MAGIC
            || u32_at(&header, 8) != COUNT as u32
            || u32_at(&header, 12) != FPS as u32
        {
            return Err("unsupported packed clip header".into());
        }
        let mut entries = Vec::with_capacity(COUNT);
        let mut expected_offset = HEADER as u64;
        for i in 0..COUNT {
            let b = &header[16 + i * 28..16 + (i + 1) * 28];
            let e = Entry {
                x: u32_at(b, 0),
                y: u32_at(b, 4),
                width: u32_at(b, 8),
                height: u32_at(b, 12),
                offset: u64::from_le_bytes(b[16..24].try_into().unwrap()),
                length: u32_at(b, 24) as usize,
            };
            if e.width == 0
                || e.height == 0
                || e.width > FRAME_W
                || e.height > FRAME_H
                || e.x > FRAME_W - e.width
                || e.y > FRAME_H - e.height
                || e.length == 0
                || e.length > MAX_ENCODED
                || e.offset != expected_offset
            {
                return Err("invalid packed frame index".into());
            }
            expected_offset = e
                .offset
                .checked_add(e.length as u64)
                .ok_or("frame offset overflow")?;
            if expected_offset > length {
                return Err("truncated packed frame".into());
            }
            entries.push(e);
        }
        if expected_offset != length {
            return Err("unexpected packed clip length".into());
        }
        Ok(Self { file, entries })
    }
    fn frame(&mut self, index: u32) -> Result<Frame, String> {
        let entry = self
            .entries
            .get(index as usize)
            .ok_or("frame index out of range")?
            .clone();
        self.file
            .seek(SeekFrom::Start(entry.offset))
            .map_err(|e| e.to_string())?;
        let mut encoded = vec![0; entry.length];
        self.file
            .read_exact(&mut encoded)
            .map_err(|e| e.to_string())?;
        let pixels = decode_frame(&encoded, entry.width, entry.height)?;
        Ok(Frame {
            name: String::new(),
            index,
            entry,
            pixels,
        })
    }
}
fn decode_frame(encoded: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let features = webp::BitstreamFeatures::new(encoded).ok_or("invalid frame WebP")?;
    if features.width() != width
        || features.height() != height
        || features.has_animation()
        || !matches!(features.format(), Some(webp::BitstreamFormat::Lossless))
    {
        return Err("packed WebP dimensions or encoding do not match index".into());
    }
    let decoded = webp::Decoder::new(encoded)
        .decode()
        .ok_or("frame WebP decode failed")?;
    if decoded.is_alpha() {
        Ok(decoded.to_vec())
    } else {
        Ok(decoded
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect())
    }
}
struct Frame {
    name: String,
    index: u32,
    entry: Entry,
    pixels: Vec<u8>,
}
impl Frame {
    fn draw(&self, mirror: bool, shift: i32, premul: bool, canvas: &mut Canvas) {
        canvas.buf.fill(0);
        let e = &self.entry;
        for y in 0..e.height {
            for x in 0..e.width {
                let offset = ((y * e.width + x) * 4) as usize;
                let p: [u8; 4] = self.pixels[offset..offset + 4].try_into().unwrap();
                if p[3] == 0 {
                    continue;
                }
                let sx = if mirror {
                    FRAME_W - 1 - e.x - x
                } else {
                    e.x + x
                };
                let dx = PAD as i32 + shift + sx as i32;
                let dy = PAD + e.y + y;
                if dx < 0 || dx >= canvas.width as i32 || dy >= canvas.height {
                    continue;
                }
                let dest = ((dy * canvas.width + dx as u32) * 4) as usize;
                canvas.buf[dest..dest + 4].copy_from_slice(&if premul {
                    premultiply(p)
                } else {
                    p
                });
            }
        }
    }
}

/// One bounded request and result; no decoded animation cache.
pub struct PackedSheet {
    names: HashSet<String>,
    tx: SyncSender<(String, u32)>,
    rx: Receiver<(String, Result<Frame, String>)>,
    pending: bool,
    current: Option<Frame>,
    spare: Option<Frame>,
    failed: HashSet<String>,
    premultiply: bool,
    last: Option<(String, u32, bool, i32)>,
    hot_bytes: Arc<AtomicUsize>,
    raw_budget: Arc<AtomicUsize>,
}
impl PackedSheet {
    pub fn load(media: &Path) -> Result<Self, String> {
        Self::load_with_budget(media, 32)
    }
    pub fn load_with_budget(media: &Path, budget_mib: usize) -> Result<Self, String> {
        let original = media.join("anim").is_dir();
        let clip_names = if original {
            names(media)?
        } else {
            Library::load(media)?.clips
        };
        let dir = media.join("packed");
        let media = media.to_path_buf();
        let (tx, requests) = mpsc::sync_channel::<(String, u32)>(1);
        let (results, rx) = mpsc::sync_channel(1);
        let hot_bytes = Arc::new(AtomicUsize::new(0));
        let worker_bytes = hot_bytes.clone();
        let raw_budget = Arc::new(AtomicUsize::new(budget_mib * 1048576));
        let worker_budget = raw_budget.clone();
        std::thread::Builder::new()
            .name("pet-frame-decode".into())
            .spawn(move || {
                let mut open: Option<(String, Reader)> = None;
                let mut hot = HotCache::new(worker_bytes);
                let mut source =
                    raw::ClipCache::new(media, worker_budget, hot.published_bytes.clone());
                while let Ok((name, index)) = requests.recv() {
                    let value = (|| {
                        if original {
                            return source.as_mut().map_err(|e| e.clone())?.frame(&name, index);
                        }
                        if open.as_ref().is_none_or(|(n, _)| n != &name) {
                            open = Some((
                                name.clone(),
                                Reader::open(&dir.join(format!("{name}.pet")))?,
                            ));
                        }
                        hot.select(&name);
                        let reader = &mut open.as_mut().unwrap().1;
                        if let Some(frame) =
                            hot.get(&name, index, &reader.entries[index as usize])?
                        {
                            return Ok(frame);
                        }
                        let mut frame = reader.frame(index)?;
                        frame.name = name.clone();
                        hot.insert(&frame)?;
                        Ok(frame)
                    })();
                    if results.send((name, value)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            names: clip_names.into_iter().collect(),
            tx,
            rx,
            pending: false,
            current: None,
            spare: None,
            failed: HashSet::new(),
            premultiply: true,
            last: None,
            hot_bytes,
            raw_budget,
        })
    }
    fn receive(&mut self) {
        if let Ok((name, result)) = self.rx.try_recv() {
            self.pending = false;
            match result {
                Ok(frame) => {
                    if self.current.is_none() {
                        self.current = Some(frame);
                    } else {
                        self.spare = Some(frame);
                    }
                }
                Err(e) => {
                    eprintln!("animation {name}: {e}");
                    self.failed.insert(name);
                }
            }
        }
    }
    fn request(&mut self, name: &str, index: u32) {
        if !self.pending && !self.failed.contains(name) {
            if !self.names.contains(name) {
                eprintln!("animation {name}: missing from library");
                self.failed.insert(name.into());
            } else if self.tx.send((name.into(), index)).is_ok() {
                self.pending = true;
            } else {
                eprintln!("animation decoder stopped");
                self.failed.insert(name.into());
            }
        }
    }
    pub fn ready(&mut self, pose: &str) -> bool {
        self.receive();
        let name = crate::character::pose_anim(pose).unwrap_or(pose);
        if self.failed.contains(name) || self.current.as_ref().is_some_and(|f| f.name == name) {
            return true;
        }
        if self.spare.as_ref().is_some_and(|f| f.name == name) {
            self.current = self.spare.take();
            return true;
        }
        self.request(name, 0);
        self.failed.contains(name)
    }
    pub fn invalidate(&mut self) {
        self.last = None;
    }
    pub fn set_premultiply(&mut self, on: bool) {
        if self.premultiply != on {
            self.premultiply = on;
            self.invalidate();
        }
    }
    pub fn bytes(&self) -> usize {
        self.current.as_ref().map_or(0, |f| f.pixels.len())
            + self.spare.as_ref().map_or(0, |f| f.pixels.len())
            + self.hot_bytes.load(Ordering::Relaxed)
    }
    pub fn set_budget(&mut self, mib: usize) {
        self.raw_budget.store(mib * 1048576, Ordering::Relaxed);
    }
    pub fn draw(
        &mut self,
        pose: &str,
        elapsed: u64,
        mirror: bool,
        shift: i32,
        canvas: &mut Canvas,
    ) -> bool {
        self.receive();
        let name = crate::character::pose_anim(pose).unwrap_or(pose);
        let index = ((elapsed * FPS / 1000) % COUNT as u64) as u32;
        if self.failed.contains(name) {
            return false;
        }
        if self
            .spare
            .as_ref()
            .is_some_and(|f| f.name == name && f.index == index)
        {
            self.current = self.spare.take();
        }
        if self
            .current
            .as_ref()
            .is_none_or(|f| f.name != name || f.index != index)
        {
            self.request(name, index);
            return false;
        }
        let frame = self.current.as_ref().unwrap();
        let left = if mirror {
            FRAME_W - frame.entry.x - frame.entry.width
        } else {
            frame.entry.x
        };
        let shift = shift.clamp(-(left as i32), (FRAME_W - left - frame.entry.width) as i32);
        let key = (name.to_owned(), index, mirror, shift);
        if self.last.as_ref() == Some(&key) {
            return false;
        }
        frame.draw(mirror, shift, self.premultiply, canvas);
        self.last = Some(key);
        let next = (index + 1) % COUNT as u32;
        if !self
            .spare
            .as_ref()
            .is_some_and(|f| f.name == name && f.index == next)
        {
            self.request(name, next);
        }
        true
    }
}

/// Read the original with the same decoder and repairs as the development renderer.
fn source_anim(media: &Path, name: &str) -> Result<Anim, String> {
    let path = media.join("anim").join(format!("{name}.webp"));
    if image::image_dimensions(&path).map_err(|e| format!("{}: {e}", path.display()))?
        != (FRAME_W * COLS, FRAME_H * ROWS)
    {
        return Err(format!(
            "{}: expected 5120x5400 source sheet",
            path.display()
        ));
    }
    process_source(media, read_image(&path)?)
}

enum NativePixels {
    Native(webp::WebPImage),
    Rgba(Vec<u8>),
}
impl std::ops::Deref for NativePixels {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Native(p) => p,
            Self::Rgba(p) => p,
        }
    }
}
impl std::ops::DerefMut for NativePixels {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Native(p) => p,
            Self::Rgba(p) => p,
        }
    }
}
fn native_source_anim(media: &Path, name: &str) -> Result<Anim<NativePixels>, String> {
    let path = media.join("anim").join(format!("{name}.webp"));
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let info = webp::BitstreamFeatures::new(&bytes).ok_or("invalid source WebP")?;
    if (info.width(), info.height()) != (FRAME_W * COLS, FRAME_H * ROWS) || info.has_animation() {
        return Err("expected one 5120x5400 source sheet".into());
    }
    let decoded = webp::Decoder::new(&bytes)
        .decode()
        .ok_or("source WebP decode failed")?;
    // Keep libwebp's owned RGBA allocation, avoiding a second 105 MiB copy.
    let pixels = if decoded.is_alpha() {
        NativePixels::Native(decoded)
    } else {
        NativePixels::Rgba(
            decoded
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
        )
    };
    let image = image::ImageBuffer::from_raw(info.width(), info.height(), pixels)
        .ok_or("invalid source RGBA length")?;
    process_source(media, image)
}

fn process_source<C: std::ops::Deref<Target = [u8]> + std::ops::DerefMut>(
    media: &Path,
    mut image: image::ImageBuffer<image::Rgba<u8>, C>,
) -> Result<Anim<C>, String> {
    if image.dimensions() != (FRAME_W * COLS, FRAME_H * ROWS) {
        return Err("expected 5120x5400 source sheet".into());
    }
    let mut frames = Vec::with_capacity(COUNT);
    for i in 0..COUNT as u32 {
        let (ox, oy) = (i % COLS * FRAME_W, i / COLS * FRAME_H);
        let (mut left, mut top, mut right, mut bottom) = (FRAME_W, FRAME_H, 0, 0);
        for y in 0..FRAME_H {
            for x in 0..FRAME_W {
                if image.get_pixel(ox + x, oy + y)[3] != 0 {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x + 1);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        if right == 0 {
            (left, top, right, bottom) = (0, 0, 1, 1);
        }
        frames.push(FrameMeta {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
            atlas_x: ox + left,
            atlas_y: oy + top,
        });
    }
    let meta = Metadata {
        version: 2,
        source_bytes: 0,
        source_modified_ns: 0,
        atlas_width: image.width(),
        atlas_height: image.height(),
        frames,
    };
    apply_source_repairs(media, &mut image, &meta)?;
    Ok(Anim { image, meta })
}

fn apply_source_repairs<C: std::ops::Deref<Target = [u8]> + std::ops::DerefMut>(
    media: &Path,
    image: &mut image::ImageBuffer<image::Rgba<u8>, C>,
    meta: &Metadata,
) -> Result<(), String> {
    if crate::character::selected() == "codex-girl" && !cleaned_assets(&media.join("optimized"))? {
        for frame in &meta.frames {
            repair_body_matte(image, frame);
        }
        blend_loop_seam(image, meta);
    }
    Ok(())
}

fn write_clip<C: std::ops::Deref<Target = [u8]> + 'static>(
    anim: &Anim<C>,
    path: &Path,
    effort: i32,
) -> Result<(), String> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or("missing clip parent")?)
        .map_err(|e| e.to_string())?;
    temp.write_all(&vec![0; HEADER])
        .map_err(|e| e.to_string())?;
    let mut header = Vec::with_capacity(HEADER);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&(COUNT as u32).to_le_bytes());
    header.extend_from_slice(&(FPS as u32).to_le_bytes());
    let mut config = webp::WebPConfig::new().map_err(|_| "initialize WebP encoder")?;
    config.lossless = 1;
    config.method = effort;
    config.quality = 75.0;
    config.exact = 0; // Only RGB at alpha=0 may change; verified below.
    let mut offset = HEADER as u64;
    for frame in &anim.meta.frames {
        let cropped = image::imageops::crop_imm(
            &anim.image,
            frame.atlas_x,
            frame.atlas_y,
            frame.width,
            frame.height,
        )
        .to_image();
        let encoded = webp::Encoder::from_rgba(cropped.as_raw(), frame.width, frame.height)
            .encode_advanced(&config)
            .map_err(|e| format!("encode frame: {e:?}"))?;
        let decoded = decode_frame(&encoded, frame.width, frame.height)?;
        for (a, b) in cropped
            .as_raw()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(decoded.as_chunks::<4>().0.iter())
        {
            if a[3] != b[3] || (a[3] != 0 && a != b) {
                return Err("lossless visible pixel verification failed".into());
            }
        }
        for n in [frame.x, frame.y, frame.width, frame.height] {
            header.extend_from_slice(&n.to_le_bytes());
        }
        header.extend_from_slice(&offset.to_le_bytes());
        header.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        temp.write_all(&encoded).map_err(|e| e.to_string())?;
        offset += encoded.len() as u64;
    }
    temp.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    temp.write_all(&header).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize, Deserialize, PartialEq)]
struct Stamp {
    source_bytes: u64,
    source_modified_ns: u128,
    effort: i32,
    repaired: bool,
}
pub fn pack_assets(media: &Path, output: &Path, effort: i32) -> Result<(), String> {
    if !(0..=6).contains(&effort) {
        return Err("pack effort must be 0..6".into());
    }
    let all = names(media)?;
    let out = output.join("packed");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let state = out.join(".build-state.json");
    let mut done: HashMap<String, Stamp> = match std::fs::read(&state) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => return Err(e.to_string()),
    };
    for (i, name) in all.iter().enumerate() {
        let (source_bytes, source_modified_ns) =
            fingerprint(&media.join("anim").join(format!("{name}.webp")))?;
        let repaired = crate::character::selected() == "codex-girl"
            && !cleaned_assets(&media.join("optimized"))?;
        let stamp = Stamp {
            source_bytes,
            source_modified_ns,
            effort,
            repaired,
        };
        let path = out.join(format!("{name}.pet"));
        if done.get(name) == Some(&stamp) && Reader::open(&path).is_ok() {
            continue;
        }
        let start = Instant::now();
        let anim = source_anim(media, name)?;
        write_clip(&anim, &path, effort)?;
        done.insert(name.clone(), stamp);
        crate::settings::atomic_json(&state, &done)?;
        eprintln!(
            "[{}/{}] {name}: {:.2} MiB, 120 frames verified, {:.1}s",
            i + 1,
            all.len(),
            path.metadata().map_err(|e| e.to_string())?.len() as f64 / 1048576.0,
            start.elapsed().as_secs_f64()
        );
    }
    let library = Library {
        version: 1,
        pet: crate::character::selected().into(),
        clips: all,
    };
    crate::settings::atomic_json(&out.join("library.json"), &library)?;
    Ok(())
}

/// Headless check using the actual release reader, no source sheets or Wayland.
pub fn check_assets(media: &Path) -> Result<(), String> {
    let library = Library::load(media)?;
    let start = Instant::now();
    let mut total = 0u64;
    let mut peak = 0usize;
    let mut max_ms = 0.0f64;
    let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
    for name in &library.clips {
        let path = media.join("packed").join(format!("{name}.pet"));
        total += path.metadata().map_err(|e| e.to_string())?.len();
        let mut reader = Reader::open(&path)?;
        for i in 0..COUNT as u32 {
            let t = Instant::now();
            let frame = reader
                .frame(i)
                .map_err(|e| format!("{name} frame {i}: {e}"))?;
            max_ms = max_ms.max(t.elapsed().as_secs_f64() * 1000.0);
            peak = peak.max(frame.pixels.len());
            frame.draw(i % 2 == 1, 0, true, &mut canvas);
        }
    }
    println!(
        "{}: {} clips, {} frames OK; {:.2} MiB stored; {:.3} MiB max decoded frame; {:.2} ms max read/decode; {:.2}s total",
        library.pet,
        library.clips.len(),
        library.clips.len() * COUNT,
        total as f64 / 1048576.0,
        peak as f64 / 1048576.0,
        max_ms,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn hot_cache_is_lazy_lossless_bounded_and_survives_a_short_reaction() {
        let bytes = Arc::new(AtomicUsize::new(0));
        let mut cache = HotCache::new(bytes.clone());
        for _ in 0..COUNT {
            cache.select("idle");
        }
        assert!(cache.name.is_none());
        cache.select("idle");
        let entry = Entry {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
            offset: 0,
            length: 0,
        };
        let frame = Frame {
            name: "idle".into(),
            index: 0,
            entry: entry.clone(),
            pixels: vec![17, 45, 200, 93, 0, 128, 255, 255],
        };
        cache.insert(&frame).unwrap();
        assert!(bytes.load(Ordering::Relaxed) > 0);
        cache.select("reaction");
        assert_eq!(
            cache.get("idle", 0, &entry).unwrap().unwrap().pixels,
            frame.pixels
        );
        assert!(cache.get("reaction", 0, &entry).unwrap().is_none());
        cache.bytes = HOT_CACHE_LIMIT;
        let second = Frame { index: 1, ..frame };
        cache.insert(&second).unwrap();
        assert_eq!(cache.bytes, 0);
        assert_eq!(bytes.load(Ordering::Relaxed), 0);
        assert!(cache.oversized.contains("idle"));
        cache.select("idle");
        assert!(cache.name.is_none());
    }

    fn fixture(dir: &Path, names: &[&str]) -> Anim {
        std::fs::create_dir_all(dir.join("packed")).unwrap();
        let mut image = RgbaImage::new(4 * COUNT as u32, 3);
        let mut frames = Vec::new();
        for i in 0..COUNT as u32 {
            let ax = i * 4;
            for y in 0..3 {
                for x in 0..4 {
                    let p = match (x, y) {
                        (0, 0) => [199, 20, 91, 0],
                        (1, 0) => [17, 45, 200, 93],
                        _ => [i as u8, 126, 247, 255],
                    };
                    image.put_pixel(ax + x, y, image::Rgba(p));
                }
            }
            frames.push(FrameMeta {
                x: if i % 2 == 0 { 0 } else { FRAME_W - 4 },
                y: i % (FRAME_H - 3),
                width: 4,
                height: 3,
                atlas_x: ax,
                atlas_y: 0,
            });
        }
        let anim = Anim {
            image,
            meta: Metadata {
                version: 2,
                source_bytes: 0,
                source_modified_ns: 0,
                atlas_width: 4 * COUNT as u32,
                atlas_height: 3,
                frames,
            },
        };
        for name in names {
            write_clip(&anim, &dir.join("packed").join(format!("{name}.pet")), 0).unwrap();
        }
        crate::settings::atomic_json(
            &dir.join("packed/library.json"),
            &Library {
                version: 1,
                pet: crate::character::selected().into(),
                clips: names.iter().map(|s| s.to_string()).collect(),
            },
        )
        .unwrap();
        anim
    }
    fn wait_draw(sheet: &mut PackedSheet, name: &str, elapsed: u64, canvas: &mut Canvas) {
        let start = Instant::now();
        loop {
            sheet.ready(name);
            if sheet.draw(name, elapsed, false, 0, canvas) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "frame decode timeout"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn lossless_roundtrip_preserves_every_visible_pixel_and_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let anim = fixture(dir.path(), &["sample"]);
        let mut reader = Reader::open(&dir.path().join("packed/sample.pet")).unwrap();
        for i in 0..COUNT as u32 {
            let f = reader.frame(i).unwrap();
            for (pos, p) in f.pixels.as_chunks::<4>().0.iter().enumerate() {
                let a = anim
                    .image
                    .get_pixel(i * 4 + pos as u32 % 4, pos as u32 / 4)
                    .0;
                assert_eq!(a[3], p[3]);
                if a[3] != 0 {
                    assert_eq!(&a, p);
                }
            }
        }
    }
    #[test]
    fn packed_render_matches_atlas_for_mirroring_edges_and_premultiplication() {
        let dir = tempfile::tempdir().unwrap();
        let anim = fixture(dir.path(), &["sample"]);
        let mut reader = Reader::open(&dir.path().join("packed/sample.pet")).unwrap();
        let mut a = Canvas::new(WINDOW_W, WINDOW_H);
        let mut b = Canvas::new(WINDOW_W, WINDOW_H);
        for i in [0, 1, 65, 119] {
            let f = reader.frame(i).unwrap();
            for mirror in [false, true] {
                let left = if mirror {
                    FRAME_W - f.entry.x - f.entry.width
                } else {
                    f.entry.x
                };
                for shift in [-ART_SHIFT, 0, ART_SHIFT] {
                    let shift =
                        shift.clamp(-(left as i32), (FRAME_W - left - f.entry.width) as i32);
                    for premul in [false, true] {
                        anim.draw(i, mirror, shift, premul, &mut a);
                        f.draw(mirror, shift, premul, &mut b);
                        assert_eq!(a.buf, b.buf, "frame {i}, mirror {mirror}, shift {shift}");
                    }
                }
            }
        }
    }
    #[test]
    fn release_loads_without_originals_and_bounds_prefetch_to_one_frame() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["first", "second"]);
        let mut sheet = PackedSheet::load(dir.path()).unwrap();
        let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
        for i in 0..COUNT as u64 {
            wait_draw(&mut sheet, "first", (i * 1000).div_ceil(FPS), &mut canvas);
            assert!(sheet.bytes() <= HOT_CACHE_LIMIT + 2 * 4 * 3 * 4);
        }
        wait_draw(&mut sheet, "second", 0, &mut canvas);
        assert!(sheet.bytes() <= HOT_CACHE_LIMIT + 96);
        assert!(!sheet.draw("second", 20, false, 0, &mut canvas));
        check_assets(dir.path()).unwrap();
    }
    #[test]
    fn superseded_request_cannot_draw_the_wrong_clip() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["first", "second"]);
        let mut sheet = PackedSheet::load(dir.path()).unwrap();
        assert!(!sheet.ready("first"));
        let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
        wait_draw(&mut sheet, "second", 1000, &mut canvas);
        assert_eq!(sheet.current.as_ref().unwrap().name, "second");
        assert_eq!(sheet.current.as_ref().unwrap().index, 12);
    }
    #[test]
    fn prefetched_frame_waits_until_its_time_without_redrawing_or_decoding_twice() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["sample"]);
        let mut sheet = PackedSheet::load(dir.path()).unwrap();
        let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
        wait_draw(&mut sheet, "sample", 0, &mut canvas);
        let previous = canvas.buf.clone();
        let start = Instant::now();
        while sheet.spare.is_none() {
            sheet.ready("sample");
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(sheet.spare.as_ref().unwrap().index, 1);
        assert!(!sheet.draw("sample", 20, false, 0, &mut canvas));
        assert_eq!(previous, canvas.buf);
        assert!(sheet.draw("sample", 84, false, 0, &mut canvas));
        assert_eq!(sheet.current.as_ref().unwrap().index, 1);
        assert_eq!(sheet.bytes(), 48);
    }
    #[test]
    fn truncated_and_invalid_indices_are_rejected_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["sample"]);
        let path = dir.path().join("packed/sample.pet");
        let bytes = std::fs::read(&path).unwrap();
        for length in [0, 12, HEADER - 1, bytes.len() - 1] {
            std::fs::write(&path, &bytes[..length]).unwrap();
            assert!(Reader::open(&path).is_err());
        }
        let mut invalid = bytes;
        invalid[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&path, invalid).unwrap();
        assert!(Reader::open(&path).is_err());
    }
    #[test]
    fn broken_frame_does_not_overwrite_canvas_or_freeze_clock() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["first", "broken"]);
        let mut sheet = PackedSheet::load(dir.path()).unwrap();
        let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
        wait_draw(&mut sheet, "first", 0, &mut canvas);
        let previous = canvas.buf.clone();
        std::fs::write(dir.path().join("packed/broken.pet"), b"broken").unwrap();
        let start = Instant::now();
        while !sheet.ready("broken") {
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(sheet.failed.contains("broken"));
        assert!(!sheet.draw("broken", 0, false, 0, &mut canvas));
        assert_eq!(canvas.buf, previous);
        assert!(sheet.ready("missing"));
    }
    #[test]
    fn manifest_rejects_wrong_pet_and_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["sample"]);
        for (pet, names) in [
            (crate::character::selected(), vec!["../sample"]),
            ("unknown", vec!["sample"]),
            (crate::character::selected(), vec!["sample", "sample"]),
        ] {
            crate::settings::atomic_json(
                &dir.path().join("packed/library.json"),
                &Library {
                    version: 1,
                    pet: pet.into(),
                    clips: names.into_iter().map(String::from).collect(),
                },
            )
            .unwrap();
            assert!(PackedSheet::load(dir.path()).is_err());
        }
    }
    #[test]
    fn frame_dimensions_are_checked_before_webp_allocation() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), &["sample"]);
        let mut reader = Reader::open(&dir.path().join("packed/sample.pet")).unwrap();
        reader.entries[0].width = 3;
        assert!(reader.frame(0).is_err());
    }

    #[test]
    #[ignore = "requires repository original and prepared character assets"]
    fn real_export_matches_development_renderer_pixel_for_pixel() {
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("media");
        let dir = tempfile::tempdir().unwrap();
        // Rebuild only the three reference clips in a temporary directory.
        // Existing optimized caches can be stale; never rewrite a user's cache.
        let reference = tempfile::tempdir().unwrap();
        std::fs::create_dir(reference.path().join("anim")).unwrap();
        for pose in ["idle", "happy", "walk-left"] {
            let name = crate::character::pose_anim(pose).unwrap();
            std::fs::copy(
                media.join("anim").join(format!("{name}.webp")),
                reference.path().join("anim").join(format!("{name}.webp")),
            )
            .unwrap();
        }
        if media.join("ASSETS.json").exists() {
            std::fs::copy(
                media.join("ASSETS.json"),
                reference.path().join("ASSETS.json"),
            )
            .unwrap();
        }
        prepare_assets(reference.path(), false).unwrap();
        let mut a = Canvas::new(WINDOW_W, WINDOW_H);
        let mut b = Canvas::new(WINDOW_W, WINDOW_H);
        for pose in ["idle", "happy", "walk-left"] {
            let name = crate::character::pose_anim(pose).unwrap();
            let original = source_anim(&media, name).unwrap();
            let baseline = Anim::decode(
                &reference.path().join("optimized"),
                name,
                load_metadata(reference.path(), name).unwrap(),
            )
            .unwrap();
            let path = dir.path().join("sample.pet");
            write_clip(&original, &path, 0).unwrap();
            let mut reader = Reader::open(&path).unwrap();
            for i in 0..COUNT as u32 {
                let frame = reader.frame(i).unwrap();
                for mirror in [false, true] {
                    baseline.draw(i, mirror, 0, true, &mut a);
                    frame.draw(mirror, 0, true, &mut b);
                    assert_eq!(a.buf, b.buf, "{name} frame {i}, mirror {mirror}");
                }
            }
        }
    }
}
