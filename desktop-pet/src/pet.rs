#[path = "../../claude-girl/src/pet.rs"]
mod claude;
#[path = "../../codex-girl/src/pet.rs"]
mod codex;
#[path = "../../dsh-girl/src/pet.rs"]
mod dsh;
#[path = "../../gemini-girl/src/pet.rs"]
mod gemini;
#[path = "../../glm-girl/src/pet.rs"]
mod glm;
#[path = "../../grok-girl/src/pet.rs"]
mod grok;
#[path = "../../kimi-girl/src/pet.rs"]
mod kimi;
#[path = "../../qwen-girl/src/pet.rs"]
mod qwen;
use crate::sensors::state::DerivedState;

pub fn happy_skip_ms() -> u64 {
    match crate::character::selected() {
        "claude-girl" => claude::HAPPY_SKIP_MS,
        "dsh-girl" => dsh::HAPPY_SKIP_MS,
        "codex-girl" => codex::HAPPY_SKIP_MS,
        "grok-girl" => grok::HAPPY_SKIP_MS,
        "qwen-girl" => qwen::HAPPY_SKIP_MS,
        "gemini-girl" => gemini::HAPPY_SKIP_MS,
        "glm-girl" => glm::HAPPY_SKIP_MS,
        "kimi-girl" => kimi::HAPPY_SKIP_MS,
        _ => 0,
    }
}

pub struct Pet(Box<dyn Behavior>);
trait Behavior {
    fn apply(&mut self, state: &DerivedState);
    fn on_click(&mut self, now: u64);
    fn on_copy(&mut self, now: u64);
    fn on_drag(&mut self);
    fn on_drag_end(&mut self);
    fn sync_pos(&mut self, x: i32, y: i32);
    fn set_max_x(&mut self, max_x: i32);
    fn tick(&mut self, now: u64) -> Option<&'static str>;
    fn walking(&self) -> bool;
    fn pos(&self) -> (i32, i32);
    fn clip_start(&self) -> u64;
}

macro_rules! dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        $self.0.$method($($arg),*)
    };
}

impl Pet {
    pub fn new() -> Self {
        let pet: Box<dyn Behavior> = match crate::character::selected() {
            "claude-girl" => Box::new(claude::Pet::new()),
            "dsh-girl" => Box::new(dsh::Pet::new()),
            "codex-girl" => Box::new(codex::Pet::new()),
            "grok-girl" => Box::new(grok::Pet::new()),
            "qwen-girl" => Box::new(qwen::Pet::new()),
            "gemini-girl" => Box::new(gemini::Pet::new()),
            "glm-girl" => Box::new(glm::Pet::new()),
            "kimi-girl" => Box::new(kimi::Pet::new()),
            _ => unreachable!(),
        };
        Self(pet)
    }
    pub fn apply(&mut self, state: &DerivedState) {
        dispatch!(self, apply, state);
    }
    pub fn on_click(&mut self, now: u64) {
        dispatch!(self, on_click, now);
    }
    pub fn on_copy(&mut self, now: u64) {
        dispatch!(self, on_copy, now);
    }
    pub fn on_drag(&mut self) {
        dispatch!(self, on_drag);
    }
    pub fn on_drag_end(&mut self) {
        dispatch!(self, on_drag_end);
    }
    pub fn sync_pos(&mut self, x: i32, y: i32) {
        dispatch!(self, sync_pos, x, y);
    }
    pub fn set_max_x(&mut self, max_x: i32) {
        dispatch!(self, set_max_x, max_x);
    }
    pub fn tick(&mut self, now: u64) -> Option<&'static str> {
        dispatch!(self, tick, now)
    }
    pub fn walking(&self) -> bool {
        dispatch!(self, walking)
    }
    pub fn pos(&self) -> (i32, i32) {
        dispatch!(self, pos)
    }
    pub fn clip_start(&self) -> u64 {
        dispatch!(self, clip_start)
    }
}

impl Default for Pet {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! behavior {
    ($module:ident) => {
        impl Behavior for $module::Pet {
            fn apply(&mut self, state: &DerivedState) {
                self.apply(state);
            }
            fn on_click(&mut self, now: u64) {
                self.on_click(now);
            }
            fn on_copy(&mut self, now: u64) {
                self.on_copy(now);
            }
            fn on_drag(&mut self) {
                self.on_drag();
            }
            fn on_drag_end(&mut self) {
                self.on_drag_end();
            }
            fn sync_pos(&mut self, x: i32, y: i32) {
                $module::Pet::sync_pos(self, x, y);
            }
            fn set_max_x(&mut self, max_x: i32) {
                $module::Pet::set_max_x(self, max_x);
            }
            fn tick(&mut self, now: u64) -> Option<&'static str> {
                $module::Pet::tick(self, now)
            }
            fn walking(&self) -> bool {
                $module::Pet::walking(self)
            }
            fn pos(&self) -> (i32, i32) {
                $module::Pet::pos(self)
            }
            fn clip_start(&self) -> u64 {
                $module::Pet::clip_start(self)
            }
        }
    };
}
behavior!(claude);
behavior!(dsh);
behavior!(codex);
behavior!(grok);
behavior!(qwen);
behavior!(gemini);
behavior!(glm);
behavior!(kimi);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensor_updates_keep_movement_with_the_running_clip() {
        fn check(mut pet: impl Behavior, name: &str, distance: i32, mode_during_walk: &str) {
            let mut state = DerivedState {
                mode: "asleep".into(),
                spotify_playing: false,
                hour: 12,
                battery_pct: 100,
                is_charging: true,
            };
            let mut x = 500;
            for now in (0..=66_000).step_by(100) {
                if now == 62_500 {
                    state.mode = mode_during_walk.into();
                }
                if now % 2500 == 0 {
                    pet.apply(&state);
                }
                pet.sync_pos(x, 100);
                pet.set_max_x(1000);
                let pose = pet.tick(now);
                if now >= 60_000 {
                    assert_eq!(pose, Some("walk-left"), "{name} at {now}");
                }
                if pet.walking() {
                    x = pet.pos().0;
                }
            }
            assert_eq!(x, 500 - distance, "{name} in {mode_during_walk}");
        }

        for mode in ["asleep", "action"] {
            check(claude::Pet::new(), "claude", 120, mode);
            check(dsh::Pet::new(), "dsh", 180, mode);
            check(codex::Pet::new(), "codex", 150, mode);
            check(grok::Pet::new(), "grok", 120, mode);
            check(qwen::Pet::new(), "qwen", 120, mode);
            check(gemini::Pet::new(), "gemini", 120, mode);
            check(glm::Pet::new(), "glm", 150, mode);
            check(kimi::Pet::new(), "kimi", 150, mode);
        }
    }
}
