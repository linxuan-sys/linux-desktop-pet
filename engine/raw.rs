//! Original WebP on disk, cropped lossless frame cache in RAM.
use super::*;
use std::path::PathBuf;

fn release_unused_decode_memory() {
    // glibc's worker-thread arena retains source-decoder scratch after free.
    // Return unused pages after cache changes, never on each displayed frame.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim takes no pointers and supports concurrent callers.
    unsafe {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> i32;
        }
        malloc_trim(0);
    }
}

struct Clip {
    entries: Vec<Entry>,
    frames: Vec<Vec<u8>>,
    bytes: usize,
}
impl Clip {
    fn load(media: &Path, name: &str) -> Result<Self, String> {
        // Preserve existing alpha, skip cleaned assets, and repair legacy Codex
        // source sheets once while they are cropped into the in-memory cache.
        Self::from_anim(native_source_anim(media, name)?)
    }
    fn from_anim<C: std::ops::Deref<Target = [u8]> + 'static>(
        anim: Anim<C>,
    ) -> Result<Self, String> {
        let mut encoder = zstd::bulk::Compressor::new(1).map_err(|e| e.to_string())?;
        let mut frames = Vec::with_capacity(COUNT);
        let mut entries = Vec::with_capacity(COUNT);
        let mut bytes = 0;
        for f in &anim.meta.frames {
            if f.width == 0
                || f.height == 0
                || f.width > anim.meta.atlas_width
                || f.height > anim.meta.atlas_height
                || f.atlas_x > anim.meta.atlas_width - f.width
                || f.atlas_y > anim.meta.atlas_height - f.height
            {
                return Err("invalid frame bounds".into());
            }
            let mut pixels =
                image::imageops::crop_imm(&anim.image, f.atlas_x, f.atlas_y, f.width, f.height)
                    .to_image()
                    .into_raw();
            // Alpha-zero RGB is never displayed; do not retain green-screen
            // residue there. All nonzero-alpha pixels remain byte-for-byte exact.
            for p in pixels.as_chunks_mut::<4>().0 {
                if p[3] == 0 {
                    p.fill(0);
                }
            }
            let mut encoded = encoder.compress(&pixels).map_err(|e| e.to_string())?;
            // zstd reserves compress_bound(raw size), which can be much larger
            // than the result. Keep only compressed storage in the hot cache.
            encoded.shrink_to_fit();
            bytes += encoded.capacity();
            entries.push(Entry {
                x: f.x,
                y: f.y,
                width: f.width,
                height: f.height,
            });
            frames.push(encoded);
        }
        Ok(Self {
            entries,
            frames,
            bytes,
        })
    }
    fn frame(
        &self,
        name: &str,
        index: u32,
        decoder: &mut zstd::bulk::Decompressor<'static>,
    ) -> Result<Frame, String> {
        let entry = self
            .entries
            .get(index as usize)
            .ok_or("frame index out of range")?
            .clone();
        let expected = (entry.width * entry.height * 4) as usize;
        let pixels = decoder
            .decompress(&self.frames[index as usize], expected)
            .map_err(|e| e.to_string())?;
        if pixels.len() != expected {
            return Err("invalid cached RGBA frame length".into());
        }
        Ok(Frame {
            name: name.into(),
            index,
            entry,
            pixels,
        })
    }
}

/// At most one source sheet is decoded at a time. Previous actions stay cropped
/// and compressed, with LRU eviction. One oversized active clip can stand alone.
pub(super) struct ClipCache {
    media: PathBuf,
    clips: HashMap<String, Clip>,
    order: VecDeque<String>,
    bytes: usize,
    budget: Arc<AtomicUsize>,
    published_bytes: Arc<AtomicUsize>,
    decoder: zstd::bulk::Decompressor<'static>,
    pub(super) loads: usize,
}
impl ClipCache {
    pub(super) fn new(
        media: PathBuf,
        budget: Arc<AtomicUsize>,
        published_bytes: Arc<AtomicUsize>,
    ) -> Result<Self, String> {
        Ok(Self {
            media,
            clips: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            budget,
            published_bytes,
            decoder: zstd::bulk::Decompressor::new().map_err(|e| e.to_string())?,
            loads: 0,
        })
    }
    fn evict(&mut self, protected: Option<&str>) -> bool {
        let Some(at) = self
            .order
            .iter()
            .position(|n| Some(n.as_str()) != protected)
        else {
            return false;
        };
        let name = self.order.remove(at).unwrap();
        if let Some(clip) = self.clips.remove(&name) {
            self.bytes -= clip.bytes;
        }
        self.published_bytes.store(self.bytes, Ordering::Relaxed);
        true
    }
    pub(super) fn frame(&mut self, name: &str, index: u32) -> Result<Frame, String> {
        if index >= COUNT as u32 {
            return Err("frame index out of range".into());
        }
        let budget = self.budget.load(Ordering::Relaxed);
        let changed = if !self.clips.contains_key(name) {
            // Make room BEFORE decoding a 105 MiB source, not afterwards.
            let reserve = budget.min(16 * 1048576);
            while self.bytes > budget.saturating_sub(reserve) && self.evict(None) {}
            let clip = Clip::load(&self.media, name)?;
            self.loads += 1;
            while self.bytes + clip.bytes > budget && self.evict(None) {}
            self.bytes += clip.bytes;
            self.clips.insert(name.into(), clip);
            self.order.push_back(name.into());
            true
        } else {
            let mut evicted = false;
            while self.bytes > budget && self.evict(Some(name)) {
                evicted = true;
            }
            self.order.retain(|n| n != name);
            self.order.push_back(name.into());
            evicted
        };
        if changed {
            release_unused_decode_memory();
        }
        self.published_bytes.store(self.bytes, Ordering::Relaxed);
        self.clips[name].frame(name, index, &mut self.decoder)
    }
}

pub(super) fn check_assets(media: &Path) -> Result<(), String> {
    let all = names(media)?;
    let budget = Arc::new(AtomicUsize::new(32 * 1048576));
    let published = Arc::new(AtomicUsize::new(0));
    let mut cache = ClipCache::new(media.to_path_buf(), budget, published)?;
    let mut canvas = Canvas::new(WINDOW_W, WINDOW_H);
    let start = Instant::now();
    let mut max_cached = 0;
    let mut max_frame = 0;
    let mut cold_ms = 0.0f64;
    let mut total = 0;
    for name in &all {
        total += media
            .join("anim")
            .join(format!("{name}.webp"))
            .metadata()
            .map_err(|e| e.to_string())?
            .len();
        for i in 0..COUNT as u32 {
            let t = Instant::now();
            let frame = cache
                .frame(name, i)
                .map_err(|e| format!("{name}, frame {i}: {e}"))?;
            if i == 0 {
                cold_ms = cold_ms.max(t.elapsed().as_secs_f64() * 1000.0);
            }
            max_frame = max_frame.max(frame.pixels.len());
            max_cached = max_cached.max(cache.bytes);
            frame.draw(i % 2 == 1, 0, true, &mut canvas);
        }
    }
    println!(
        "{}: {} original WebP clips, {} frames OK; {:.2} MiB stored; {:.2} MiB max cropped cache; {:.3} MiB max frame; {:.1} ms max cold load; {} source decodes; {:.2}s total",
        crate::character::selected(),
        all.len(),
        all.len() * COUNT,
        total as f64 / 1048576.0,
        max_cached as f64 / 1048576.0,
        max_frame as f64 / 1048576.0,
        cold_ms,
        cache.loads,
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_cleaned_assets_skip_repairs_and_legacy_raw_assets_keep_alpha_and_seam_fixes() {
        if crate::character::selected() != "codex-girl" {
            return;
        }
        let mut image = RgbaImage::new(20 * COUNT as u32, 20);
        let mut frames = Vec::new();
        for i in 0..COUNT as u32 {
            for y in 0..20 {
                for x in 0..20 {
                    let value = if i == 0 { 220 } else { 200 };
                    let alpha = if x == 0 {
                        44
                    } else if (6..9).contains(&x) && (6..9).contains(&y) {
                        93
                    } else {
                        255
                    };
                    image.put_pixel(i * 20 + x, y, image::Rgba([value, value, value, alpha]));
                }
            }
            frames.push(FrameMeta {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
                atlas_x: i * 20,
                atlas_y: 0,
            });
        }
        let encoded = webp::Encoder::from_rgba(image.as_raw(), image.width(), image.height())
            .encode_lossless();
        let decoded = webp::Decoder::new(&encoded).decode().unwrap();
        let mut native = image::ImageBuffer::from_raw(
            image.width(),
            image.height(),
            NativePixels::Native(decoded),
        )
        .unwrap();
        let meta = Metadata {
            atlas_width: image.width(),
            atlas_height: image.height(),
            frames,
        };
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ASSETS.json"), r#"{"status":"cleaned"}"#).unwrap();
        apply_source_repairs(dir.path(), &mut native, &meta).unwrap();
        assert_eq!(native.get_pixel(7, 7)[3], 93);
        assert_eq!(native.get_pixel(119 * 20 + 7, 7)[0], 200);
        std::fs::write(dir.path().join("ASSETS.json"), r#"{"status":"raw"}"#).unwrap();
        apply_source_repairs(dir.path(), &mut native, &meta).unwrap();
        assert_eq!(native.get_pixel(7, 7)[3], 255);
        assert_eq!(native.get_pixel(0, 7)[3], 44);
        assert_eq!(native.get_pixel(119 * 20 + 7, 7)[0], 220);
        let clip = Clip::from_anim(Anim {
            image: native,
            meta,
        })
        .unwrap();
        let mut decoder = zstd::bulk::Decompressor::new().unwrap();
        let last = clip.frame("sample", 119, &mut decoder).unwrap().pixels;
        for _ in 0..4 {
            assert_eq!(
                clip.frame("sample", 119, &mut decoder).unwrap().pixels,
                last
            );
        }
    }

    fn animation() -> Anim {
        let mut image = RgbaImage::new(4 * COUNT as u32, 3);
        let mut frames = Vec::new();
        for i in 0..COUNT as u32 {
            for y in 0..3 {
                for x in 0..4 {
                    let alpha = if (x, y) == (0, 0) {
                        0
                    } else if x == 1 {
                        93
                    } else {
                        255
                    };
                    image.put_pixel(i * 4 + x, y, image::Rgba([i as u8, 145, 222, alpha]));
                }
            }
            frames.push(FrameMeta {
                x: if i % 2 == 0 { 0 } else { FRAME_W - 4 },
                y: 20,
                width: 4,
                height: 3,
                atlas_x: i * 4,
                atlas_y: 0,
            });
        }
        Anim {
            image,
            meta: Metadata {
                atlas_width: 4 * COUNT as u32,
                atlas_height: 3,
                frames,
            },
        }
    }
    #[test]
    fn compressed_cache_does_not_retain_uncompressed_buffer_capacity() {
        let clip = Clip::from_anim(animation()).unwrap();
        let allocated: usize = clip.frames.iter().map(Vec::capacity).sum();
        let compressed: usize = clip.frames.iter().map(Vec::len).sum();
        assert_eq!(
            allocated, compressed,
            "cache must discard compression scratch capacity"
        );
        assert_eq!(
            clip.bytes, allocated,
            "cache budget must include allocated capacity"
        );
    }

    #[test]
    fn cropped_cache_preserves_visible_pixels_alpha_mirroring_and_premultiply() {
        let anim = animation();
        let mut expected = Canvas::new(WINDOW_W, WINDOW_H);
        let mut actual = Canvas::new(WINDOW_W, WINDOW_H);
        let reference = animation();
        let clip = Clip::from_anim(anim).unwrap();
        let mut decoder = zstd::bulk::Decompressor::new().unwrap();
        for i in 0..COUNT as u32 {
            for mirror in [false, true] {
                for premul in [false, true] {
                    reference.draw(i, mirror, 0, premul, &mut expected);
                    clip.frame("sample", i, &mut decoder).unwrap().draw(
                        mirror,
                        0,
                        premul,
                        &mut actual,
                    );
                    assert_eq!(
                        actual.buf, expected.buf,
                        "frame {i}, mirror {mirror}, premul {premul}"
                    );
                }
            }
        }
    }
    #[test]
    fn cache_hits_do_not_read_sources_or_process_again_and_eviction_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let budget = Arc::new(AtomicUsize::new(usize::MAX));
        let published = Arc::new(AtomicUsize::new(0));
        let mut cache =
            ClipCache::new(dir.path().to_path_buf(), budget.clone(), published.clone()).unwrap();
        for name in ["old", "active"] {
            let clip = Clip::from_anim(animation()).unwrap();
            cache.bytes += clip.bytes;
            cache.clips.insert(name.into(), clip);
            cache.order.push_back(name.into());
        }
        // No source files exist: any accidental reload/reprocessing will fail.
        let a = cache.frame("active", 20).unwrap().pixels;
        assert_eq!(cache.frame("active", 20).unwrap().pixels, a);
        assert_eq!(cache.loads, 0);
        budget.store(1, Ordering::Relaxed);
        assert!(cache.frame("active", 21).is_ok());
        assert_eq!(cache.clips.len(), 1);
        assert!(cache.clips.contains_key("active"));
        assert_eq!(published.load(Ordering::Relaxed), cache.bytes);
        assert!(cache.frame("active", COUNT as u32).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires repository DSH assets; run alone to measure process RSS"]
    fn real_clip_switches_keep_resident_memory_bounded() {
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dsh-girl/media");
        let mut sheet = Sheet::load(&media, 16).unwrap();
        fn rss() -> usize {
            std::fs::read_to_string("/proc/self/smaps_rollup")
                .unwrap()
                .lines()
                .find(|l| l.starts_with("Rss:"))
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse::<usize>()
                .unwrap()
                * 1024
        }
        let baseline = rss();
        let mut max_resident = baseline;
        let mut max_cached = 0;
        let all = names(&media).unwrap();
        assert!(all.len() >= 16, "need 16 distinct clips for cache churn");
        for name in all.iter().take(16) {
            let start = Instant::now();
            while !sheet.ready(name) {
                assert!(start.elapsed().as_secs() < 30, "clip load timed out");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            assert!(!sheet.failed.contains(name), "clip failed to load: {name}");
            max_resident = max_resident.max(rss());
            max_cached = max_cached.max(sheet.bytes());
        }
        let growth = max_resident.saturating_sub(baseline);
        println!(
            "16 cold clips: {:.2} MiB max RSS after load, {:.2} MiB cache, {:.2} MiB RSS growth",
            max_resident as f64 / 1048576.,
            max_cached as f64 / 1048576.,
            growth as f64 / 1048576.
        );
        // Allow decoder and allocator overhead above retained frame storage,
        // while catching growth toward 100 MiB after repeated cold loads.
        assert!(
            growth <= max_cached + 64 * 1048576,
            "resident growth exceeds cached data plus 64 MiB of decoder/allocator overhead"
        );
    }

    #[test]
    #[ignore = "requires repository original WebP assets"]
    fn real_raw_clips_match_existing_processed_renderer_and_never_write_sources() {
        let media = Path::new(env!("CARGO_MANIFEST_DIR")).join("media");
        let budget = Arc::new(AtomicUsize::new(32 * 1048576));
        let mut cache =
            ClipCache::new(media.clone(), budget, Arc::new(AtomicUsize::new(0))).unwrap();
        let mut expected = Canvas::new(WINDOW_W, WINDOW_H);
        let mut actual = Canvas::new(WINDOW_W, WINDOW_H);
        for pose in ["idle", "happy", "walk-left"] {
            let name = crate::character::pose_anim(pose).unwrap();
            let path = media.join("anim").join(format!("{name}.webp"));
            let before = std::fs::read(&path).unwrap();
            let reference = source_anim(&media, name).unwrap();
            for i in 0..COUNT as u32 {
                for mirror in [false, true] {
                    reference.draw(i, mirror, 0, true, &mut expected);
                    cache
                        .frame(name, i)
                        .unwrap()
                        .draw(mirror, 0, true, &mut actual);
                    assert_eq!(actual.buf, expected.buf, "{name} frame {i}");
                }
            }
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        assert_eq!(cache.loads, 3);
    }
}
