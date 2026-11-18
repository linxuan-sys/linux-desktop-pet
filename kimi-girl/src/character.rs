/// Whether the renderer premultiplies alpha before handing pixels to the
/// compositor. Wayland's Argb8888 is a premultiplied format, so `true` is
/// correct; `false` keeps the authored colours exactly and suits artwork whose
/// antialiased edges carry green-screen residue (that shows up as dark speckle
/// once premultiplied). Override with `premultiply` in the config file.
pub const DEFAULT_PREMULTIPLY: bool = true;
pub fn selected() -> &'static str {
    env!("CARGO_PKG_NAME")
}
pub fn select(_: &str) -> Result<&'static str, String> {
    Ok(selected())
}

#[path = "../../engine/poses.rs"]
mod poses;
pub use poses::animation_for;
pub fn pose_anim(pose: &str) -> Option<&'static str> {
    animation_for(selected(), pose)
}
