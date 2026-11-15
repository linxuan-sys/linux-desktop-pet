/// Whether the renderer premultiplies alpha before handing pixels to the
/// compositor. Wayland's Argb8888 is a premultiplied format, so `true` is
/// correct; `false` keeps the authored colours exactly and suits artwork whose
/// antialiased edges carry green-screen residue (that shows up as dark speckle
/// once premultiplied). Override with `premultiply` in the config file.
pub const DEFAULT_PREMULTIPLY: bool = true;

const IDLE_ANIM: &str = "待机";
const HAPPY_ANIM: &str = "开心";

pub fn pose_anim(pose: &str) -> Option<&'static str> {
    Some(match pose {
        "idle" => IDLE_ANIM,
        "happy" => HAPPY_ANIM,
        "dance" => "跳舞",
        "typing" => "写代码",
        "music" => "听音乐",
        "walk-left" | "walk-right" => "奔跑",
        "copy" => "记录",
        "charging" => "庆祝",
        "low-battery" => "没电",
        "late-night" | "drowsy" => "打哈欠",
        "asleep" => "睡觉",
        "dragged" => "被鼠标拖拽悬空反馈",
        _ => return None,
    })
}
