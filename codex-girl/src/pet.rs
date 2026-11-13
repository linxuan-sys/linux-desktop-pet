use crate::sensors::state::DerivedState;
use std::sync::OnceLock;

// One clip is 120 frames at 12fps = 10s. Nothing switches before its clip
// ends; the only exception is a click, which must interrupt instantly.
const CLIP_MS: u64 = 10_000;
/// A click plays the happy hop from its 3s mark to the end, so the long
/// wind-up never shows. The renderer skips the first HAPPY_SKIP_MS.
pub const HAPPY_MS: u64 = 7_000;
pub const HAPPY_SKIP_MS: u64 = 3_000;
// Three 10s work clips, then one 10s music clip. 28s puts the first music
// on the boundary after three clips; 38s = 10s music + three more clips.
const FIRST_MUSIC_AT: u64 = 28_000;
const MUSIC_INTERVAL: u64 = 38_000;
const LOW_BATTERY_PCT: u32 = 20;

/// The run window inside the 10s clip: the window moves from 3s to 5.5s.
const WALK_START_MS: u64 = 3_000;
const WALK_MOVE_MS: u64 = 2_500;
/// Window travel speed while the run is on.
const WALK_PX_PER_SEC: i32 = 60;
/// Cheap cosmetic randomness (no rand crate).
fn nanos() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq)]
enum Step {
    Idle,
    Random,
    Walk,
}

/// Two idle clips, one random clip, twice, then a walk.
const STEPS: &[Step] = &[
    Step::Idle,
    Step::Idle,
    Step::Random,
    Step::Idle,
    Step::Idle,
    Step::Random,
    Step::Walk,
];

const EASTER_ACTIONS: &[&str] = &[
    "三球抛接", "下五子棋", "东张西望", "优雅提裙旋舞", "偷吃零食被抓住",
    "傲娇生气", "写代码", "写吉祥纹", "凭空生花", "分文不剩", "动物环绕",
    "原地专心玩魔方", "原地敲击桌面互动", "原地漂浮踏步", "原地跳跃抓碎头顶物品", "原地踱步张望", "原地蹲下玩玩具汽车",
    "原地重力下蹲压缩", "发呆碎碎念", "变鸽子", "吃Token", "吃冰淇淋融化", "吃午餐",
    "吃大闸蟹", "吃年糕", "吃早餐", "吃晚餐", "吃汤圆", "吃白饭",
    "吃粽子", "吃腊八粥", "吃西瓜", "吃重阳糕", "吃长寿面", "吃青团",
    "吃饺子", "听音乐", "吹气球", "吹笛子", "大口吃零食", "奔跑",
    "害羞惊讶", "对屏碎碎念", "小提琴演奏", "庆祝", "开心", "待机",
    "忙碌点按", "思考冒泡", "打哈欠", "打瞌睡被惊醒", "抽陀螺", "拆礼物",
    "挠痒咯咯笑", "挥手", "提裙屈膝礼仪", "插茱萸赏菊", "摇扇纳凉", "撸猫",
    "收红包", "放河灯", "放烟花", "数钱皱眉", "整体换装试色", "是啊吃什么",
    "晨间刷牙", "没电", "涮火锅", "深度思考碎碎念", "玩游戏气急败坏", "用龙尾拍打地面",
    "闪亮登场", "睡觉", "穿针乞巧", "舞狮头", "蝴蝶蜜蜂环绕头顶开花", "袋空如洗",
    "被吓一跳", "被落叶淹没", "被鼠标拖拽悬空反馈", "装点圣诞树", "记录", "超大伸懒腰",
    "跳舞", "踢毽子", "钱袋如常", "钱袋满溢", "银袋叮当", "骑木马",
    "龙息吐泡泡特效",
];

/// Clips that already mean a dedicated state/pose; the random pool must not
/// steal them, or a random pick would collide with a real state.
const SPECIAL: &[&str] = &[
    "写代码", "听音乐", "奔跑", "庆祝", "开心", "待机",
    "打哈欠", "没电", "睡觉", "记录", "跳舞",
];

/// Random library clip, excluding everything with a dedicated meaning.
fn roll_random() -> &'static str {
    static POOL: OnceLock<Vec<&'static str>> = OnceLock::new();
    let pool = POOL.get_or_init(|| {
        EASTER_ACTIONS
            .iter()
            .copied()
            .filter(|a| !SPECIAL.contains(a))
            .collect()
    });
    pool[nanos() as usize % pool.len()]
}

const LAZY_START: u64 = u64::MAX;

enum Reaction {
    Happy { start: u64 },
    Charging { start: u64 },
    Copy { start: u64 },
}

#[derive(Clone, Copy)]
enum Mode {
    Awake,
    Action,
    Drowsy,
    Asleep,
}

pub struct Pet {
    mode: Mode,
    last_state_mode: String,
    reaction: Option<Reaction>,
    was_charging: bool,
    spotify: bool,
    hour: u32,
    battery_pct: u32,
    charging: bool,
    next_music: u64,
    music_until: Option<u64>,
    // position + walk state (screen coords; the picture window is bottom-anchored)
    pos_x: i32,
    pos_y: i32,
    /// rightmost legal window x (logical); set from the host each frame
    max_x: i32,
    walk: Option<Walk>,
    // the clip currently on screen, and when it started (frame 0)
    current: Option<&'static str>,
    current_start: u64,
    // awake schedule: one full 10s clip per step
    step_index: usize,
    at_step_start: bool,
    random_anim: &'static str,
    /// walk direction cycle: two left runs, then two right runs, repeat
    walk_phase: usize,
}

/// One commanded walk: slides from x0 to target over exactly one 10s
/// clip, so the feet and the window move on the same clock.
#[derive(Clone, Copy)]
struct Walk {
    start: u64,
    x0: i32,
    target: i32,
    right: bool,
}

impl Reaction {
    fn frame(&self, now: u64) -> Option<&'static str> {
        // saturating: a queued reaction still carries LAZY_START (u64::MAX)
        match *self {
            Reaction::Happy { start } if now < start.saturating_add(HAPPY_MS) => Some("happy"),
            Reaction::Charging { start } if now < start.saturating_add(CLIP_MS) => {
                Some("charging")
            }
            Reaction::Copy { start } if now < start.saturating_add(CLIP_MS) => Some("copy"),
            _ => None,
        }
    }
}

impl Pet {
    pub fn new() -> Pet {
        Pet {
            mode: Mode::Awake,
            last_state_mode: "awake".into(),
            reaction: None,
            was_charging: true, // start like the renderer inits (charging), so only a real edge fires
            spotify: false,
            hour: 12,
            battery_pct: 100,
            charging: true,
            next_music: LAZY_START,
            music_until: None,
            pos_x: 0,
            pos_y: 0,
            max_x: i32::MAX,
            walk: None,
            current: None,
            current_start: 0,
            step_index: 0,
            at_step_start: true,
            random_anim: EASTER_ACTIONS[0],
            walk_phase: 0,
        }
    }

    pub fn apply(&mut self, st: &DerivedState) {
        self.spotify = st.spotify_playing;
        self.hour = st.hour;
        self.battery_pct = st.battery_pct;
        self.charging = st.is_charging;

        // leaving awake aborts the walk; the schedule restarts at step 0
        if st.mode != "awake" {
            self.walk = None;
        }

        // charging celebration is an edge (upstream wasCharging)
        if st.is_charging && !self.was_charging && self.reaction.is_none() {
            self.reaction = Some(Reaction::Charging { start: LAZY_START });
        }
        self.was_charging = st.is_charging;

        let new_mode = st.mode.as_str();
        if new_mode != self.last_state_mode {
            self.step_index = 0;
            self.at_step_start = true;
            self.last_state_mode = new_mode.to_string();
            if new_mode == "action" {
                // entering action re-arms the music relay
                self.next_music = LAZY_START;
                self.music_until = None;
            }
        }

        self.mode = match new_mode {
            "action" => Mode::Action,
            // keep the drowsy -> asleep progression; every 2.5s state poll
            // must not push a sleeping pet back to drowsy
            "asleep" => match self.mode {
                Mode::Drowsy | Mode::Asleep => self.mode,
                _ => Mode::Drowsy,
            },
            _ => Mode::Awake,
        };
    }

    pub fn on_copy(&mut self, _now: u64) {
        // upstream gate (canClipboardReact): awake only
        if self.reaction.is_none() && matches!(self.mode, Mode::Awake) {
            self.reaction = Some(Reaction::Copy { start: LAZY_START });
        }
    }

    /// A click interrupts whatever is on screen, including a walk or typing,
    /// and plays one full happy hop. Clicks during the hop are ignored.
    pub fn on_click(&mut self, now: u64) {
        if matches!(self.reaction, Some(Reaction::Happy { .. })) {
            return;
        }
        self.walk = None;
        self.reaction = Some(Reaction::Happy { start: now });
    }

    // ---- walk plumbing (screen coords; the host carves these from margins) ----

    /// Read back the window position; ignored mid-walk so the walk clock wins.
    pub fn sync_pos(&mut self, x: i32, y: i32) {
        if self.walk.is_none() {
            self.pos_x = x;
            self.pos_y = y;
        }
    }

    /// Rightmost legal window x from the host; 0 means unknown.
    pub fn set_max_x(&mut self, x: i32) {
        if x > 0 {
            self.max_x = x;
        }
    }

    pub fn pos(&self) -> (i32, i32) {
        (self.pos_x, self.pos_y)
    }

    /// Start time (ms) of the clip on screen; the renderer indexes the video
    /// by this, so a repeated pose still restarts at frame 0.
    pub fn clip_start(&self) -> u64 {
        self.current_start
    }

    pub fn walking(&self) -> bool {
        self.walk.is_some()
    }

    /// A drag physically moved the window: drop any walk in progress.
    pub fn on_drag(&mut self) {
        self.walk = None;
    }

    fn set_pose(&mut self, pose: &'static str, now: u64) {
        if self.current != Some(pose) {
            self.current = Some(pose);
            self.current_start = now;
        }
    }

    fn walk_frame(&mut self, now: u64) -> Option<&'static str> {
        let Walk {
            start,
            x0,
            target,
            right,
        } = match self.walk {
            None => return None,
            Some(w) => w,
        };
        let pose = if right { "walk-right" } else { "walk-left" };
        let elapsed = now.saturating_sub(start);
        if elapsed >= CLIP_MS {
            self.pos_x = target;
            self.walk = None;
            return Some(pose);
        }
        // before the run the window stays put; after it, the run's last
        // position holds until the clip ends
        if elapsed < WALK_START_MS {
            self.pos_x = x0;
            return Some(pose);
        }
        let moving = (elapsed - WALK_START_MS).min(WALK_MOVE_MS);
        let dx = (target - x0) as i64;
        self.pos_x = x0 + ((dx * moving as i64) / WALK_MOVE_MS as i64) as i32;
        Some(pose)
    }

    /// One 10s clip walking left or right: the window moves only during the
    /// run window. The target is clamped to the screen, so at a wall the
    /// window stays put and the clip runs in place.
    fn begin_walk(&mut self, now: u64, right: bool) -> &'static str {
        let dist = WALK_PX_PER_SEC * WALK_MOVE_MS as i32 / 1000;
        let target = if right {
            (self.pos_x + dist).min(self.max_x.max(self.pos_x))
        } else {
            (self.pos_x - dist).max(0)
        };
        self.walk = Some(Walk {
            start: now,
            x0: self.pos_x,
            target,
            right,
        });
        if right { "walk-right" } else { "walk-left" }
    }

    pub fn tick(&mut self, now: u64) -> Option<&'static str> {
        // finished reactions drop out
        if matches!(&self.reaction, Some(r) if r.frame(now).is_none()) {
            self.reaction = None;
        }

        let elapsed = now.saturating_sub(self.current_start);
        let clip_ms = if self.current == Some("happy") {
            HAPPY_MS
        } else {
            CLIP_MS
        };
        let clip_done = self.current.is_none() || elapsed >= clip_ms;

        // Reactions: a click interrupts instantly; other events wait until
        // the running clip finishes so no animation is cut in half.
        let mut reaction_pose = None;
        if let Some(r) = &mut self.reaction {
            let urgent = matches!(r, Reaction::Happy { .. });
            if urgent || clip_done {
                match r {
                    Reaction::Happy { .. } => {}
                    Reaction::Copy { start } | Reaction::Charging { start } => {
                        if *start == LAZY_START {
                            *start = now;
                        }
                    }
                }
                reaction_pose = r.frame(now);
            }
        }
        if let Some(pose) = reaction_pose {
            self.set_pose(pose, now);
            return Some(pose);
        }

        // The walk drives the window and is itself one 10s clip.
        if let Some(pose) = self.walk_frame(now) {
            self.set_pose(pose, now);
            return Some(pose);
        }

        // Hold the current clip until its full loop ends.
        if !clip_done {
            return self.current;
        }

        let next = self.next_pose(now);
        // A new step starts a new clip even when the pose name repeats
        // (consecutive Idle steps, or the same random clip twice).
        self.current = Some(next);
        self.current_start = now;
        Some(next)
    }

    /// The pose to start once the running clip has finished.
    fn next_pose(&mut self, now: u64) -> &'static str {
        match self.mode {
            Mode::Asleep => return "asleep",
            Mode::Drowsy => {
                if self.current == Some("drowsy") {
                    self.mode = Mode::Asleep;
                    return "asleep";
                }
                return "drowsy";
            }
            Mode::Action => {
                if self.next_music == LAZY_START {
                    self.next_music = now + FIRST_MUSIC_AT;
                }
                if self.music_until.is_none() && now >= self.next_music {
                    self.music_until = Some(now + CLIP_MS);
                    self.next_music = now + MUSIC_INTERVAL;
                }
                if let Some(until) = self.music_until {
                    if now < until {
                        return "music";
                    }
                    self.music_until = None;
                }
                if self.spotify {
                    return "dance";
                }
                return "typing";
            }
            Mode::Awake => {}
        }

        if self.spotify {
            return "dance";
        }
        if self.battery_pct <= LOW_BATTERY_PCT && !self.charging {
            return "low-battery";
        }
        if self.hour < 6 {
            return "late-night";
        }

        // Advance the schedule by exactly one step per finished clip.
        if self.at_step_start {
            self.at_step_start = false;
        } else if self.current.is_some() {
            self.step_index = (self.step_index + 1) % STEPS.len();
        }
        match STEPS[self.step_index] {
            Step::Walk => {
                // two left runs, then two right runs; at a wall the window
                // stays put and the clip runs in place
                let right = self.walk_phase >= 2;
                self.walk_phase = (self.walk_phase + 1) % 4;
                self.begin_walk(now, right)
            }
            Step::Random => {
                self.random_anim = roll_random();
                self.random_anim
            }
            Step::Idle => "idle",
        }
    }
}
