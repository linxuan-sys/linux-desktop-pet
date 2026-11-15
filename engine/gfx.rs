//! Original-resolution animation storage and premultiplied rendering.
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};

pub const FRAME_W: u32 = 640;
pub const FRAME_H: u32 = 360;
pub const PAD: u32 = 6;
pub const ART_SHIFT: i32 = 148;
pub const WINDOW_W: u32 = FRAME_W + PAD * 2;
pub const WINDOW_H: u32 = FRAME_H + PAD * 2;
const COLS: u32 = 8;
const ROWS: u32 = 15;
pub const FPS: u64 = 12;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Metadata {
    version: u32,
    source_bytes: u64,
    source_modified_ns: u128,
    atlas_width: u32,
    atlas_height: u32,
    frames: Vec<FrameMeta>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
struct FrameMeta {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    atlas_x: u32,
    atlas_y: u32,
}
impl Metadata {
    fn bytes(&self) -> usize {
        (self.atlas_width * self.atlas_height * 4) as usize
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 2
            || self.atlas_width == 0
            || self.atlas_height == 0
            || self.atlas_width > 16_383
            || self.atlas_height > 16_383
            || self.frames.len() != (COLS * ROWS) as usize
        {
            return Err("invalid crop metadata".into());
        }
        for frame in &self.frames {
            if frame.width == 0
                || frame.height == 0
                || frame.width > FRAME_W
                || frame.height > FRAME_H
                || frame.x > FRAME_W - frame.width
                || frame.y > FRAME_H - frame.height
                || frame.width > self.atlas_width
                || frame.height > self.atlas_height
                || frame.atlas_x > self.atlas_width - frame.width
                || frame.atlas_y > self.atlas_height - frame.height
            {
                return Err("invalid frame crop metadata".into());
            }
        }
        Ok(())
    }
}
fn fingerprint(path: &Path) -> Result<(u64, u128), String> {
    let m = path
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let ns = m
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    Ok((m.len(), ns))
}
fn names(media: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(media.join("anim")).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "webp") {
            names.push(
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or("non-UTF8 asset name")?
                    .to_owned(),
            );
        }
    }
    names.sort();
    if names.is_empty() {
        return Err("animation directory is empty".into());
    }
    Ok(names)
}
fn load_metadata(media: &Path, name: &str) -> Result<Metadata, String> {
    let path = media.join("optimized").join(format!("{name}.json"));
    let m: Metadata = serde_json::from_slice(
        &std::fs::read(&path)
            .map_err(|e| format!("{}: {e}; run --prepare-assets", path.display()))?,
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    m.validate()?;
    let (bytes, ns) = fingerprint(&media.join("anim").join(format!("{name}.webp")))?;
    if bytes != m.source_bytes || ns != m.source_modified_ns {
        return Err(format!("{name}: original changed; run --prepare-assets"));
    }
    Ok(m)
}
fn read_image(path: &Path) -> Result<RgbaImage, String> {
    image::ImageReader::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("{}: {e}", path.display()))
        .map(|im| im.into_rgba8())
}
/// Offline only: retain the original resolution and each frame's coordinates.
pub fn prepare_assets(media: &Path, verify_existing: bool) -> Result<(), String> {
    let all = names(media)?;
    let out = media.join("optimized");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for (index, name) in all.iter().enumerate() {
        let dest = out.join(format!("{name}.webp"));
        if !verify_existing && dest.is_file() && load_metadata(media, name).is_ok() {
            continue;
        }
        let source = media.join("anim").join(format!("{name}.webp"));
        let original = read_image(&source)?;
        if original.dimensions() != (FRAME_W * COLS, FRAME_H * ROWS) {
            return Err(format!(
                "{name}: expected 5120x5400, got {:?}",
                original.dimensions()
            ));
        }
        let mut frames = Vec::with_capacity((COLS * ROWS) as usize);
        for index in 0..COLS * ROWS {
            let (col, row) = (index % COLS, index / COLS);
            let (mut left, mut top, mut right, mut bottom) = (FRAME_W, FRAME_H, 0, 0);
            for y in 0..FRAME_H {
                for x in 0..FRAME_W {
                    if original.get_pixel(col * FRAME_W + x, row * FRAME_H + y)[3] != 0 {
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
                atlas_x: 0,
                atlas_y: 0,
            });
        }
        let (atlas_width, atlas_height) = pack_frames(&mut frames)?;
        let (source_bytes, source_modified_ns) = fingerprint(&source)?;
        let meta = Metadata {
            version: 2,
            source_bytes,
            source_modified_ns,
            atlas_width,
            atlas_height,
            frames,
        };
        let mut atlas = RgbaImage::new(atlas_width, atlas_height);
        for (index, frame) in meta.frames.iter().enumerate() {
            let (col, row) = (index as u32 % COLS, index as u32 / COLS);
            for y in 0..frame.height {
                for x in 0..frame.width {
                    atlas.put_pixel(
                        frame.atlas_x + x,
                        frame.atlas_y + y,
                        *original
                            .get_pixel(col * FRAME_W + frame.x + x, row * FRAME_H + frame.y + y),
                    );
                }
            }
        }
        let tmp = tempfile::NamedTempFile::new_in(&out).map_err(|e| e.to_string())?;
        let mut config =
            webp::WebPConfig::new().map_err(|_| "initialize WebP encoder".to_string())?;
        config.lossless = 1;
        config.method = 4;
        config.quality = 50.0;
        config.exact = 0; // Transparent RGB is invisible; alpha stays lossless.
        config.alpha_compression = 0;
        config.alpha_filtering = 0;
        let encoded = webp::Encoder::new(
            atlas.as_raw(),
            webp::PixelLayout::Rgba,
            atlas.width(),
            atlas.height(),
        )
        .encode_advanced(&config)
        .map_err(|e| format!("{name}: WebP encode failed: {e:?}"))?;
        tmp.as_file()
            .write_all(&encoded)
            .map_err(|e| e.to_string())?;
        drop(atlas);
        let decoded = image::load_from_memory_with_format(
            &std::fs::read(tmp.path()).map_err(|e| e.to_string())?,
            image::ImageFormat::WebP,
        )
        .map_err(|e| e.to_string())?
        .into_rgba8();
        verify_crop(&original, &decoded, &meta)?;
        tmp.persist(&dest).map_err(|e| e.to_string())?;
        let mut json = tempfile::NamedTempFile::new_in(&out).map_err(|e| e.to_string())?;
        serde_json::to_writer(json.as_file_mut(), &meta).map_err(|e| e.to_string())?;
        json.persist(out.join(format!("{name}.json")))
            .map_err(|e| e.to_string())?;
        eprintln!(
            "[{}/{}] {name}: atlas {}x{}, {:.1} MiB decoded, visible pixels verified",
            index + 1,
            all.len(),
            atlas_width,
            atlas_height,
            meta.bytes() as f64 / 1048576.0
        );
    }
    Ok(())
}
fn pack_frames(frames: &mut [FrameMeta]) -> Result<(u32, u32), String> {
    let area: u64 = frames
        .iter()
        .map(|f| u64::from(f.width) * u64::from(f.height))
        .sum();
    let side = (area as f64).sqrt().ceil() as u32;
    let width = side.max(frames.iter().map(|f| f.width).max().unwrap_or(1));
    let mut order: Vec<usize> = (0..frames.len()).collect();
    order.sort_unstable_by_key(|&i| std::cmp::Reverse(frames[i].height));
    let (mut x, mut y, mut row_height) = (0u32, 0u32, 0u32);
    for index in order {
        let frame = &mut frames[index];
        if x + frame.width > width {
            y = y
                .checked_add(row_height)
                .ok_or("animation atlas is too tall")?;
            x = 0;
            row_height = 0;
        }
        frame.atlas_x = x;
        frame.atlas_y = y;
        x += frame.width;
        row_height = row_height.max(frame.height);
    }
    let height = y
        .checked_add(row_height)
        .ok_or("animation atlas is too tall")?;
    if width > 16_383 || height > 16_383 {
        return Err(format!(
            "animation atlas {width}x{height} exceeds WebP limits"
        ));
    }
    Ok((width, height))
}
fn verify_crop(original: &RgbaImage, cropped: &RgbaImage, m: &Metadata) -> Result<(), String> {
    if cropped.dimensions() != (m.atlas_width, m.atlas_height) {
        return Err("crop dimensions changed".into());
    }
    for (index, frame) in m.frames.iter().enumerate() {
        let (col, row) = (index as u32 % COLS, index as u32 / COLS);
        for y in 0..FRAME_H {
            for x in 0..FRAME_W {
                let source = original.get_pixel(col * FRAME_W + x, row * FRAME_H + y);
                let inside = x >= frame.x
                    && x < frame.x + frame.width
                    && y >= frame.y
                    && y < frame.y + frame.height;
                if !inside {
                    if source[3] != 0 {
                        return Err("crop removed visible pixels".into());
                    }
                    continue;
                }
                let pixel =
                    cropped.get_pixel(frame.atlas_x + x - frame.x, frame.atlas_y + y - frame.y);
                if source[3] != pixel[3] || (source[3] != 0 && source != pixel) {
                    return Err(format!(
                        "lossless verification failed at frame {index}, {x},{y}"
                    ));
                }
            }
        }
    }
    Ok(())
}
/// A cleaned public asset already includes the Codex repairs. Legacy media
/// without a status marker still needs the runtime compatibility fixes.
fn cleaned_assets(dir: &Path) -> Result<bool, String> {
    let path = dir
        .parent()
        .ok_or("optimized directory has no media parent")?
        .join("ASSETS.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    #[derive(Deserialize)]
    struct Status {
        status: String,
    }
    let status: Status =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    match status.status.as_str() {
        "cleaned" => Ok(true),
        "raw" => Ok(false),
        _ => Err(format!("{}: unsupported asset status", path.display())),
    }
}

/// Restore only the largest visible body's interior. Keep the exterior AA
/// band, real transparent holes, detached effects and all original RGB.
fn repair_body_matte(image: &mut RgbaImage, frame: &FrameMeta) {
    let (w, h) = (frame.width as usize, frame.height as usize);
    if w < 5 || h < 5 {
        return;
    }
    let alpha: Vec<u8> = (0..w * h)
        .map(|pos| {
            image.get_pixel(
                frame.atlas_x + (pos % w) as u32,
                frame.atlas_y + (pos / w) as u32,
            )[3]
        })
        .collect();
    if alpha.iter().all(|&a| a == 0 || a == 255) {
        return;
    }
    let mut seen = vec![false; w * h];
    let mut body = Vec::new();
    let mut region = Vec::new();
    for start in 0..w * h {
        if seen[start] || alpha[start] == 0 {
            continue;
        }
        region.clear();
        region.push(start);
        seen[start] = true;
        let mut cursor = 0;
        while cursor < region.len() {
            let pos = region[cursor];
            cursor += 1;
            let (x, y) = ((pos % w) as i32, (pos / w) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let next = ny as usize * w + nx as usize;
                    if alpha[next] != 0 && !seen[next] {
                        seen[next] = true;
                        region.push(next);
                    }
                }
            }
        }
        if region.len() > body.len() {
            std::mem::swap(&mut body, &mut region);
        }
    }
    for pos in body {
        if alpha[pos] == 255 {
            continue;
        }
        let (x, y) = (pos % w, pos / w);
        if x < 2 || y < 2 || x + 2 >= w || y + 2 >= h {
            continue;
        }
        if (y - 2..=y + 2).all(|ny| (x - 2..=x + 2).all(|nx| alpha[ny * w + nx] != 0)) {
            image.get_pixel_mut(frame.atlas_x + x as u32, frame.atlas_y + y as u32)[3] = 255;
        }
    }
}

/// Ease a clip's final second into the first frame's brightness. Codex's
/// legacy source sheets drift darker over each 10-second loop, so restarting at
/// frame zero made the whole character flash brighter. Only RGB changes;
/// pose, silhouette, alpha and the original asset files stay untouched.
fn blend_loop_seam(image: &mut RgbaImage, meta: &Metadata) {
    const TAIL: usize = 12;
    if meta.frames.len() <= TAIL {
        return;
    }
    let first = &meta.frames[0];
    let last = &meta.frames[meta.frames.len() - 1];
    let left = first.x.max(last.x);
    let top = first.y.max(last.y);
    let right = (first.x + first.width).min(last.x + last.width);
    let bottom = (first.y + first.height).min(last.y + last.height);
    if right <= left || bottom <= top {
        return;
    }
    let mut first_sum = [0u64; 3];
    let mut last_sum = [0u64; 3];
    let mut samples = 0u64;
    for y in (top..bottom).step_by(2) {
        for x in (left..right).step_by(2) {
            let a = image.get_pixel(first.atlas_x + x - first.x, first.atlas_y + y - first.y);
            let b = image.get_pixel(last.atlas_x + x - last.x, last.atlas_y + y - last.y);
            if a[3] == 0 || b[3] == 0 {
                continue;
            }
            for c in 0..3 {
                first_sum[c] += u64::from(a[c]);
                last_sum[c] += u64::from(b[c]);
            }
            samples += 1;
        }
    }
    if samples < 64 {
        return;
    }
    let mut gain = [1.0f32; 3];
    for c in 0..3 {
        let first_mean = first_sum[c] as f32 / samples as f32;
        let last_mean = last_sum[c] as f32 / samples as f32;
        if last_mean < 1.0 {
            return;
        }
        gain[c] = (first_mean / last_mean).clamp(0.85, 1.15);
    }
    let min_gain = gain.iter().copied().fold(f32::INFINITY, f32::min);
    let max_gain = gain.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max_gain - min_gain > 0.04 || gain.iter().all(|g| (g - 1.0).abs() < 0.004) {
        return;
    }
    for index in meta.frames.len() - TAIL..meta.frames.len() {
        let progress = (index + TAIL + 1 - meta.frames.len()) as f32 / TAIL as f32;
        let weight = progress * progress * (3.0 - 2.0 * progress);
        let frame = meta.frames[index].clone();
        for y in 0..frame.height {
            for x in 0..frame.width {
                let pixel = image.get_pixel_mut(frame.atlas_x + x, frame.atlas_y + y);
                if pixel[3] == 0 {
                    continue;
                }
                for c in 0..3 {
                    let scale = 1.0 + (gain[c] - 1.0) * weight;
                    pixel[c] = (f32::from(pixel[c]) * scale).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}
struct Anim {
    image: RgbaImage,
    meta: Metadata,
}
impl Anim {
    fn decode(dir: &Path, name: &str, meta: Metadata) -> Result<Self, String> {
        let path = dir.join(format!("{name}.webp"));
        if image::image_dimensions(&path).map_err(|e| e.to_string())?
            != (meta.atlas_width, meta.atlas_height)
        {
            return Err(format!(
                "{name}: crop dimensions do not match metadata; run --prepare-assets"
            ));
        }
        let mut image = read_image(&path)?;
        let needs_codex_repairs = env!("CARGO_PKG_NAME") == "codex-girl" && !cleaned_assets(dir)?;
        if needs_codex_repairs {
            for frame in &meta.frames {
                repair_body_matte(&mut image, frame);
            }
            blend_loop_seam(&mut image, &meta);
        }
        Ok(Self { image, meta })
    }
    fn draw(&self, index: u32, mirror: bool, shift: i32, canvas: &mut Canvas) {
        canvas.buf.fill(0);
        let frame = &self.meta.frames[index as usize];
        for y in 0..frame.height {
            for x in 0..frame.width {
                let p = self.image.get_pixel(frame.atlas_x + x, frame.atlas_y + y).0;
                let sx = if mirror {
                    FRAME_W - 1 - (frame.x + x)
                } else {
                    frame.x + x
                };
                let dx = PAD as i32 + shift + sx as i32;
                let dy = PAD + frame.y + y;
                if dx < 0 || dx >= canvas.width as i32 || dy >= canvas.height {
                    continue;
                }
                let offset = (dy * canvas.width + dx as u32) as usize * 4;
                canvas.buf[offset..offset + 4].copy_from_slice(&premultiply(p));
            }
        }
    }
}
pub fn premultiply(p: [u8; 4]) -> [u8; 4] {
    let a = p[3] as u16;
    [
        ((p[0] as u16 * a + 127) / 255) as u8,
        ((p[1] as u16 * a + 127) / 255) as u8,
        ((p[2] as u16 * a + 127) / 255) as u8,
        p[3],
    ]
}
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub buf: Vec<u8>,
}
impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            buf: vec![0; (width * height * 4) as usize],
        }
    }
    pub fn resize(&self, width: u32, height: u32) -> Canvas {
        let source = image::RgbaImage::from_raw(self.width, self.height, self.buf.clone()).unwrap();
        let resized = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        );
        let mut buf = resized.into_raw();
        for p in buf.as_chunks_mut::<4>().0 {
            for c in 0..3 {
                p[c] = p[c].min(p[3]);
            }
        }
        Canvas { width, height, buf }
    }
}
type DecodeResult = (String, Result<Anim, String>);
/// One in-flight request, no hidden queue; all cached pixels share one budget.
pub struct Sheet {
    metadata: HashMap<String, Metadata>,
    anims: HashMap<String, Anim>,
    order: VecDeque<String>,
    failed: HashSet<String>,
    pending: Option<String>,
    tx: SyncSender<(String, Metadata)>,
    rx: Receiver<DecodeResult>,
    budget: usize,
    happy: String,
    last: Option<(String, u32, bool, i32)>,
}
impl Sheet {
    pub fn load(media: &Path, budget_mib: usize) -> Result<Self, String> {
        let mut metadata = HashMap::new();
        for name in names(media)? {
            metadata.insert(name.clone(), load_metadata(media, &name)?);
        }
        let dir = media.join("optimized");
        let (tx, requests) = mpsc::sync_channel::<(String, Metadata)>(1);
        let (results, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("pet-decode".into())
            .spawn(move || {
                while let Ok((name, meta)) = requests.recv() {
                    let value = Anim::decode(&dir, &name, meta);
                    if results.send((name, value)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            metadata,
            anims: HashMap::new(),
            order: VecDeque::new(),
            failed: HashSet::new(),
            pending: None,
            tx,
            rx,
            budget: budget_mib * 1048576,
            happy: crate::character::pose_anim("happy").unwrap().into(),
            last: None,
        })
    }
    pub fn invalidate(&mut self) {
        self.last = None;
    }
    pub fn set_budget(&mut self, mib: usize) {
        self.budget = mib * 1048576;
        let reserved = self
            .pending
            .as_ref()
            .and_then(|n| self.metadata.get(n))
            .map_or(0, Metadata::bytes);
        while self.bytes() + reserved > self.budget {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            self.anims.remove(&old);
        }
    }
    /// Failed clips do not stop the clock indefinitely. They are logged once.
    pub fn ready(&mut self, pose: &str) -> bool {
        let name = crate::character::pose_anim(pose).unwrap_or(pose);
        if let Ok((loaded, result)) = self.rx.try_recv() {
            self.pending = None;
            match result {
                Ok(anim) if loaded == name || loaded == self.happy => {
                    self.order.push_back(loaded.clone());
                    self.anims.insert(loaded, anim);
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("animation {loaded}: {e}");
                    self.failed.insert(loaded);
                }
            }
        }
        if self.failed.contains(name) {
            return true;
        }
        if self.anims.contains_key(name) {
            self.order.retain(|n| n != name);
            self.order.push_back(name.into());
            return true;
        }
        let Some(meta) = self.metadata.get(name).cloned() else {
            if self.failed.insert(name.into()) {
                eprintln!("animation {name}: missing from library");
            }
            return true;
        };
        if self.pending.is_none() {
            if meta.bytes() > self.budget {
                eprintln!("animation {name}: exceeds cache budget");
                self.failed.insert(name.into());
                return true;
            }
            while self.bytes() + meta.bytes() > self.budget {
                let victim = self
                    .order
                    .iter()
                    .position(|n| n != &self.happy)
                    .unwrap_or(0);
                let Some(old) = self.order.remove(victim) else {
                    break;
                };
                self.anims.remove(&old);
            }
            if self.tx.send((name.into(), meta)).is_err() {
                eprintln!("animation decoder stopped");
                self.failed.insert(name.into());
                return true;
            }
            self.pending = Some(name.into());
        }
        false
    }
    pub fn bytes(&self) -> usize {
        self.anims.values().map(|a| a.meta.bytes()).sum()
    }
    pub fn draw(
        &mut self,
        pose: &str,
        elapsed: u64,
        mirror: bool,
        shift: i32,
        canvas: &mut Canvas,
    ) -> bool {
        let name = crate::character::pose_anim(pose).unwrap_or(pose);
        let index = ((elapsed * FPS / 1000) % (COLS * ROWS) as u64) as u32;
        let Some(anim) = self.anims.get(name) else {
            return false;
        };
        let frame = &anim.meta.frames[index as usize];
        // Edge alignment must never cut off wide props or effects. Mirroring
        // changes the frame's origin, but not its original frame coordinates.
        let left = if mirror {
            FRAME_W - frame.x - frame.width
        } else {
            frame.x
        };
        let shift = shift.clamp(-(left as i32), (FRAME_W - left - frame.width) as i32);
        let key = (name.to_owned(), index, mirror, shift);
        if self.last.as_ref() == Some(&key) {
            return false;
        }
        anim.draw(index, mirror, shift, canvas);
        self.last = Some(key);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("anim")).unwrap();
        std::fs::create_dir(dir.path().join("optimized")).unwrap();
        for (name, color) in [("a", 10), ("b", 20), ("c", 30)] {
            let source = dir.path().join("anim").join(format!("{name}.webp"));
            std::fs::write(&source, b"fingerprint fixture").unwrap();
            let (source_bytes, source_modified_ns) = fingerprint(&source).unwrap();
            let m = Metadata {
                version: 2,
                source_bytes,
                source_modified_ns,
                atlas_width: COLS,
                atlas_height: ROWS,
                frames: (0..COLS * ROWS)
                    .map(|index| FrameMeta {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                        atlas_x: index % COLS,
                        atlas_y: index / COLS,
                    })
                    .collect(),
            };
            let image = RgbaImage::from_pixel(COLS, ROWS, image::Rgba([color, 0, 0, 255]));
            image
                .save(dir.path().join("optimized").join(format!("{name}.webp")))
                .unwrap();
            std::fs::write(
                dir.path().join("optimized").join(format!("{name}.json")),
                serde_json::to_vec(&m).unwrap(),
            )
            .unwrap();
        }
        dir
    }
    #[test]
    fn body_matte_patches_are_opaque_but_edges_and_holes_stay_transparent() {
        let mut im = RgbaImage::from_pixel(9, 9, image::Rgba([240, 230, 245, 255]));
        im.put_pixel(4, 4, image::Rgba([240, 230, 245, 214]));
        im.put_pixel(5, 4, image::Rgba([240, 230, 245, 93]));
        im.put_pixel(0, 3, image::Rgba([240, 230, 245, 93]));
        im.put_pixel(1, 7, image::Rgba([240, 230, 245, 0]));
        im.put_pixel(2, 7, image::Rgba([240, 230, 245, 93]));
        let frame = FrameMeta {
            x: 0,
            y: 0,
            width: 9,
            height: 9,
            atlas_x: 0,
            atlas_y: 0,
        };
        repair_body_matte(&mut im, &frame);
        assert_eq!(im.get_pixel(4, 4).0, [240, 230, 245, 255]);
        assert_eq!(im.get_pixel(5, 4)[3], 255);
        assert_eq!(im.get_pixel(0, 3)[3], 93);
        assert_eq!(im.get_pixel(1, 7)[3], 0);
        assert_eq!(im.get_pixel(2, 7)[3], 93);
    }

    #[test]
    fn detached_translucent_effect_is_not_made_opaque() {
        let mut im = RgbaImage::new(24, 12);
        for y in 1..11 {
            for x in 1..11 {
                im.put_pixel(x, y, image::Rgba([200, 180, 160, 255]));
            }
        }
        for y in 2..9 {
            for x in 16..23 {
                im.put_pixel(x, y, image::Rgba([255, 255, 255, 93]));
            }
        }
        im.put_pixel(5, 5, image::Rgba([200, 180, 160, 93]));
        let f = FrameMeta {
            x: 0,
            y: 0,
            width: 24,
            height: 12,
            atlas_x: 0,
            atlas_y: 0,
        };
        repair_body_matte(&mut im, &f);
        assert_eq!(im.get_pixel(5, 5)[3], 255);
        assert_eq!(im.get_pixel(19, 5)[3], 93);
    }

    #[test]
    fn loop_seam_brightens_only_the_tail_and_preserves_shape_and_alpha() {
        let mut image = RgbaImage::new(16 * 120, 16);
        let mut frames = Vec::new();
        for index in 0..120 {
            let value = if index == 0 { 220 } else { 200 };
            for y in 0..16 {
                for x in 0..16 {
                    image.put_pixel(
                        index * 16 + x,
                        y,
                        image::Rgba([value, value - 10, value - 20, 200]),
                    );
                }
            }
            frames.push(FrameMeta {
                x: 0,
                y: 0,
                width: 16,
                height: 16,
                atlas_x: index * 16,
                atlas_y: 0,
            });
        }
        let meta = Metadata {
            version: 2,
            source_bytes: 0,
            source_modified_ns: 0,
            atlas_width: 16 * 120,
            atlas_height: 16,
            frames,
        };
        let alpha_before: Vec<u8> = image.pixels().map(|p| p[3]).collect();
        blend_loop_seam(&mut image, &meta);
        assert_eq!(image.get_pixel(0, 0).0, [220, 210, 200, 200]);
        assert_eq!(image.get_pixel(107 * 16, 0).0, [200, 190, 180, 200]);
        let tail = image.get_pixel(119 * 16, 0);
        assert!((tail[0] as i16 - 220).abs() <= 1, "tail {tail:?}");
        assert_eq!(
            alpha_before,
            image.pixels().map(|p| p[3]).collect::<Vec<_>>()
        );
    }

    #[test]
    fn loop_seam_does_not_guess_when_endpoints_share_no_visible_pixels() {
        let mut image = RgbaImage::from_pixel(8, 4, image::Rgba([220, 210, 200, 255]));
        for y in 0..4 {
            for x in 4..8 {
                image.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
            }
        }
        let frames = (0..120)
            .map(|index| FrameMeta {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
                atlas_x: if index == 0 { 0 } else { 4 },
                atlas_y: 0,
            })
            .collect();
        let meta = Metadata {
            version: 2,
            source_bytes: 0,
            source_modified_ns: 0,
            atlas_width: 8,
            atlas_height: 4,
            frames,
        };
        let before = image.clone();
        blend_loop_seam(&mut image, &meta);
        assert_eq!(image, before);
    }

    fn matte_fixture() -> (tempfile::TempDir, Metadata, RgbaImage) {
        let dir = library();
        let mut meta = load_metadata(dir.path(), "a").unwrap();
        meta.atlas_width = 15 * 120;
        meta.atlas_height = 15;
        let mut image = RgbaImage::new(meta.atlas_width, meta.atlas_height);
        for (index, frame) in meta.frames.iter_mut().enumerate() {
            *frame = FrameMeta {
                x: 0,
                y: 0,
                width: 15,
                height: 15,
                atlas_x: index as u32 * 15,
                atlas_y: 0,
            };
            for y in 3..12 {
                for x in 3..12 {
                    let alpha = if y == 3 {
                        44
                    } else if (6..9).contains(&x) && (6..9).contains(&y) {
                        93
                    } else {
                        255
                    };
                    image.put_pixel(frame.atlas_x + x, y, image::Rgba([50, 65, 120, alpha]));
                }
            }
        }
        image.save(dir.path().join("optimized/a.webp")).unwrap();
        std::fs::write(
            dir.path().join("optimized/a.json"),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        (dir, meta, image)
    }

    #[test]
    fn codex_raw_matte_repairs_interiors_but_preserves_edges() {
        if env!("CARGO_PKG_NAME") != "codex-girl" {
            return;
        }
        let (dir, meta, _) = matte_fixture();
        let anim = Anim::decode(&dir.path().join("optimized"), "a", meta).unwrap();
        assert_eq!(anim.image.get_pixel(7, 7)[3], 255);
        assert_eq!(anim.image.get_pixel(7, 3)[3], 44);
        assert_eq!(anim.image.get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn cleaned_assets_are_not_repaired_again() {
        let (dir, meta, image) = matte_fixture();
        std::fs::write(dir.path().join("ASSETS.json"), r#"{"status":"cleaned"}"#).unwrap();
        let anim = Anim::decode(&dir.path().join("optimized"), "a", meta).unwrap();
        assert_eq!(anim.image, image);
    }

    #[test]
    fn invalid_asset_status_does_not_trigger_codex_repairs() {
        let (dir, meta, _) = matte_fixture();
        std::fs::write(dir.path().join("ASSETS.json"), "{broken").unwrap();
        let result = Anim::decode(&dir.path().join("optimized"), "a", meta);
        assert_eq!(result.is_err(), env!("CARGO_PKG_NAME") == "codex-girl");
    }

    #[test]
    #[ignore = "requires repository Codex assets"]
    fn codex_running_hair_does_not_show_the_desktop_through_matte_patches() {
        if env!("CARGO_PKG_NAME") != "codex-girl" {
            return;
        }
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("media");
        let meta = load_metadata(&media, "奔跑").unwrap();
        let anim = Anim::decode(&media.join("optimized"), "奔跑", meta).unwrap();
        let frame = &anim.meta.frames[15];
        for (x, y) in [(240, 72), (260, 95)] {
            assert_eq!(
                anim.image
                    .get_pixel(frame.atlas_x + x - frame.x, frame.atlas_y + y - frame.y)[3],
                255
            );
        }
        let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
        for index in 0..120 {
            anim.draw(index, false, 0, &mut canvas);
            assert!(
                canvas
                    .buf
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[..3].iter().all(|v| *v <= p[3]))
            );
        }
    }

    #[test]
    fn codex_raw_loop_blends_tail_without_touching_cleaned_assets() {
        if env!("CARGO_PKG_NAME") != "codex-girl" {
            return;
        }
        let dir = library();
        let mut meta = load_metadata(dir.path(), "a").unwrap();
        meta.atlas_width = 20 * 120;
        meta.atlas_height = 20;
        let mut image = RgbaImage::new(meta.atlas_width, meta.atlas_height);
        for (index, frame) in meta.frames.iter_mut().enumerate() {
            *frame = FrameMeta {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                atlas_x: index as u32 * 20,
                atlas_y: 0,
            };
            let value = if index == 0 { 220 } else { 200 };
            for y in 0..20 {
                for x in 0..20 {
                    let alpha = if x == 0 && y == 0 { 128 } else { 255 };
                    image.put_pixel(
                        frame.atlas_x + x,
                        y,
                        image::Rgba([value, value - 10, value - 20, alpha]),
                    );
                }
            }
        }
        image.save(dir.path().join("optimized/a.webp")).unwrap();
        std::fs::write(dir.path().join("ASSETS.json"), r#"{"status":"raw"}"#).unwrap();
        let raw = Anim::decode(&dir.path().join("optimized"), "a", meta.clone()).unwrap();
        assert_eq!(raw.image.get_pixel(119 * 20 + 7, 7)[0], 220);
        assert_eq!(raw.image.get_pixel(107 * 20 + 7, 7)[0], 200);
        assert_eq!(raw.image.get_pixel(0, 0)[3], 128);
        std::fs::write(dir.path().join("ASSETS.json"), r#"{"status":"cleaned"}"#).unwrap();
        let cleaned = Anim::decode(&dir.path().join("optimized"), "a", meta).unwrap();
        assert_eq!(cleaned.image, image);
    }

    fn await_clip(sheet: &mut Sheet, name: &str) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !sheet.ready(name) {
            assert!(std::time::Instant::now() < end, "decoder did not finish");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[test]
    fn cache_evicts_before_decode_and_never_exceeds_reserved_budget() {
        let dir = library();
        let mut sheet = Sheet::load(dir.path(), 128).unwrap();
        sheet.budget = 960; // exactly two tiny clips
        await_clip(&mut sheet, "a");
        await_clip(&mut sheet, "b");
        assert_eq!(sheet.bytes(), 960);
        assert!(!sheet.ready("c"));
        assert_eq!(sheet.bytes(), 480);
        await_clip(&mut sheet, "c");
        assert_eq!(sheet.bytes(), 960);
        assert!(!sheet.ready("a")); // least-recent clip was evicted
        await_clip(&mut sheet, "a");
        assert_eq!(sheet.bytes(), 960);
    }
    #[test]
    fn superseded_decode_does_not_replace_the_requested_clip() {
        let dir = library();
        let mut sheet = Sheet::load(dir.path(), 128).unwrap();
        sheet.ready("a");
        await_clip(&mut sheet, "c");
        assert_eq!(sheet.bytes(), 480);
        let mut c = Canvas::new(WINDOW_W, WINDOW_H);
        assert!(sheet.draw("c", 0, false, 0, &mut c));
        assert_eq!(c.buf[((PAD * WINDOW_W + PAD) * 4) as usize], 30);
        assert!(!sheet.draw("c", 20, false, 0, &mut c)); // still the same 12fps frame
    }
    #[test]
    fn edge_alignment_preserves_visible_pixels_in_both_directions() {
        let dir = library();
        let mut sheet = Sheet::load(dir.path(), 128).unwrap();
        await_clip(&mut sheet, "a"); // visible pixel starts at original x=0
        let mut c = Canvas::new(WINDOW_W, WINDOW_H);
        assert!(sheet.draw("a", 0, false, -ART_SHIFT, &mut c));
        assert_eq!(
            c.buf
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count(),
            1
        );
        assert!(sheet.draw("a", 0, true, ART_SHIFT, &mut c));
        assert_eq!(
            c.buf
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count(),
            1
        );
    }
    #[test]
    fn packed_frames_stay_in_bounds_and_do_not_overlap() {
        let mut frames: Vec<FrameMeta> = (0..COLS * ROWS)
            .map(|index| FrameMeta {
                x: 0,
                y: 0,
                width: 80 + index % 47,
                height: 60 + (index * 7) % 53,
                atlas_x: 0,
                atlas_y: 0,
            })
            .collect();
        let (width, height) = pack_frames(&mut frames).unwrap();
        for (i, a) in frames.iter().enumerate() {
            assert!(a.atlas_x + a.width <= width);
            assert!(a.atlas_y + a.height <= height);
            for b in &frames[i + 1..] {
                let disjoint = a.atlas_x + a.width <= b.atlas_x
                    || b.atlas_x + b.width <= a.atlas_x
                    || a.atlas_y + a.height <= b.atlas_y
                    || b.atlas_y + b.height <= a.atlas_y;
                assert!(disjoint, "frame rectangles overlap");
            }
        }
    }
    #[test]
    fn metadata_rejects_a_frame_larger_than_its_atlas() {
        let mut metadata = Metadata {
            version: 2,
            source_bytes: 0,
            source_modified_ns: 0,
            atlas_width: 1,
            atlas_height: 1,
            frames: (0..COLS * ROWS)
                .map(|_| FrameMeta {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                    atlas_x: 0,
                    atlas_y: 0,
                })
                .collect(),
        };
        assert!(metadata.validate().is_err());
        metadata.atlas_width = 2;
        assert!(metadata.validate().is_ok());
    }
    #[test]
    fn broken_clip_does_not_freeze_clock_or_overwrite_previous_pixels() {
        let dir = library();
        let mut sheet = Sheet::load(dir.path(), 128).unwrap();
        await_clip(&mut sheet, "a");
        let mut c = Canvas::new(WINDOW_W, WINDOW_H);
        sheet.draw("a", 0, false, 0, &mut c);
        let previous = c.buf.clone();
        std::fs::write(dir.path().join("optimized/b.webp"), b"broken").unwrap();
        await_clip(&mut sheet, "b");
        assert!(!sheet.draw("b", 0, false, 0, &mut c));
        assert_eq!(c.buf, previous);
        assert!(sheet.ready("b"));
    }
    #[test]
    fn half_transparent_edges_are_not_full_brightness() {
        assert_eq!(premultiply([255, 128, 0, 128]), [128, 64, 0, 128]);
        assert_eq!(premultiply([255, 100, 80, 0]), [0, 0, 0, 0]);
        assert_eq!(premultiply([17, 45, 200, 255]), [17, 45, 200, 255]);
    }
    #[test]
    fn crop_mirroring_keeps_original_frame_coordinates() {
        let m = Metadata {
            version: 2,
            source_bytes: 0,
            source_modified_ns: 0,
            atlas_width: COLS,
            atlas_height: ROWS,
            frames: (0..COLS * ROWS)
                .map(|index| FrameMeta {
                    x: 200,
                    y: 50,
                    width: 1,
                    height: 1,
                    atlas_x: index % COLS,
                    atlas_y: index / COLS,
                })
                .collect(),
        };
        let a = Anim {
            image: RgbaImage::from_pixel(COLS, ROWS, image::Rgba([255, 100, 0, 128])),
            meta: m,
        };
        let mut c = Canvas::new(WINDOW_W, WINDOW_H);
        a.draw(119, true, 0, &mut c);
        let offset = ((56 * WINDOW_W + 445) * 4) as usize;
        assert_eq!(&c.buf[offset..offset + 4], &[128, 50, 0, 128]);
        assert_eq!(
            c.buf
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] != 0)
                .count(),
            1
        );
    }
    #[test]
    fn resizing_transparency_preserves_premultiplied_invariant() {
        let mut c = Canvas::new(2, 1);
        c.buf = vec![128, 64, 0, 128, 0, 0, 0, 0];
        let out = c.resize(9, 3);
        assert!(
            out.buf
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
        );
    }
}
