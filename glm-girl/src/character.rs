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

const IDLE_ANIM: &str = "待机呼吸休闲";
const HAPPY_ANIM: &str = "点击回应-开心跃动";

pub fn pose_anim(pose: &str) -> Option<&'static str> {
    Some(match pose {
        "idle" => IDLE_ANIM,
        "happy" => HAPPY_ANIM,
        "dance" => "可爱宅舞",
        "typing" => "写代码",
        "music" => "悠闲哼歌",
        "walk-left" | "walk-right" => "原地左转奔跑",
        "copy" => "轻快记录",
        // GLM's sheet set has no 工作状态-* clips; stand-ins below.
        "charging" => "小蓝狐现世",
        "low-battery" => "原地重力下蹲压缩",
        "late-night" | "drowsy" => "哈欠连天",
        "asleep" => "原地小憩沉眠",
        "dragged" => "被鼠标拖拽悬空反馈",
        _ => return None,
    })
}
