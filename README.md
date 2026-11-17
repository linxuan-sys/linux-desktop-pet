# Pet

一个原生 Wayland 桌宠程序，使用 Rust、`wlr-layer-shell` 和原始 WebP
动画。一个预编译的 `pet` 程序支持八个角色；动画资源独立发布，首次使用时
按角色下载。

## 使用

安装后直接运行：

```bash
pet tui
```

TUI 提供角色选择、资源下载、大小、动作缓存和 Alpha 预乘设置。常用 CLI
入口：

```bash
pet run kimi
pet download kimi
pet download all
pet stop kimi
pet kimi
pet kimi size 120
pet kimi prepare
```

角色名为 `claude`、`dsh`、`codex`、`grok`、`qwen`、`gemini`、`glm`
和 `kimi`。`pet kimi` 同时承担启动和停止；`pet run kimi` 始终直接运行。

## 安装包

Release 中的程序包只包含预编译的 `pet`、启动器和安装脚本，不包含动画，
接收者不需要 Rust 或编译环境：

```bash
tar -xzf pet-*.tar.gz
cd pet-*/
./install.sh
pet tui
```

默认安装到 `~/.local/bin/pet` 和 `~/.local/bin/pet-launch`，应用菜单入口
为 `Pet`。卸载：

```bash
~/.local/share/pet/uninstall.sh
```

动画保存在各角色自己的 XDG 数据目录中。卸载程序不会删除已下载素材、设置
和位置记录。

## 资源模型

程序与素材完全解耦：

- 二进制和安装包由 GitHub Actions 构建。
- 原始 WebP、`ASSETS.json` 和绿幕清理标记由独立资源 Release 发布。
- TUI 只下载选中的角色，并校验清单、文件大小和 SHA-256。
- 下载后离线可用；原文件不会被重新编码或替换。
- 播放时按需解码、内存裁剪并缓存；完整大图集不会长期常驻。

默认配置位于：

```text
${XDG_CONFIG_HOME:-$HOME/.config}/pet/pet.json
```

可参考仓库中的 `pet.example.json`。位置状态位于
`${XDG_STATE_HOME:-$HOME/.local/state}/pet/`。

## 透明 TUI

启动器使用约 16% 的终端内部背景透明度和不透明文字。niri 平铺默认会把
终端放大到整个工作区，可在 niri 配置中加入：

```kdl
window-rule {
    match app-id="^pet-tui$"
    open-floating true
    opacity 1.0
    geometry-corner-radius 0
    draw-border-with-background false
    focus-ring { off; }
    border { off; }
    shadow { off; }
    background-effect {
        blur false
        xray false
    }
}
```

Kitty 启动器设置 `app-id=pet-tui`，窗口默认约 `96c × 28c`，不再全屏。

## 从源码构建

需要 Rust 1.88 或更新版本：

```bash
./pet build
./pet tui
./pet run kimi --media-dir kimi-girl/media
```

生成仅含程序的发行包：

```bash
python3 scripts/package-release.py --jobs 1
```

生成独立素材包：

```bash
nice -n 15 python3 scripts/package-assets.py all --tag assets-v1
```

发行工作流见 `.github/workflows/`。程序 Release 使用 `pet-*` 文件名；
资源 Release 使用 `assets-*` 标签。

## 画质和渲染

- 原始分辨率、120 帧、12 fps、RGB 与 Alpha 均保持不变。
- 原始 WebP 字节和绿幕清理标记原样保留。
- 已清理素材不会重复处理；旧 Codex Alpha/接缝修复只在解码缓存时执行一次。
- 动作缓存默认目标为 32 MiB，可超出的单个活动动作独立占位。
- 冷启动解码存在短暂内存峰值，完整原图集在裁剪后释放。

需要支持 `zwlr_layer_shell_v1` 的 Wayland 合成器。当前验证目标是 niri
和 Hyprland；X11 不受支持。

## 许可

源代码采用 MIT 许可。动画素材不受源代码 MIT 许可覆盖，素材权利和再分发
条件以上游 `hunmer/aboutme-pets` 为准。
