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
        "charging" => "工作状态-雀跃庆祝",
        "low-battery" => "工作状态-垂头叹气冒汗",
        "late-night" | "drowsy" => "哈欠连天",
        "asleep" => "原地小憩沉眠",
        _ => return None,
    })
}
