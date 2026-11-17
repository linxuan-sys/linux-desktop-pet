use std::sync::OnceLock;

pub const PETS: &[&str] = &[
    "claude-girl",
    "dsh-girl",
    "codex-girl",
    "grok-girl",
    "qwen-girl",
    "gemini-girl",
    "glm-girl",
    "kimi-girl",
];
static SELECTED: OnceLock<&'static str> = OnceLock::new();

pub fn select(value: &str) -> Result<&'static str, String> {
    let value = value.strip_suffix("-girl").unwrap_or(value);
    let name = PETS
        .iter()
        .copied()
        .find(|name| name.strip_suffix("-girl") == Some(value))
        .ok_or_else(|| {
            format!(
                "unknown pet {value}; choose claude, dsh, codex, grok, qwen, gemini, glm or kimi"
            )
        })?;
    if let Some(current) = SELECTED.get() {
        return (*current == name)
            .then_some(name)
            .ok_or_else(|| "pet already selected".into());
    }
    SELECTED.set(name).map_err(|_| "pet already selected")?;
    Ok(name)
}

pub fn selected() -> &'static str {
    SELECTED.get().copied().unwrap_or(env!("CARGO_PKG_NAME"))
}

pub fn pose_anim(pose: &str) -> Option<&'static str> {
    let (idle, happy) = match selected() {
        "codex-girl" => ("待机", "开心"),
        "qwen-girl" => ("待机呼吸休闲", "开心跃动"),
        _ => ("待机呼吸休闲", "点击回应-开心跃动"),
    };
    let name = match (selected(), pose) {
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

pub const DEFAULT_PREMULTIPLY: bool = true;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_resolve_without_changing_identity() {
        let selected = *SELECTED.get_or_init(|| "kimi-girl");
        assert_eq!(
            select(selected.strip_suffix("-girl").unwrap()).unwrap(),
            selected
        );
        assert_eq!(select(selected).unwrap(), selected);
        assert!(select("not-a-pet").is_err());
    }
}
