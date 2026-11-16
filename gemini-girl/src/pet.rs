use crate::sensors::state::DerivedState;
use std::sync::OnceLock;

// One clip is 120 frames at 12fps = 10s. Nothing switches before its clip
// ends; the only exception is a click, which must interrupt instantly.
const CLIP_MS: u64 = 10_000;
/// Gemini Girl's click-response clip: skip to 2.5s and play to the clip's end,
/// so the hop lasts 7.5s and never wraps past the 10s mark.
pub const HAPPY_MS: u64 = 7_500;
pub const HAPPY_SKIP_MS: u64 = 2_500;
// Three 10s work clips, then one 10s music clip. 28s puts the first music
// on the boundary after three clips; 38s = 10s music + three more clips.
/// Terminal-focus cycle: two idle clips, typing, idle, music (50s total).
const ACTION_STEPS: &[&str] = &["idle", "idle", "typing", "idle", "music"];
const LOW_BATTERY_PCT: u32 = 20;

/// The run window inside the 10s clip: the window moves from 3.5s to 5.5s.
const WALK_START_MS: u64 = 3_500;
const WALK_MOVE_MS: u64 = 2_000;
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
    "下五子棋", "余额-分文不剩", "余额-数金皱眉", "余额-袋空如洗", "余额-金袋叮当", "余额-钱袋如常",
    "余额-钱袋满溢", "偷吃零食被抓住", "写福字", "凭空生花", "动物环绕", "原地专心玩魔方",
    "原地敲击桌面互动", "原地漂浮踏步", "原地蹲下玩玩具汽车", "原地重力下蹲压缩", "变鸽子", "吃Token",
    "吃冰淇淋融化", "吃午餐", "吃大闸蟹", "吃年糕", "吃早餐", "吃晚餐",
    "吃汤圆", "吃白饭", "吃粽子", "吃糖葫芦", "吃腊八粥", "吃西瓜",
    "吃重阳糕", "吃长寿面", "吃青团", "吃饺子", "吹气球", "堆雪人",
    "大口吃零食", "小幅度原地360度旋转展示", "小提琴演奏", "工作状态-忙碌点按", "工作状态-思考冒泡", "工作状态-清点归档",
    "扑克魔术", "抽陀螺", "拆礼物", "插茱萸赏菊", "摇扇纳凉", "收红包",
    "放孔明灯", "放烟花", "放风筝", "整体换装试色", "是啊，吃什么", "晨间刷牙",
    "涮火锅", "深度思考碎碎念", "点击回应-傲娇生气", "点击回应-元气挥手", "点击回应-害羞惊讶", "照镜子",
    "玩水枪", "玩游戏气急败坏", "碎碎念-对屏碎碎念", "碎碎念-擦桌碎碎念", "穿针乞巧", "舞狮头",
    "荡秋千", "萌化小幽灵", "蝴蝶蜜蜂环绕头顶开花", "螃蟹走路", "被吓一跳", "被落叶淹没",
    "装点圣诞树", "讨糖南瓜灯", "超大伸懒腰", "轻快摇摆舞", "骑木马", "优雅猫耳娘舞",
    "原地跳跃抓碎头顶物品", "嘟嘴吹泡泡特效", "工作状态-原地踱步张望", "打瞌睡被惊醒", "放孔明灯v2", "放河灯v2",
    "放河灯v3", "星星精灵现世", "猫耳娘屈膝礼仪", "用蓬松紫色猫尾拍打地面", "碎碎念-发呆碎碎念",
];


/// Clips that already mean a dedicated state/pose; the random pool must not
/// steal them, or a random pick would collide with a real state.
const SPECIAL: &[&str] = &[
    "被鼠标拖拽悬空反馈",
    "写代码",
    "悠闲哼歌",
    "原地左转奔跑",
    "工作状态-雀跃庆祝",
    "点击回应-开心跃动",
    "待机呼吸休闲",
    "哈欠连天",
    "原地小憩沉眠",
    "轻快记录",
    "可爱宅舞",
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
    if pool.is_empty() {
        return "idle";
    }
    pool[nanos() as usize % pool.len()]
}

const LAZY_START: u64 = u64::MAX;

enum Reaction {
    Happy { start: u64 },
    Charging { start: u64 },
    Copy { start: u64 },
    /// Held while the pointer drags the window; cleared on release.
    Dragged,
}

#[derive(Clone, Copy)]
enum Mode {
    Awake,
    Action,
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
    action_step: usize,
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
            Reaction::Charging { start } if now < start.saturating_add(CLIP_MS) => Some("charging"),
            Reaction::Copy { start } if now < start.saturating_add(CLIP_MS) => Some("copy"),
            Reaction::Dragged => Some("dragged"),
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
            action_step: 0,
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
                self.action_step = 0;
            }
        }

        self.mode = match new_mode {
            "action" => Mode::Action,
            _ => Mode::Awake,
        };
    }

    /// A clipboard copy beats everything: dance, night sleep, work loop,
    /// and any reaction already on screen.
    pub fn on_copy(&mut self, _now: u64) {
        self.walk = None;
        self.reaction = Some(Reaction::Copy { start: LAZY_START });
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
    /// Pointer grabbed the window: highest priority, and the clip keeps
    /// running instead of freezing while the window moves.
    pub fn on_drag(&mut self) {
        self.walk = None;
        self.reaction = Some(Reaction::Dragged);
    }

    /// Pointer released.
    pub fn on_drag_end(&mut self) {
        if matches!(self.reaction, Some(Reaction::Dragged)) {
            self.reaction = None;
        }
        // A drag (or a reopen/scale change) invalidates any walk in flight.
        self.walk = None;
        // Release ends the drag clip at once instead of playing it out.
        self.current = None;
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
        } = self.walk?;
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
            // copy and the click hop cut in at once; the rest wait for
            // the running clip to finish
            let urgent = matches!(
                r,
                Reaction::Happy { .. } | Reaction::Copy { .. } | Reaction::Dragged
            );
            if urgent || clip_done {
                match r {
                    Reaction::Happy { .. } => {}
                    Reaction::Copy { start } | Reaction::Charging { start } => {
                        if *start == LAZY_START {
                            *start = now;
                        }
                    }
                    Reaction::Dragged => {}
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
        if self.spotify {
            return "dance";
        }
        // after midnight she alternates yawning and sleeping, even while a
        // terminal is focused
        if self.hour < 6 {
            return if self.current == Some("drowsy") { "asleep" } else { "drowsy" };
        }
        match self.mode {
            Mode::Action => {
                let pose = ACTION_STEPS[self.action_step];
                self.action_step = (self.action_step + 1) % ACTION_STEPS.len();
                return pose;
            }
            Mode::Awake => {}
        }

        if self.battery_pct <= LOW_BATTERY_PCT && !self.charging {
            return "low-battery";
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

impl Default for Pet {
    fn default() -> Self {
        Self::new()
    }
}
