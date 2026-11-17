#[path = "../../engine/assets.rs"]
mod assets;
pub mod character;
mod control;
#[path = "../../engine/gfx.rs"]
pub mod gfx;
#[path = "../../engine/host/mod.rs"]
pub mod host;
pub mod pet;
#[path = "../../engine/runtime.rs"]
pub mod runtime;
#[path = "../../engine/sensors/mod.rs"]
pub mod sensors;
#[path = "../../engine/settings.rs"]
pub mod settings;
mod tui;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = dispatch(&args);
    if let Err(error) = result {
        eprintln!("pet: {error}");
        std::process::exit(1);
    }
}

fn dispatch(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            usage();
            Ok(())
        }
        Some("--version" | "-V") => {
            println!("pet {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("tui") => {
            if args.len() != 1 {
                return Err("usage: pet tui".into());
            }
            tui::run()
        }
        Some("run") => {
            let Some(name) = args.get(1) else {
                return Err("usage: pet run <角色> [参数]".into());
            };
            let pet = character::select(name)?;
            let options = &args[2..];
            if std::env::var_os("PET_RUN_FOREGROUND").is_some()
                || options.first().is_some_and(|arg| arg == "--foreground")
            {
                let options = if options.first().is_some_and(|arg| arg == "--foreground") {
                    &options[1..]
                } else {
                    options
                };
                return runtime::entry(options);
            }
            if options.is_empty() {
                assets::download_cli()?;
                println!("{}", control::start_for(pet)?);
                return Ok(());
            }
            runtime::entry(options)
        }
        Some("download") => {
            let Some(name) = args.get(1) else {
                return Err("usage: pet download <角色>|all".into());
            };
            if name == "all" {
                return download_all();
            }
            character::select(name)
                .and_then(|_| assets::download_cli())
                .map(|path| {
                    println!("动画资源已就绪：{}", path.display());
                })
        }
        Some("assets-dir") => assets_dir(&args[1..]),
        Some("stop") => args.get(1).map_or_else(
            || Err("usage: pet stop <角色>".into()),
            |name| character::select(name).and_then(|_| control::stop()),
        ),
        Some("build") => {
            if args.len() != 1 {
                return Err("usage: pet build".into());
            }
            build()
        }
        Some("prepare") => {
            if args.len() != 1 {
                return Err("usage: pet prepare".into());
            }
            eprintln!("新版按角色按需下载，正在等价执行 pet download all…");
            download_all()
        }
        Some(name) if !name.starts_with('-') => legacy_pet(name, &args[1..]),
        _ => Err("未知命令；运行 pet --help".into()),
    }
}

fn legacy_pet(name: &str, args: &[String]) -> Result<(), String> {
    let pet = character::select(name)?;
    match args {
        [] => toggle_pet(pet),
        [command] if command == "build" => build(),
        [command] if command == "prepare" => {
            assets::ensure(&assets::default_media()?).map(|path| {
                println!("动画资源已就绪：{}", path.display());
            })
        }
        [command, percent] if command == "size" => set_size(pet, percent),
        _ => Err(format!(
            "用法：pet {name} | pet {name} size 50..150 | pet {name} prepare | pet {name} build"
        )),
    }
}

fn toggle_pet(pet: &'static str) -> Result<(), String> {
    if control::pet_running(pet) {
        control::stop_for(pet)?;
        println!("{pet} 已关闭");
        return Ok(());
    }
    assets::ensure(&assets::default_media()?)?;
    println!("{}", control::start_for(pet)?);
    Ok(())
}

fn set_size(pet: &str, percent: &str) -> Result<(), String> {
    let value = percent
        .parse::<f64>()
        .map_err(|_| "大小必须是 50–150 之间的整数百分比".to_string())?;
    if !value.is_finite() || !(50.0..=150.0).contains(&value) {
        return Err("大小必须在 50–150 之间".into());
    }
    settings::update_for(pet, |raw| {
        raw["pets"][pet]["scale"] = serde_json::json!(value / 100.0)
    })?;
    println!("{pet} 大小已保存为 {value:.0}%");
    Ok(())
}

fn assets_dir(args: &[String]) -> Result<(), String> {
    match args {
        [] => {
            println!("{}", settings::configured_assets_root()?.display());
            Ok(())
        }
        [value] if value == "default" => {
            let root = settings::set_assets_root("", true)?;
            println!("素材目录已恢复默认：{}", root.display());
            Ok(())
        }
        [value] => {
            let root = settings::set_assets_root(value, false)?;
            println!("素材目录已设为：{}", root.display());
            println!("原有素材不会移动；重启桌宠后使用新目录。");
            Ok(())
        }
        _ => Err("usage: pet assets-dir [目录|default]".into()),
    }
}

fn download_all() -> Result<(), String> {
    for pet in character::PETS {
        let status =
            std::process::Command::new(std::env::current_exe().map_err(|error| error.to_string())?)
                .args(["download", pet])
                .status()
                .map_err(|error| format!("{pet}: {error}"))?;
        if !status.success() {
            return Err(format!("{pet} 下载失败"));
        }
    }
    Ok(())
}

fn build() -> Result<(), String> {
    let root = std::env::var_os("PET_ROOT")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|path| path.join("Cargo.toml").is_file())
        })
        .ok_or("build 需要源码目录；请进入仓库或设置 PET_ROOT")?;
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "-p", "desktop-pet"])
        .current_dir(root)
        .status()
        .map_err(|error| error.to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "cargo build 失败".into())
}

fn usage() {
    println!(
        "pet tui                         打开设置与下载 TUI\n\
         pet run <角色>                   下载后后台启动并立即返回\n\
         pet run <角色> --foreground      留在当前终端前台运行（调试用）\n\
         pet run <角色> [参数]           高级运行参数在前台执行\n\
         pet download <角色>|all         下载并校验原始 WebP 资源\n\
         pet assets-dir [目录|default]   查看或设置素材存储根目录\n\
         pet stop <角色>                 停止指定角色\n\
         pet <角色>                      启动；已在运行时关闭\n\
         pet <角色> size 50..150         设置大小\n\
         pet <角色> prepare | build      单角色操作入口\n\
         pet prepare | build             下载全部角色资源 / 编译统一程序\n\
         角色：claude, dsh, codex, grok, qwen, gemini, glm, kimi"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_and_version_do_not_select_a_pet() {
        assert!(dispatch(&["--help".into()]).is_ok());
        assert!(dispatch(&["--version".into()]).is_ok());
    }

    #[test]
    fn legacy_size_is_validated_before_writing() {
        assert!(set_size(crate::character::PETS[0], "49").is_err());
        assert!(set_size(crate::character::PETS[0], "151").is_err());
    }
}
