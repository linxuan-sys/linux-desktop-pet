//! Original WebP animation decoding and premultiplied rendering.
#[cfg(test)]
use image::RgbaImage;
use serde::Deserialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

#[path = "raw.rs"]
mod raw;

pub const FRAME_W: u32 = 640;
pub const FRAME_H: u32 = 360;
pub const PAD: u32 = 6;
pub const ART_SHIFT: i32 = 148;
pub const WINDOW_W: u32 = FRAME_W + PAD * 2;
pub const WINDOW_H: u32 = FRAME_H + PAD * 2;
const COLS: u32 = 8;
const ROWS: u32 = 15;
pub(super) const COUNT: usize = (COLS * ROWS) as usize;
pub const FPS: u64 = 12;

#[derive(Clone, Debug)]
struct Metadata {
    atlas_width: u32,
    atlas_height: u32,
    frames: Vec<FrameMeta>,
}

#[derive(Clone, Debug)]
struct FrameMeta {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    atlas_x: u32,
    atlas_y: u32,
}

#[derive(Clone, Debug)]
struct Entry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct Anim<C: std::ops::Deref<Target = [u8]> = Vec<u8>> {
    image: image::ImageBuffer<image::Rgba<u8>, C>,
    meta: Metadata,
}

impl<C: std::ops::Deref<Target = [u8]>> Anim<C> {
    #[cfg(test)]
    fn draw(&self, index: u32, mirror: bool, shift: i32, do_premul: bool, canvas: &mut Canvas) {
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
                if p[3] == 0 {
                    continue;
                }
                let out = if do_premul { premultiply(p) } else { p };
                canvas.buf[offset..offset + 4].copy_from_slice(&out);
            }
        }
    }
}

pub(super) struct Frame {
    name: String,
    index: u32,
    entry: Entry,
    pixels: Vec<u8>,
}

impl Frame {
    fn draw(&self, mirror: bool, shift: i32, premul: bool, canvas: &mut Canvas) {
        canvas.buf.fill(0);
        let entry = &self.entry;
        for y in 0..entry.height {
            for x in 0..entry.width {
                let offset = ((y * entry.width + x) * 4) as usize;
                let pixel: [u8; 4] = self.pixels[offset..offset + 4].try_into().unwrap();
                if pixel[3] == 0 {
                    continue;
                }
                let sx = if mirror {
                    FRAME_W - 1 - entry.x - x
                } else {
                    entry.x + x
                };
                let dx = PAD as i32 + shift + sx as i32;
                let dy = PAD + entry.y + y;
                if dx < 0 || dx >= canvas.width as i32 || dy >= canvas.height {
                    continue;
                }
                let dest = ((dy * canvas.width + dx as u32) * 4) as usize;
                canvas.buf[dest..dest + 4].copy_from_slice(&if premul {
                    premultiply(pixel)
                } else {
                    pixel
                });
            }
        }
    }
}

/// One bounded decode request at a time. The worker keeps compressed cropped
/// frames under the configured budget and never retains the full source sheet.
pub struct Sheet {
    names: HashSet<String>,
    tx: SyncSender<(String, u32)>,
    rx: Receiver<(String, u32, Result<Frame, String>)>,
    pending: bool,
    current: Option<Frame>,
    failed: HashSet<String>,
    premultiply: bool,
    last: Option<(String, u32, bool, i32)>,
    published_bytes: Arc<AtomicUsize>,
    budget: Arc<AtomicUsize>,
}

impl Sheet {
    pub fn load(media: &Path, budget_mib: usize) -> Result<Self, String> {
        let names = names(media)?.into_iter().collect();
        let media = media.to_path_buf();
        let budget = Arc::new(AtomicUsize::new(budget_mib * 1048576));
        let published_bytes = Arc::new(AtomicUsize::new(0));
        let (tx, requests) = mpsc::sync_channel::<(String, u32)>(1);
        let (results, rx) = mpsc::sync_channel(1);
        let worker_budget = budget.clone();
        let worker_bytes = published_bytes.clone();
        let mut cache = raw::ClipCache::new(media, worker_budget, worker_bytes)?;
        std::thread::Builder::new()
            .name("pet-frame-decode".into())
            .spawn(move || {
                while let Ok((name, index)) = requests.recv() {
                    let frame = cache.frame(&name, index);
                    if results.send((name, index, frame)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            names,
            tx,
            rx,
            pending: false,
            current: None,
            failed: HashSet::new(),
            premultiply: true,
            last: None,
            published_bytes,
            budget,
        })
    }

    fn receive(&mut self) {
        if let Ok((name, index, result)) = self.rx.try_recv() {
            self.pending = false;
            match result {
                Ok(frame) => {
                    debug_assert_eq!((&frame.name, frame.index), (&name, index));
                    self.current = Some(frame);
                }
                Err(error) => {
                    eprintln!("animation {name}: {error}");
                    self.failed.insert(name);
                }
            }
        }
    }

    fn request(&mut self, name: &str, index: u32) {
        if self.pending || self.failed.contains(name) {
            return;
        }
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

    pub fn invalidate(&mut self) {
        self.last = None;
    }

    pub fn set_premultiply(&mut self, on: bool) {
        if self.premultiply != on {
            self.premultiply = on;
            self.invalidate();
        }
    }

    pub fn set_budget(&mut self, mib: usize) {
        self.budget.store(mib * 1048576, Ordering::Relaxed);
    }

    pub fn bytes(&self) -> usize {
        self.published_bytes.load(Ordering::Relaxed)
            + self.current.as_ref().map_or(0, |frame| frame.pixels.len())
    }

    /// Wait only for the first frame of a pose before the runtime starts its
    /// clock. Subsequent frames are decoded on demand by `draw`.
    pub fn ready(&mut self, pose: &str) -> bool {
        self.receive();
        let name = crate::character::pose_anim(pose).unwrap_or(pose);
        if self.failed.contains(name)
            || self
                .current
                .as_ref()
                .is_some_and(|frame| frame.name == name)
        {
            return true;
        }
        self.request(name, 0);
        self.failed.contains(name)
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
            .current
            .as_ref()
            .is_none_or(|frame| frame.name != name || frame.index != index)
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
        self.request(name, (index + 1) % COUNT as u32);
        true
    }
}

pub fn check_assets(media: &Path) -> Result<(), String> {
    raw::check_assets(media)
}

/// Headless real-time playback benchmark without creating a Wayland window.
pub fn benchmark_playback(media: &Path, seconds: u64) -> Result<(), String> {
    if !(1..=300).contains(&seconds) {
        return Err("benchmark duration must be 1..300 seconds".into());
    }
    let mut sheet = Sheet::load(media, 32)?;
    let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
    let wait = Instant::now();
    while !sheet.ready("idle") {
        if wait.elapsed().as_secs() > 30 {
            return Err("idle decode timeout".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let start = Instant::now();
    let mut drawn = 0u64;
    while start.elapsed().as_secs() < seconds {
        sheet.ready("idle");
        if sheet.draw(
            "idle",
            start.elapsed().as_millis() as u64,
            false,
            0,
            &mut canvas,
        ) {
            drawn += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    if drawn < seconds * FPS * 9 / 10 {
        return Err(format!(
            "playback too slow: {drawn} frame updates in {seconds}s"
        ));
    }
    println!(
        "{}: {drawn} frame updates in {:.2}s; {:.3} MiB cached frame data; full-resolution lossless playback, no Wayland window",
        crate::character::selected(),
        start.elapsed().as_secs_f64(),
        sheet.bytes() as f64 / 1048576.0
    );
    Ok(())
}

fn names(media: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(media.join("anim")).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("webp"))
        {
            names.push(
                path.file_stem()
                    .and_then(|stem| stem.to_str())
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

#[cfg(test)]
fn read_image(path: &Path) -> Result<image::RgbaImage, String> {
    image::ImageReader::open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .decode()
        .map_err(|error| format!("{}: {error}", path.display()))
        .map(|image| image.into_rgba8())
}

enum NativePixels {
    Native(webp::WebPImage),
    Rgba(Vec<u8>),
}

impl std::ops::Deref for NativePixels {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Native(pixels) => pixels,
            Self::Rgba(pixels) => pixels,
        }
    }
}

impl std::ops::DerefMut for NativePixels {
    fn deref_mut(&mut self) -> &mut [u8] {
        match self {
            Self::Native(pixels) => pixels,
            Self::Rgba(pixels) => pixels,
        }
    }
}

#[cfg(test)]
fn source_anim(media: &Path, name: &str) -> Result<Anim, String> {
    let path = media.join("anim").join(format!("{name}.webp"));
    if image::image_dimensions(&path).map_err(|error| format!("{}: {error}", path.display()))?
        != (FRAME_W * COLS, FRAME_H * ROWS)
    {
        return Err(format!(
            "{}: expected 5120x5400 source sheet",
            path.display()
        ));
    }
    process_source(media, read_image(&path)?)
}

fn native_source_anim(media: &Path, name: &str) -> Result<Anim<NativePixels>, String> {
    let path = media.join("anim").join(format!("{name}.webp"));
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
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
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
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
    for index in 0..COUNT as u32 {
        let (origin_x, origin_y) = (index % COLS * FRAME_W, index / COLS * FRAME_H);
        let (mut left, mut top, mut right, mut bottom) = (FRAME_W, FRAME_H, 0, 0);
        for y in 0..FRAME_H {
            for x in 0..FRAME_W {
                if image.get_pixel(origin_x + x, origin_y + y)[3] != 0 {
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
            atlas_x: origin_x + left,
            atlas_y: origin_y + top,
        });
    }
    let meta = Metadata {
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
    if crate::character::selected() == "codex-girl" && !cleaned_assets(media)? {
        for frame in &meta.frames {
            repair_body_matte(image, frame);
        }
        blend_loop_seam(image, meta);
    }
    Ok(())
}

#[derive(Deserialize)]
struct AssetStatus {
    status: String,
}

fn cleaned_assets(media: &Path) -> Result<bool, String> {
    let path = media.join("ASSETS.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let status: AssetStatus =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    match status.status.as_str() {
        "cleaned" => Ok(true),
        "raw" => Ok(false),
        _ => Err(format!("{}: unsupported asset status", path.display())),
    }
}

/// Restore only the largest visible body's interior. Keep the exterior AA
/// band, real transparent holes, detached effects and all original RGB.
fn repair_body_matte<C: std::ops::Deref<Target = [u8]> + std::ops::DerefMut>(
    image: &mut image::ImageBuffer<image::Rgba<u8>, C>,
    frame: &FrameMeta,
) {
    let (width, height) = (frame.width as usize, frame.height as usize);
    if width < 5 || height < 5 {
        return;
    }
    let alpha: Vec<u8> = (0..width * height)
        .map(|position| {
            image.get_pixel(
                frame.atlas_x + (position % width) as u32,
                frame.atlas_y + (position / width) as u32,
            )[3]
        })
        .collect();
    if alpha.iter().all(|&value| value == 0 || value == 255) {
        return;
    }
    let mut seen = vec![false; width * height];
    let mut body = Vec::new();
    let mut region = Vec::new();
    for start in 0..width * height {
        if seen[start] || alpha[start] == 0 {
            continue;
        }
        region.clear();
        region.push(start);
        seen[start] = true;
        let mut cursor = 0;
        while cursor < region.len() {
            let position = region[cursor];
            cursor += 1;
            let (x, y) = ((position % width) as i32, (position / width) as i32);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (next_x, next_y) = (x + dx, y + dy);
                    if next_x < 0 || next_y < 0 || next_x >= width as i32 || next_y >= height as i32
                    {
                        continue;
                    }
                    let next = next_y as usize * width + next_x as usize;
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
    for position in body {
        if alpha[position] == 255 {
            continue;
        }
        let (x, y) = (position % width, position / width);
        if x < 2 || y < 2 || x + 2 >= width || y + 2 >= height {
            continue;
        }
        if (y - 2..=y + 2)
            .all(|next_y| (x - 2..=x + 2).all(|next_x| alpha[next_y * width + next_x] != 0))
        {
            image.get_pixel_mut(frame.atlas_x + x as u32, frame.atlas_y + y as u32)[3] = 255;
        }
    }
}

/// Ease a clip's final second into the first frame's brightness. Only RGB
/// changes; pose, silhouette, alpha and source files remain untouched.
fn blend_loop_seam<C: std::ops::Deref<Target = [u8]> + std::ops::DerefMut>(
    image: &mut image::ImageBuffer<image::Rgba<u8>, C>,
    meta: &Metadata,
) {
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
            let first_pixel =
                image.get_pixel(first.atlas_x + x - first.x, first.atlas_y + y - first.y);
            let last_pixel = image.get_pixel(last.atlas_x + x - last.x, last.atlas_y + y - last.y);
            if first_pixel[3] == 0 || last_pixel[3] == 0 {
                continue;
            }
            for channel in 0..3 {
                first_sum[channel] += u64::from(first_pixel[channel]);
                last_sum[channel] += u64::from(last_pixel[channel]);
            }
            samples += 1;
        }
    }
    if samples < 64 {
        return;
    }
    let mut gain = [1.0f32; 3];
    for channel in 0..3 {
        let first_mean = first_sum[channel] as f32 / samples as f32;
        let last_mean = last_sum[channel] as f32 / samples as f32;
        if last_mean < 1.0 {
            return;
        }
        gain[channel] = (first_mean / last_mean).clamp(0.85, 1.15);
    }
    let min_gain = gain.iter().copied().fold(f32::INFINITY, f32::min);
    let max_gain = gain.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if max_gain - min_gain > 0.04
        || gain
            .iter()
            .all(|channel_gain| (channel_gain - 1.0).abs() < 0.004)
    {
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
                for channel in 0..3 {
                    let scale = 1.0 + (gain[channel] - 1.0) * weight;
                    pixel[channel] = (f32::from(pixel[channel]) * scale)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// Straight alpha in, premultiplied out, rounded.
pub fn premultiply(pixel: [u8; 4]) -> [u8; 4] {
    let alpha = pixel[3] as u16;
    [
        ((pixel[0] as u16 * alpha + 127) / 255) as u8,
        ((pixel[1] as u16 * alpha + 127) / 255) as u8,
        ((pixel[2] as u16 * alpha + 127) / 255) as u8,
        pixel[3],
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
        for pixel in buf.as_chunks_mut::<4>().0 {
            for channel in 0..3 {
                pixel[channel] = pixel[channel].min(pixel[3]);
            }
        }
        Canvas { width, height, buf }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_transparent_edges_are_not_full_brightness() {
        assert_eq!(premultiply([255, 128, 0, 128]), [128, 64, 0, 128]);
        assert_eq!(premultiply([255, 100, 80, 0]), [0, 0, 0, 0]);
        assert_eq!(premultiply([17, 45, 200, 255]), [17, 45, 200, 255]);
    }

    #[test]
    fn resizing_transparency_preserves_premultiplied_invariant() {
        let mut canvas = Canvas::new(2, 1);
        canvas.buf = vec![128, 64, 0, 128, 0, 0, 0, 0];
        let out = canvas.resize(9, 3);
        assert!(
            out.buf
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3])
        );
    }
}
