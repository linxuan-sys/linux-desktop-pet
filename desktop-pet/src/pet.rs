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
