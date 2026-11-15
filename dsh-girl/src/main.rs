// Shared engine, independent character state machine and animation vocabulary.
mod character;
#[path = "../../engine/gfx.rs"]
mod gfx;
#[path = "../../engine/host/mod.rs"]
pub mod host;
pub mod pet;
#[path = "../../engine/runtime.rs"]
mod runtime;
#[path = "../../engine/sensors/mod.rs"]
mod sensors;
#[path = "../../engine/settings.rs"]
mod settings;

fn main() {
    runtime::main();
}
