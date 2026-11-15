pub mod battery;
pub mod clipboard;
pub mod focus;
pub mod mpris;
pub mod proc;
pub mod state;

pub mod linux;

pub use linux::poll_system_state;
