// Spritesheet animation compositor. Every pose is one 8×15 WebP sheet of
// 640×360 frames at 12fps; nothing else is loaded and there is no fallback.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Native source frame, drawn 1:1 in physical pixels (no resampling).
const FRAME_W: u32 = 640;
const FRAME_H: u32 = 360;
/// 6px of transparent gutter around the frame; the right side is consumed by
/// ART_SHIFT when the art is pushed toward the edge.
pub const PAD: u32 = 6;
/// The source art has transparent padding on both sides. Shift it toward the
/// nearer screen edge so the visible pet sits close to that edge; main.rs
/// flips the sign between the right and left edges.
pub const ART_SHIFT: i32 = 148;
/// Buffer size in physical pixels; the host maps it to surface-local units.
pub const WINDOW_W: u32 = FRAME_W + 2 * PAD;
pub const WINDOW_H: u32 = FRAME_H + 2 * PAD;

/// Every action spritesheet is 8×15 frames at 12fps (source clips are 10s).
const COLS: u32 = 8;
const ROWS: u32 = 15;
const FPS: u64 = 12;
/// One decoded sheet is ~105MB; four in the LRU plus a pinned happy sheet.
const CACHE: usize = 4;
const IDLE_ANIM: &str = "待机";
const HAPPY_ANIM: &str = "开心";

#[derive(Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    rgba: Vec<u8>,
}

pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub buf: Vec<u8>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Canvas {
        Canvas {
            width,
            height,
            buf: vec![0; (width * height * 4) as usize],
        }
    }
}

/// One action animation: a scaled 8×15 spritesheet plus its frame geometry.
struct Anim {
    sheet: Frame,
    frame_w: u32,
    frame_h: u32,
}

impl Anim {
    fn new(sheet: Frame) -> Anim {
        Anim {
            frame_w: sheet.width / COLS,
            frame_h: sheet.height / ROWS,
            sheet,
        }
    }

    fn frame(&self, now_ms: u64, mirror: bool) -> Frame {
        let count = (COLS * ROWS) as u64;
        let idx = (now_ms * FPS / 1000) % count;
        let col = (idx % COLS as u64) as u32;
        let row = (idx / COLS as u64) as u32;
        let (fw, fh) = (self.frame_w, self.frame_h);
        let row_bytes = (fw * 4) as usize;
        let mut rgba = vec![0u8; row_bytes * fh as usize];
        for y in 0..fh {
            let src = (((row * fh + y) * self.sheet.width + col * fw) * 4) as usize;
            let dst = y as usize * row_bytes;
            let line = &self.sheet.rgba[src..src + row_bytes];
            if mirror {
                for (x, px) in line.chunks_exact(4).enumerate() {
                    let d = dst + (fw as usize - 1 - x) * 4;
                    rgba[d..d + 4].copy_from_slice(px);
                }
            } else {
                rgba[dst..dst + row_bytes].copy_from_slice(line);
            }
        }
        Frame {
            width: fw,
            height: fh,
            rgba,
        }
    }
}

/// Pet pose → action file stem (every state the pose engine can reach).
fn pose_anim(pose: &str) -> Option<&'static str> {
    Some(match pose {
        "idle" => IDLE_ANIM,
        "happy" => HAPPY_ANIM,
        "dance" => "跳舞",
        "typing" => "写代码",
        "music" => "听音乐",
        "walk-left" | "walk-right" => "奔跑",
        "copy" => "记录",
        "charging" => "庆祝",
        "low-battery" => "没电",
        "late-night" | "drowsy" => "打哈欠",
        "asleep" => "睡觉",
        _ => return None,
    })
}

/// Lazily loaded animations from `<media>/anim/<action>.webp`.
pub struct Sheet {
    dir: PathBuf,
    names: Vec<String>,
    anims: HashMap<String, Anim>,
    order: VecDeque<String>,
    /// click reaction sheet; pinned so a click never waits on the LRU
    happy: Option<Anim>,
}

impl Sheet {
    pub fn load(media_dir: &Path) -> Sheet {
        let dir = media_dir.join("anim");
        let mut names = Vec::new();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "webp") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        names.push(stem.to_owned());
                    }
                }
            }
        }
        names.sort();
        Sheet {
            dir,
            names,
            anims: HashMap::new(),
            order: VecDeque::new(),
            happy: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Current frame for a pose. Idle easter eggs return raw action names,
    /// which pass through unchanged. Unknown names draw nothing.
    pub fn frame_at(&mut self, pose: &str, now_ms: u64, mirror: bool) -> Option<Frame> {
        let name = pose_anim(pose).unwrap_or(pose);
        if !self.names.iter().any(|n| n.as_str() == name) {
            return None;
        }
        Some(self.anim_mut(name)?.frame(now_ms, mirror))
    }

    /// Load an animation on first use; evict the least recent when full.
    fn anim_mut(&mut self, name: &str) -> Option<&mut Anim> {
        if name == HAPPY_ANIM {
            if self.happy.is_none() {
                let anim = self.decode(name)?;
                self.happy = Some(anim);
            }
            return self.happy.as_mut();
        }
        if !self.anims.contains_key(name) {
            let anim = self.decode(name)?;
            while self.anims.len() >= CACHE {
                let Some(old) = self.order.pop_front() else {
                    break;
                };
                self.anims.remove(&old);
            }
            self.anims.insert(name.to_owned(), anim);
        }
        self.order.retain(|n| n.as_str() != name);
        self.order.push_back(name.to_owned());
        self.anims.get_mut(name)
    }

    /// Decode `<media>/anim/<name>.webp` into memory.
    fn decode(&self, name: &str) -> Option<Anim> {
        let img = image::ImageReader::open(self.dir.join(format!("{name}.webp")))
            .ok()?
            .decode()
            .ok()?
            .to_rgba8();
        Some(Anim::new(Frame {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        }))
    }
}

/// Draw `src` onto `dst` at integer `scale`, offset (dx, dy).
/// Nearest neighbour, straight alpha over.
pub fn blit_scale(dst: &mut Canvas, src: &Frame, dx: i32, dy: i32, scale: u32) {
    for sy in 0..src.height {
        for sx in 0..src.width {
            let i = ((sy * src.width + sx) * 4) as usize;
            let (px, py, pb, a) = (
                src.rgba[i],
                src.rgba[i + 1],
                src.rgba[i + 2],
                src.rgba[i + 3],
            );
            if a == 0 && scale > 1 {
                continue;
            }
            for oy in 0..scale {
                let dy2 = dy + (sy * scale + oy) as i32;
                if dy2 < 0 || dy2 >= dst.height as i32 {
                    continue;
                }
                for ox in 0..scale {
                    let dx2 = dx + (sx * scale + ox) as i32;
                    if dx2 < 0 || dx2 >= dst.width as i32 {
                        continue;
                    }
                    let di = ((dy2 as u32 * dst.width + dx2 as u32) * 4) as usize;
                    if a == 255 {
                        dst.buf[di] = px;
                        dst.buf[di + 1] = py;
                        dst.buf[di + 2] = pb;
                        dst.buf[di + 3] = 255;
                    } else {
                        let da = dst.buf[di + 3] as u32;
                        let oa = a as u32;
                        let out = 255 - (255 - oa) * (255 - da) / 255;
                        if out > 0 {
                            for c in 0..3 {
                                let sc = [px, py, pb][c] as u32;
                                let dc = dst.buf[di + c] as u32;
                                let v = (sc * oa + dc * da * (255 - oa) / 255) / out.max(1);
                                dst.buf[di + c] = v.min(255) as u8;
                            }
                            dst.buf[di + 3] = out as u8;
                        }
                    }
                }
            }
        }
    }
}
