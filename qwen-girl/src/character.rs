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
// Qwen Girl's clip set drops the "点击回应-" prefix.
const HAPPY_ANIM: &str = "开心跃动";

pub fn pose_anim(pose: &str) -> Option<&'static str> {
    Some(match pose {
        "idle" => IDLE_ANIM,
        "happy" => HAPPY_ANIM,
        "dance" => "可爱宅舞",
        "typing" => "写代码",
        "music" => "悠闲哼歌",
        "walk-left" | "walk-right" => "原地左转奔跑",
        "copy" => "轻快记录",
        "charging" => "工作状态-雀跃庆祝",
        "low-battery" => "工作状态-垂头叹气冒汗",
        "late-night" | "drowsy" => "哈欠连天",
        "asleep" => "原地小憩沉眠",
        "dragged" => "被鼠标拖拽悬空反馈",
        _ => return None,
    })
}
