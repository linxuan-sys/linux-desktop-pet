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
        _ => return None,
    })
}
