pub mod state;
pub mod proc;
pub mod battery;
pub mod mpris;
pub mod clipboard;
pub mod focus;

pub mod linux;

pub use linux::poll_system_state;
