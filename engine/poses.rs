//! One pose-to-animation map shared by unified and standalone programs.
pub fn animation_for(pet: &str, pose: &str) -> Option<&'static str> {
    let (idle, happy) = match pet {
        "codex-girl" => ("待机", "开心"),
        "qwen-girl" => ("待机呼吸休闲", "开心跃动"),
        _ => ("待机呼吸休闲", "点击回应-开心跃动"),
    };
    let name = match (pet, pose) {
        (_, "idle") => idle,
        (_, "happy") => happy,
        ("codex-girl", "dance") => "跳舞",
        ("codex-girl", "music") => "听音乐",
        ("codex-girl", "walk-left" | "walk-right") => "奔跑",
        ("codex-girl", "copy") => "记录",
        ("codex-girl", "charging") => "庆祝",
        ("codex-girl", "low-battery") => "没电",
        ("codex-girl", "late-night" | "drowsy") => "打哈欠",
        ("codex-girl", "asleep") => "睡觉",
        ("gemini-girl" | "grok-girl", "low-battery") => "工作状态-思考冒泡",
        ("glm-girl", "charging") => "小蓝狐现世",
        ("glm-girl", "low-battery") => "原地重力下蹲压缩",
        (_, "dance") => "可爱宅舞",
        (_, "typing") => "写代码",
        (_, "music") => "悠闲哼歌",
        (_, "walk-left" | "walk-right") => "原地左转奔跑",
        (_, "copy") => "轻快记录",
        (_, "charging") => "工作状态-雀跃庆祝",
        (_, "low-battery") => "工作状态-垂头叹气冒汗",
        (_, "late-night" | "drowsy") => "哈欠连天",
        (_, "asleep") => "原地小憩沉眠",
        (_, "dragged") => "被鼠标拖拽悬空反馈",
        _ => return None,
    };
    Some(name)
}
