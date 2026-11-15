<div align="center">
  <img src="docs/poster.png" alt="Rust Desktop Pet Trio — Claude Girl, DSH Girl and Codex Girl on Linux" width="100%">
</div>

<div align="center">
  <video src="docs/2026-11-12-223118-REGION.mp4" controls playsinline preload="metadata" width="100%"></video>
  <p><a href="docs/2026-11-12-223118-REGION.mp4">▶ Watch the demo · 观看演示</a></p>
</div>

<h1 align="center">Rust Desktop Pet Trio</h1>

<p align="center">
  <b>Three little companions, living natively on your Linux desktop.</b><br>
  <b>三位少女，三种陪伴，原生住在你的 Linux 桌面上。</b>
</p>

<p align="center">
  <code>Rust 2024</code> · <code>Linux</code> · <code>Wayland</code> · <code>layer-shell</code> · <code>357 animations</code> · <code>MIT</code>
</p>

---

## English

### Three girls, one native Linux desktop

**Rust Desktop Pet Trio** is a collection of three independent desktop pets for Linux, all written in **Rust**:

- **Claude Girl** — warm orange, 166 hand-drawn animation clips.
- **DSH Girl** — deep ocean blue, 101 hand-drawn animation clips.
- **Codex Girl** — silver violet, 90 hand-drawn animation clips.

Each pet is a small, transparent, always-on-top **Wayland layer-shell** window that lives at the bottom of the screen. They idle, wander, react when you click them, notice when your battery is low, dance when music is playing, and keep you company while you code.

There is no Electron, no browser engine, no X11 fallback, and no background daemon. Just three Rust binaries, 357 hand-drawn 10-second clips, and a state machine that tries to feel alive.

### Why Rust, why Linux, why Wayland

This project is built around three hard requirements.

**Rust, because a desktop pet should be invisible in the process list.** All three pets use Rust 2024 and compile to a single executable of roughly 2 MB with `strip`, LTO, and `opt-level = "s"`. There is no garbage collector, no JavaScript runtime, and no bundled browser. The renderer blits one 640×360 RGBA frame into a shared-memory buffer; the sensors run on ordinary threads. Memory use stays small and predictable while the pet is on screen all day.

**Linux, because that is where these pets live.** The code talks directly to Linux interfaces: `/proc` for process detection, sysfs for battery state, D-Bus/MPRIS for music, the XDG base directories for config and media, and systemd user services for optional autostart. There is no Windows or macOS build, and no plan to pretend otherwise.

**Wayland, because layer-shell is the right primitive.** Each pet creates a `zwlr_layer_shell_v1` overlay surface with a transparent buffer, fractional-scale awareness, click-through transparency, and an input region built from the visible pixels. The tested compositors are **niri** and **Hyprland**; other wlr-layer-shell compositors may work, but focus detection falls back to "no focus information". This is a Wayland-native project, not an XWayland app.

### The trio

| Pet | Directory | Binary | Animations | Palette | Click-happy clip | Walk movement |
| --- | --- | --- | ---: | --- | --- | --- |
| **Claude Girl** | `claude-girl/` | `claude-girl` | 166 | warm orange / copper | 4.0–10.0 s (6 s) | 3.0–5.0 s, 120 px |
| **DSH Girl** | `dsh-girl/` | `dsh-girl` | 101 | deep blue / silver | 3.5–10.0 s (6.5 s) | 1.5–4.5 s, 180 px |
| **Codex Girl** | `codex-girl/` | `codex-girl` | 90 | silver white / violet | 3.0–10.0 s (7 s) | 3.0–5.5 s, 150 px |

The three projects share the same engine and the same pose vocabulary, but each animation set has its own rhythm. The click-happy start and the walk movement window are therefore tuned separately, so each character reaches her jump and her run at the right moment. Every animation file itself is still a complete 10-second clip.

### Shared features

- **357 hand-drawn animations.** Each pose is a 5120×5400 WebP sprite sheet: 8 columns × 15 rows, 120 frames of 640×360 at 12 fps, played as one complete 10-second loop.
- **Native Wayland overlay.** `zwlr_layer_shell_v1`, fractional scaling, transparent background, click-through outside the artwork, input region derived from opaque pixels.
- **Terminal awareness.** Focus detection through niri IPC or the Hyprland socket, matched on the focused window's app id against a list of common Linux terminals. Focusing any terminal switches the pet to work mode; otherwise she follows her relaxed awake routine.
- **A small daily routine.** Work mode runs a fixed 50-second loop: two idle clips, typing, idle, music. It starts as soon as any terminal is focused. Awake mode (no terminal focused) idles, looks around, picks a random animation, and walks. Between 00:00 and 06:00 she alternates a yawn and a nap instead of either, and music still wins over everything.
- **Small reactions.** Happy hop on click, celebration when the charger is plugged in, low-battery reminder, dance when MPRIS reports music, quiet "copy" animation when the clipboard changes. A clipboard copy outranks everything else. Priority, highest first: clipboard copy, music playing (dance), yawning then sleeping after midnight, then the work loop.
- **Walking.** The window moves only during the run window of the clip. Direction cycles two left runs, then two right runs; at a screen edge the window stops and the clip keeps running in place.
- **Readable architecture.** Pet engine, Wayland host and sensors are separate modules. No framework, no runtime, no magic.
- **Configuration and click commands.** A small JSON file maps each mouse button to a terminal command; commands are split on whitespace and spawned directly, with a 1200 ms click debounce.

### Requirements

- Linux with a Wayland compositor that supports `zwlr_layer_shell_v1`. **niri** and **Hyprland** are the tested targets. X11 is not supported.
- Rust 1.85 or newer, because all three crates use edition 2024.
- `pkg-config`, `wayland`, and `wayland-protocols` development files.
- Optional: a working D-Bus session bus for MPRIS music detection.
- Optional: `wl-clipboard` support for the clipboard reaction.

On Arch Linux:

```bash
sudo pacman -S --needed rust pkg-config wayland wayland-protocols
```

### Build

The three pets share one engine (`engine/`) through a Cargo workspace, so a
single command at the repository root builds all of them:

```bash
cd ~/.local/opt/linux-desktop-pet
cargo build --release
```

The binaries are written to one shared target directory:

```text
target/release/claude-girl
target/release/dsh-girl
target/release/codex-girl
```

### Prepare the animation cache (required, once per asset set)

The renderer reads pre-cropped atlases from `<pet>/media/optimized/`, which is
generated from `<pet>/media/anim/`. That directory is not in git, so it must be
built once after cloning:

```bash
pet claude prepare
pet dsh prepare
pet codex prepare
```

Or without the launcher:

```bash
./target/release/claude-girl --media-dir claude-girl/media --prepare-assets
./target/release/dsh-girl    --media-dir dsh-girl/media    --prepare-assets
./target/release/codex-girl  --media-dir codex-girl/media  --prepare-assets
```

This takes a few minutes per pet (166 / 101 / 90 sheets). It is idempotent:
sheets that are already up to date are skipped, so an interrupted run can simply
be restarted. Expect roughly 3.5 GB of output because the atlases are encoded
losslessly.

### Run

The launcher is the easy path — it passes the media directory and stops the
process if it is already running:

```bash
pet claude
pet dsh
pet codex
```

Without the launcher, point the binary at its media directory explicitly:

```bash
./target/release/claude-girl --media-dir claude-girl/media
```

Running one pet per terminal is fine. To start all three detached (fish):

```fish
for p in codex-girl claude-girl dsh-girl
    pkill -x $p 2>/dev/null
    env -C ~/.local/opt/linux-desktop-pet setsid -f ./target/release/$p --media-dir $p/media </dev/null >/dev/null 2>&1
end
```

Check they are alive, and stop them:

```bash
pgrep -a -x 'codex-girl|claude-girl|dsh-girl'
pkill -x 'codex-girl|claude-girl|dsh-girl'
```

The launcher staggers the three along the bottom-right corner when more than one
is running; a single pet parks at the very edge.

### Resize at runtime

```bash
pet codex size 120
```

`50`–`150` percent. The value is saved to the config file and applied to a
running pet within half a second.

### Deploy the launcher (recommended)

Keep the whole repository at:

```text
~/.local/opt/linux-desktop-pet
```

Install the single-file launcher to:

```text
~/.local/bin/pet
```

From an existing checkout, install the launcher with one command:

```bash
install -Dm755 pet ~/.local/bin/pet
```

For a fresh clone, this one-liner creates both directories, clones the repository, and installs the launcher (replace `YOUR_REPO_URL`):

```bash
mkdir -p ~/.local/opt ~/.local/bin && git clone YOUR_REPO_URL ~/.local/opt/linux-desktop-pet && install -Dm755 ~/.local/opt/linux-desktop-pet/pet ~/.local/bin/pet
```

The `pet` file in the repository is the source copy. Edit that one and reinstall it; do not edit `~/.local/bin/pet` directly.

Build once at the repository root, prepare the assets once, then use the launcher:

```bash
cd ~/.local/opt/linux-desktop-pet
cargo build --release
pet claude prepare && pet dsh prepare && pet codex prepare
```

```bash
pet claude
pet codex
pet dsh
```

Running the same command again closes that pet. The launcher expects the repository at `~/.local/opt/linux-desktop-pet`; if you keep it elsewhere, set `PET_ROOT`:

```bash
PET_ROOT=/path/to/linux-desktop-pet pet claude
```

Startup errors are written to `~/.local/state/pet/<name>.log`.

### Install a standalone binary (optional)

Skip this section if you use the `pet` launcher above. Install one pet into your user prefix:

```bash
pet=claude-girl
install -Dm755 "$pet/target/release/$pet" ~/.local/bin/"$pet"
mkdir -p ~/.local/share/"$pet"
cp -r "$pet/media" ~/.local/share/"$pet"/
```

The binary searches for `media/anim` in this order:

1. `$XDG_DATA_HOME/<pet>/media` (usually `~/.local/share/<pet>/media`)
2. `media/` next to the executable
3. `../media`
4. `../../media`
5. `./media`

If you want the pet to start with your graphical session, create a user service. Replace `claude-girl` with `dsh-girl` or `codex-girl` as needed:

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/claude-girl.service <<'SERVICE'
[Unit]
Description=Claude Girl desktop pet
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=%h/.local/bin/claude-girl
Restart=on-failure

[Install]
WantedBy=graphical-session.target
SERVICE
systemctl --user daemon-reload
systemctl --user enable --now claude-girl.service
```

### Configuration

All three pets read one file:

```text
$XDG_CONFIG_HOME/desktop-pet/desktop-pet.json
# usually ~/.config/desktop-pet/desktop-pet.json
```

The file is **not created automatically**. Without it every pet runs on its
built-in defaults. A ready-to-edit template ships in the repository root:

```bash
mkdir -p ~/.config/desktop-pet
cp desktop-pet.example.json ~/.config/desktop-pet/desktop-pet.json
```

`pet <name> size <percent>` also creates the file if it is missing.

Each pet has its own section:

```json
{
  "pets": {
    "claude-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "output": null,
      "click": { "left": "", "middle": "kitty", "right": "kitty claude" }
    },
    "dsh-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "click": { "left": "", "middle": "kitty", "right": "kitty dsh-tui" }
    },
    "codex-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "click": { "left": "", "middle": "kitty", "right": "kitty codex" }
    }
  }
}
```

Each binary selects `pets.<package-name>` and falls back to built-in defaults for missing keys, so you only need to keep the sections you want to change.

| Key | Default | Meaning |
| --- | --- | --- |
| `scale` | `1.0` | Size multiplier, `0.5`–`1.5`. `1.0` draws one buffer pixel per canvas pixel, i.e. no resampling. |
| `cache_mib` | `128` | Decoded-atlas budget in MiB, `128`–`512`. One atlas is roughly 25–70 MiB, so 384 holds about ten clips. |
| `output` | `null` | Connector name to pin the pet to, e.g. `"DP-1"`. `null` follows the compositor. |
| `premultiply` | per pet | Premultiply alpha before writing to the shared buffer. Wayland's `Argb8888` is a premultiplied format, so `true` is correct. Codex Girl defaults to `false` because her sheets carry heavy green-screen residue in the antialiased edge, which premultiplication turns into dark speckle. |
| `click.left` | `""` | Left-click command. Empty means no command; the click still plays the happy reaction. |
| `click.middle` | `kitty` | Middle-click command. |
| `click.right` | per pet | `kitty claude` for Claude Girl, `kitty dsh-tui` for DSH Girl, `kitty codex` for Codex Girl. |

The file is polled every 500 ms and applied without restarting. Invalid values
are rejected and the last good configuration stays in effect. Window position is
remembered across restarts in a separate state file.

Polling intervals are fixed in the binary: system state every 2.5 s, render tick
every 16 ms (62.5 Hz). The renderer only presents when the frame actually
changes, so the higher tick rate costs nothing while a 12 fps clip is playing.

Commands are split on whitespace and spawned directly, without a shell. If you need pipes or quoting, point the config at a small wrapper script. Click commands are debounced for 1200 ms, so a burst of clicks cannot open a terminal swarm.

### How it works

```text
sensors (2.5 s) ──► DerivedState ──► Pet state machine ──► pose ──► renderer (16 ms)
focus socket ─────► SharedCache ───┘
clipboard ────────► reaction ──────┘
```

- **Sensors.** A background thread polls the focused window, MPRIS playback,
  battery and hour every 2.5 seconds. A compositor-specific thread streams focus
  events into a shared cache.
- **Pet engine.** Each pose is locked for a complete 10-second clip. A click and
  a clipboard copy interrupt immediately; charging and other reactions wait for
  the current clip to finish. Music has top priority, then the midnight
  yawn/sleep alternation, then the mode schedule.
- **Modes.** With a terminal focused the pet runs a fixed 50-second loop: idle,
  idle, typing, idle, music. Otherwise it follows the seven-step awake routine:
  idle, idle, random, idle, idle, random, walk.
- **Walking.** The window is stationary before and after the run window,
  interpolates its position during the run, and clamps at the screen edges; at an
  edge the window stops and the clip runs in place.
- **Renderer.** `prepare_assets` crops every one of the 120 cells to its own
  alpha bounding box and shelf-packs the crops into one atlas, cutting a 105 MiB
  sheet to roughly 35 MiB. At runtime `Sheet` decodes atlases on a dedicated
  thread, evicts by byte budget, and writes the current frame straight into the
  shared-memory buffer as premultiplied-free BGRA. Frames are double-buffered and
  released back by `wl_buffer.release`. The layer surface stays transparent; the
  input region follows the visible pixels.

### Animation format

Each file in `<pet>/media/anim/` is a WebP sprite sheet:

- **5120×5400** pixels, **8 columns × 15 rows**
- **120 frames** of **640×360**, played at **12 fps** = **10 seconds**
- Transparent background; the artwork is padded inside each cell

The dedicated pose mapping used by Claude Girl and DSH Girl:

| Pose | Source clip |
| --- | --- |
| `idle` | `待机呼吸休闲` |
| `happy` | `点击回应-开心跃动` |
| `dance` | `可爱宅舞` |
| `typing` | `写代码` |
| `music` | `悠闲哼歌` |
| `walk-left` / `walk-right` | `原地左转奔跑` (right is mirrored) |
| `copy` | `轻快记录` |
| `charging` | `工作状态-雀跃庆祝` |
| `low-battery` | `工作状态-垂头叹气冒汗` |
| `drowsy` | `哈欠连天` |
| `asleep` | `原地小憩沉眠` |

Codex Girl uses the same states with her own clip names: `待机`, `开心`, `跳舞`, `写代码`, `听音乐`, `奔跑`, `记录`, `庆祝`, `没电`, `打哈欠`, `睡觉`.

To add a random animation, put a new WebP in `media/anim/` and add its filename stem to `EASTER_ACTIONS` in `src/pet.rs`. To add a dedicated state, map the new stem in `pose_anim` in `src/gfx.rs`.

### Project layout

```text
linux-desktop-pet/
├── Cargo.toml                workspace: three crates, one shared engine
├── engine/                   shared by all three pets
│   ├── gfx.rs                atlas crop/pack, decode thread, byte-budget LRU, blit
│   ├── runtime.rs            main loop, config hot-reload, signal plumbing
│   ├── settings.rs           config parse, atomic write, placement persistence
│   ├── host/
│   │   ├── mod.rs            Host trait, input regions, RGBA→BGRA
│   │   └── wl.rs             layer-shell surface, double buffering, scaling
│   └── sensors/              focus (niri/Hyprland), MPRIS, battery, clipboard
├── claude-girl/
│   ├── src/
│   │   ├── main.rs           8 lines: points at ../../engine
│   │   ├── character.rs      pose → animation filename
│   │   └── pet.rs            state machine (the only pet-specific logic)
│   └── media/
│       ├── anim/             166 source WebP sheets (in git)
│       └── optimized/        generated atlases + json (git-ignored)
├── dsh-girl/                 same layout, 101 animations
├── codex-girl/               same layout, 90 animations
├── docs/poster.png
├── pet                       launcher: start/stop/size/prepare
├── desktop-pet.example.json  config template — copy to ~/.config/desktop-pet/
├── LICENSE
└── README.md
```

Adding a fourth character means one new crate with three files — `main.rs`,
`character.rs` and `pet.rs` — plus its media directory. No engine changes.

### Troubleshooting

- **`no media/optimized/... ; run --prepare-assets`** — the cropped atlas cache is missing. Run `pet <name> prepare` (a few minutes per pet).
- **`no media/anim directory`** — pass `--media-dir <pet>/media`, or install `media/` into `~/.local/share/<pet>/media`.
- **The pet is 17% smaller than before** — `scale: 1.0` is one buffer pixel per canvas pixel, i.e. no resampling. The old default drew 761 physical pixels where 652 is exact; set `scale` to `1.17` to match that size.
- **The pet is invisible or the layer never appears** — check that you are on Wayland and that your compositor supports `zwlr_layer_shell_v1`. niri and Hyprland are the tested targets.
- **Nothing happens under X11** — X11 is not supported. This is a Wayland-native application.
- **Focus reactions do not fire** — focus detection is implemented for niri and Hyprland. Other compositors fall back to no focus information.
- **No music reaction** — make sure the player exposes MPRIS on the session bus.
- **No clipboard reaction** — make sure `wl-clipboard` support is available.
- **Clicks open terminals unexpectedly** — edit the `click` section of the config. Commands are split on whitespace and spawned without a shell.
- **The three pets overlap** — they are independent windows; overlapping is normal.

### License

The source code in this repository is released under the **MIT License** — see [LICENSE](LICENSE).

**Asset notice.** The character artwork and animation frames in `*/media/anim/` are **not** covered by the MIT license. They come from the [aboutme-pets](https://github.com/hunmer/aboutme-pets) project. Please follow the upstream asset terms; for public redistribution, credit the upstream project. Do not use the artwork commercially without permission.

### Credits

- **Original concept and static-pose state machine:** [adarsh-67r/clawd](https://github.com/adarsh-67r/clawd) (MIT). These pets began as a port of clawd's state machine from static PNG poses to 10-second WebP clips.
- **Character art and animation clips:** [hunmer/aboutme-pets](https://github.com/hunmer/aboutme-pets). Claude Girl, DSH Girl and Codex Girl use the Claude, DSH and Codex spritesheets from that project.
- **Rust ecosystem:** `wayland-client`, `wayland-protocols`, `wayland-protocols-wlr`, `zbus`, `image`, `wl-clipboard-rs`, `serde`, `chrono`.
- The trio poster is composited from the projects' own animation frames.

---

## 中文

### 三位少女，一个原生 Linux 桌面

**Rust 桌宠三重奏** 是三个独立的 Linux 桌面宠物，全部用 **Rust** 编写：

- **Claude Girl** — 暖橙色，166 段手绘动画。
- **DSH Girl** — 深海蓝，101 段手绘动画。
- **Codex Girl** — 银白紫，90 段手绘动画。

每只桌宠都是一个透明、置顶的 **Wayland layer-shell** 窗口，常驻在屏幕底部。她们会发呆、会散步、被点击时会开心跳一下、电量低时会提醒你、有音乐时会跳舞，也会在你写代码时安静陪着你。

没有 Electron，没有浏览器内核，没有 X11 兜底，也没有后台守护进程。只有三个 Rust 可执行文件、357 段手绘的 10 秒动画，和一个努力让自己“活着”的状态机。

### 为什么是 Rust，为什么是 Linux，为什么是 Wayland

这个项目围绕三个硬要求构建。

**用 Rust，因为桌宠不该出现在进程列表的存在感里。** 三只桌宠都使用 Rust 2024，开启 `strip`、LTO 和 `opt-level = "s"` 后，每只约为 2 MB 的单文件可执行程序。没有垃圾回收器，没有 JavaScript 运行时，也没有捆绑浏览器。渲染器只把一帧 640×360 的 RGBA 画面送进共享内存缓冲区，传感器跑在普通线程上。即使常驻一整天，内存占用也小而稳定。

**为 Linux，因为这些桌宠就住在这里。** 代码直接使用 Linux 接口：`/proc` 识别进程，sysfs 读取电池，D-Bus/MPRIS 获取音乐，XDG 目录读取配置和素材，systemd 用户服务负责可选的开机自启。没有 Windows 或 macOS 版本，也不会假装有。

**原生 Wayland，因为 layer-shell 才是正确的底层原语。** 每只桌宠创建一个 `zwlr_layer_shell_v1` 覆盖层，使用透明缓冲区，支持分数缩放；透明区域可以点击穿透，输入区域只跟随实际可见的像素。已验证的合成器是 **niri** 和 **Hyprland**；其他支持 wlr-layer-shell 的合成器可能可用，但焦点检测会退化成“没有焦点信息”。这是原生 Wayland 应用，不是 XWayland 程序。

### 三只桌宠

| 桌宠 | 目录 | 二进制 | 动画数量 | 主色调 | 点击开心片段 | 奔跑移动窗口 |
| --- | --- | --- | ---: | --- | --- | --- |
| **Claude Girl** | `claude-girl/` | `claude-girl` | 166 | 暖橙 / 铜色 | 4.0–10.0 秒（6 秒） | 3.0–5.0 秒，120 像素 |
| **DSH Girl** | `dsh-girl/` | `dsh-girl` | 101 | 深海蓝 / 银色 | 3.5–10.0 秒（6.5 秒） | 1.5–4.5 秒，180 像素 |
| **Codex Girl** | `codex-girl/` | `codex-girl` | 90 | 银白 / 紫色 | 3.0–10.0 秒（7 秒） | 3.0–5.5 秒，150 像素 |

三个项目共用同一套引擎和姿势词汇，但每套动画的节奏不同。因此点击开心的起点和奔跑移动窗口分别调过，保证每个角色在合适的时间起跳、起跑。每个动画文件本身仍然是完整的 10 秒片段。

### 共同特性

- **357 段手绘动画。** 每个姿势是一张 5120×5400 的 WebP 精灵图：8 列 × 15 行，120 帧 640×360，12 fps，完整播放 10 秒。
- **原生 Wayland 覆盖层。** `zwlr_layer_shell_v1`、分数缩放、透明背景、画面外点击穿透，输入区域由不透明像素决定。
- **感知终端。** 通过 niri IPC 或 Hyprland socket 读取焦点窗口的 app id，和常见 Linux 终端名单比对。聚焦任意终端就进入工作模式，否则按清醒日程生活。
- **自己的小日常。** 工作模式固定 50 秒循环：两段发呆、一段写代码、一段发呆、一段听音乐，只要聚焦任意终端就开始。清醒模式（没聚焦终端）发呆、东张西望、随机做动作、起身散步。00:00–06:00 之间改为哈欠和睡觉轮流播放，音乐仍然盖过一切。
- **小小的反应。** 点击开心跳、插电源庆祝、低电量提醒、MPRIS 有音乐时跳舞、剪贴板变化时安静“记录”。复制剪贴板的优先级高于其他所有状态。优先级从高到低：复制剪贴板、音乐播放（跳舞）、0 点后打哈欠再睡、工作循环。
- **走路。** 只在动画的奔跑窗口移动；方向循环是两次左跑、两次右跑；碰到屏幕边缘会 clamp，窗口停住但动画继续原地踏步。
- **可读的架构。** 宠物引擎、Wayland 宿主、传感器各自独立；没有框架、没有运行时、没有魔法。
- **配置与点击命令。** 一个 JSON 把每个鼠标键映射到终端命令；命令按空白拆分后直接启动，点击有 1200 毫秒防抖。

### 环境要求

- Linux + 支持 `zwlr_layer_shell_v1` 的 Wayland 合成器。**niri** 和 **Hyprland** 是已验证目标。不支持 X11。
- Rust 1.85 或更新版本，三个 crate 都使用 edition 2024。
- `pkg-config`、`wayland`、`wayland-protocols` 开发文件。
- 可选：可用的 D-Bus 会话总线，用于 MPRIS 音乐检测。
- 可选：`wl-clipboard` 支持，用于剪贴板反应。

Arch Linux：

```bash
sudo pacman -S --needed rust pkg-config wayland wayland-protocols
```

### 编译

三只桌宠通过 Cargo workspace 共用同一份引擎（`engine/`），所以在仓库根目录一条命令就能全部编译：

```bash
cd ~/.local/opt/linux-desktop-pet
cargo build --release
```

产物在同一个 target 目录下：

```text
target/release/claude-girl
target/release/dsh-girl
target/release/codex-girl
```

### 生成素材缓存（必须，每套素材跑一次）

渲染器读的是 `<pet>/media/optimized/` 里的裁剪图集，它由 `<pet>/media/anim/` 生成。这个目录不进 git，所以克隆后必须生成一次：

```bash
pet claude prepare
pet dsh prepare
pet codex prepare
```

不用启动脚本的话：

```bash
./target/release/claude-girl --media-dir claude-girl/media --prepare-assets
./target/release/dsh-girl    --media-dir dsh-girl/media    --prepare-assets
./target/release/codex-girl  --media-dir codex-girl/media  --prepare-assets
```

每只几分钟（166 / 101 / 90 张）。这个过程是幂等的：已经是最新的素材会直接跳过，所以中途中断了直接重跑即可。因为是无损编码，输出大约 3.5 GB。

### 运行

用启动脚本最省事——它会传好素材目录，而且已经在跑的时候再执行就是关闭：

```bash
pet claude
pet dsh
pet codex
```

不用启动脚本的话，要显式指定素材目录：

```bash
./target/release/claude-girl --media-dir claude-girl/media
```

每只开一个终端也行。要让三只同时后台运行（fish）：

```fish
for p in codex-girl claude-girl dsh-girl
    pkill -x $p 2>/dev/null
    env -C ~/.local/opt/linux-desktop-pet setsid -f ./target/release/$p --media-dir $p/media </dev/null >/dev/null 2>&1
end
```

确认存活、以及全部关闭：

```bash
pgrep -a -x 'codex-girl|claude-girl|dsh-girl'
pkill -x 'codex-girl|claude-girl|dsh-girl'
```

同时开多只时，启动脚本会让它们沿右下角依次错开；只开一只时贴在屏幕最边缘。

### 运行时调大小

```bash
pet codex size 120
```

`50`–`150` 百分比。这个值会写进配置文件，运行中的桌宠半秒内生效。

### 部署启动脚本（推荐）

整个仓库放在：

```text
~/.local/opt/linux-desktop-pet
```

单文件启动脚本安装到：

```text
~/.local/bin/pet
```

如果仓库已经在本地，用一条命令安装启动脚本：

```bash
install -Dm755 pet ~/.local/bin/pet
```

全新克隆时，用下面这一条同时创建目录、克隆仓库并安装脚本（把 `YOUR_REPO_URL` 换成你的仓库地址）：

```bash
mkdir -p ~/.local/opt ~/.local/bin && git clone YOUR_REPO_URL ~/.local/opt/linux-desktop-pet && install -Dm755 ~/.local/opt/linux-desktop-pet/pet ~/.local/bin/pet
```

仓库里的 `pet` 是源文件，以后改脚本改这一份，再重新 `install` 覆盖；不要直接改 `~/.local/bin/pet`。

先在仓库根目录编译一次、生成一次素材，之后就能用启动脚本了：

```bash
cd ~/.local/opt/linux-desktop-pet
cargo build --release
pet claude prepare && pet dsh prepare && pet codex prepare
```

```bash
pet claude
pet codex
pet dsh
```

同一个命令再执行一次就是关闭对应桌宠。脚本默认项目在 `~/.local/opt/linux-desktop-pet`；如果放在别处，用 `PET_ROOT` 指定：

```bash
PET_ROOT=/path/to/linux-desktop-pet pet claude
```

启动失败的日志在 `~/.local/state/pet/<name>.log`。

### 安装单只二进制（可选）

如果你使用上面的 `pet` 启动脚本，可以跳过这一节。把一只桌宠装进用户目录：

```bash
pet=claude-girl
install -Dm755 "$pet/target/release/$pet" ~/.local/bin/"$pet"
mkdir -p ~/.local/share/"$pet"
cp -r "$pet/media" ~/.local/share/"$pet"/
```

二进制按以下顺序查找 `media/anim`：

1. `$XDG_DATA_HOME/<pet>/media`（通常是 `~/.local/share/<pet>/media`）
2. 可执行文件旁边的 `media/`
3. `../media`
4. `../../media`
5. `./media`

如果想让桌宠随图形会话启动，可以创建 systemd 用户服务。把 `claude-girl` 换成 `dsh-girl` 或 `codex-girl` 即可：

```bash
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/claude-girl.service <<'SERVICE'
[Unit]
Description=Claude Girl desktop pet
After=graphical-session.target
PartOf=graphical-session.target

[Service]
ExecStart=%h/.local/bin/claude-girl
Restart=on-failure

[Install]
WantedBy=graphical-session.target
SERVICE
systemctl --user daemon-reload
systemctl --user enable --now claude-girl.service
```

### 配置

三只桌宠共用一份配置：

```text
$XDG_CONFIG_HOME/desktop-pet/desktop-pet.json
# 通常是 ~/.config/desktop-pet/desktop-pet.json
```

这个文件**不会自动生成**。没有它时每只桌宠都跑内置默认值。仓库根目录带了一份可直接改的模板：

```bash
mkdir -p ~/.config/desktop-pet
cp desktop-pet.example.json ~/.config/desktop-pet/desktop-pet.json
```

`pet <名字> size <百分比>` 在文件缺失时也会顺手创建它。

每只桌宠有自己的段：

```json
{
  "pets": {
    "claude-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "output": null,
      "click": { "left": "", "middle": "kitty", "right": "kitty claude" }
    },
    "dsh-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "click": { "left": "", "middle": "kitty", "right": "kitty dsh-tui" }
    },
    "codex-girl": {
      "scale": 1.0,
      "cache_mib": 384,
      "click": { "left": "", "middle": "kitty", "right": "kitty codex" }
    }
  }
}
```

每只按 `pets.<包名>` 选择自己的段，缺失的键回退到内置默认值，所以只保留想改的段也可以。

| 键 | 默认值 | 含义 |
| --- | --- | --- |
| `scale` | `1.0` | 大小倍数，`0.5`–`1.5`。`1.0` 表示一个缓冲区像素对应一个画布像素，也就是完全不重采样。 |
| `cache_mib` | `128` | 已解码图集的内存预算，单位 MiB，`128`–`512`。单张图集约 25–70 MiB，所以 384 大概能存十段动画。 |
| `output` | `null` | 固定在某块屏幕上的连接器名，例如 `"DP-1"`。`null` 表示跟随合成器。 |
| `premultiply` | 按桌宠 | 写入共享缓冲区前是否预乘 alpha。Wayland 的 `Argb8888` 是预乘格式，所以 `true` 才正确。Codex Girl 默认 `false`，因为她的素材抗锯齿边缘残留了大量绿幕，预乘会把它压成深色雪花点。 |
| `click.left` | `""` | 左键命令。空字符串表示不启动程序，但仍然播放点击开心反应。 |
| `click.middle` | `kitty` | 中键命令。 |
| `click.right` | 按桌宠 | Claude Girl 是 `kitty claude`，DSH Girl 是 `kitty dsh-tui`，Codex Girl 是 `kitty codex`。 |

配置文件每 500 毫秒检查一次，改完不用重启就生效。非法值会被拒绝，并保持上一份有效配置。窗口位置记在单独的状态文件里，重启后恢复。

轮询间隔固定在程序里：系统状态每 2.5 秒，渲染节拍每 16 毫秒（62.5 Hz）。渲染器只在画面真的变化时才提交，所以播 12 fps 动画时更高的节拍不产生额外开销。

命令按空白拆分后直接启动，不经过 shell。如果需要管道或引号，请把配置指向一个小包装脚本。点击命令有 1200 毫秒防抖，连续点击不会开出一堆终端。

### 工作原理

```text
传感器（2.5 秒）──► DerivedState ──► 宠物状态机 ──► 姿势 ──► 渲染器（16 毫秒）
焦点 socket ──────► SharedCache ───┘
剪贴板 ───────────► 反应 ──────────┘
```

- **传感器。** 后台线程每 2.5 秒轮询焦点窗口、MPRIS 播放状态、电池和小时。合成器专用线程把焦点事件写入共享缓存。
- **宠物引擎。** 每个姿势锁定一整段 10 秒动画。点击和复制剪贴板可以立刻打断；充电等反应会等当前片段播完。音乐优先级最高，其次是 0 点后的哈欠/睡觉交替，最后才是模式日程。
- **模式。** 聚焦终端时跑固定的 50 秒循环：发呆、发呆、写代码、发呆、听音乐。否则走七步清醒日程：发呆、发呆、随机、发呆、发呆、随机、走路。
- **走路。** 奔跑窗口之前和之后窗口保持不动，窗口期间插值移动，碰到屏幕边缘会 clamp；贴边时窗口停住，动画继续原地踏步。
- **渲染器。** `prepare_assets` 把 120 个单元格各自裁到自己的 alpha 包围盒，再把裁剪块紧凑打包成一张图集，一张 105 MiB 的精灵图降到约 35 MiB。运行时 `Sheet` 在独立线程上解码，按字节预算淘汰，并把当前帧直接写进共享内存缓冲区（BGRA）。双缓冲，靠 `wl_buffer.release` 归还。layer 表面保持透明，输入区域跟随可见像素。

### 动画格式

每个 `<pet>/media/anim/` 文件都是一张 WebP 精灵图：

- **5120×5400** 像素，**8 列 × 15 行**
- **120 帧**，每帧 **640×360**，**12 fps** = **10 秒**
- 透明背景，角色画面在单元格内留有边距

Claude Girl 和 DSH Girl 使用的专属姿势映射：

| 姿势 | 素材名 |
| --- | --- |
| `idle` | `待机呼吸休闲` |
| `happy` | `点击回应-开心跃动` |
| `dance` | `可爱宅舞` |
| `typing` | `写代码` |
| `music` | `悠闲哼歌` |
| `walk-left` / `walk-right` | `原地左转奔跑`（右侧为镜像） |
| `copy` | `轻快记录` |
| `charging` | `工作状态-雀跃庆祝` |
| `low-battery` | `工作状态-垂头叹气冒汗` |
| `drowsy` | `哈欠连天` |
| `asleep` | `原地小憩沉眠` |

Codex Girl 使用同一组状态，但素材名是：`待机`、`开心`、`跳舞`、`写代码`、`听音乐`、`奔跑`、`记录`、`庆祝`、`没电`、`打哈欠`、`睡觉`。

想增加随机动画，把新的 WebP 放进 `media/anim/`，再把文件名（不含扩展名）加入 `src/pet.rs` 的 `EASTER_ACTIONS`。想增加专属状态，在 `src/gfx.rs` 的 `pose_anim` 里映射新的素材名。

### 项目结构

```text
linux-desktop-pet/
├── Cargo.toml                workspace：三个 crate，一份共享引擎
├── engine/                   三只共用
│   ├── gfx.rs                图集裁剪/打包、解码线程、字节预算 LRU、绘制
│   ├── runtime.rs            主循环、配置热重载、信号串联
│   ├── settings.rs           配置解析、原子写入、位置持久化
│   ├── host/
│   │   ├── mod.rs            Host trait、输入区域、RGBA→BGRA
│   │   └── wl.rs             layer-shell 表面、双缓冲、缩放
│   └── sensors/              焦点（niri/Hyprland）、MPRIS、电池、剪贴板
├── claude-girl/
│   ├── src/
│   │   ├── main.rs           8 行：指向 ../../engine
│   │   ├── character.rs      姿势 → 动画文件名
│   │   └── pet.rs            状态机（唯一属于这只桌宠的逻辑）
│   └── media/
│       ├── anim/             166 张原始 WebP 精灵图（进 git）
│       └── optimized/        生成的图集 + json（不进 git）
├── dsh-girl/                 同样的结构，101 段动画
├── codex-girl/               同样的结构，90 段动画
├── docs/poster.png
├── pet                       启动脚本：启停 / 调大小 / 生成素材
├── desktop-pet.example.json  配置模板 —— 复制到 ~/.config/desktop-pet/
├── LICENSE
└── README.md
```

想加第四只角色，只需要新建一个 crate，里面三个文件——`main.rs`、`character.rs`、`pet.rs`——再加它的素材目录，引擎完全不用改。

### 常见问题

- **提示 `no media/optimized/... ; run --prepare-assets`** — 裁剪图集缓存缺失。跑 `pet <名字> prepare`（每只几分钟）。
- **提示 `no media/anim directory`** — 用 `--media-dir <pet>/media` 指定，或者把 `media/` 安装到 `~/.local/share/<pet>/media`。
- **桌宠比以前小了 17%** — `scale: 1.0` 是一个缓冲区像素对应一个画布像素，即完全不重采样。旧默认值是 761 物理像素而 652 才是精确值；想回到那个大小就把 `scale` 设成 `1.17`。
- **桌宠不显示，layer 表面没出现** — 确认你在 Wayland 会话，并且合成器支持 `zwlr_layer_shell_v1`。已验证的是 niri 和 Hyprland。
- **X11 下完全没反应** — 不支持 X11，这是原生 Wayland 应用。
- **焦点反应不触发** — 焦点检测只实现了 niri 和 Hyprland；其他合成器会退化成没有焦点信息。
- **没有音乐反应** — 确认播放器在会话总线上暴露了 MPRIS。
- **没有剪贴板反应** — 确认系统提供 `wl-clipboard` 支持。
- **点击意外打开终端** — 修改配置里的 `click` 段。命令按空白拆分后直接启动，不经过 shell。
- **三只桌宠叠在一起** — 它们是独立窗口，重叠是正常现象。

### 许可证

本仓库的源代码采用 **MIT 许可证**，详见 [LICENSE](LICENSE)。

**素材声明。** `*/media/anim/` 中的角色立绘与动画帧**不包含在 MIT 许可内**。素材来自 [aboutme-pets](https://github.com/hunmer/aboutme-pets) 项目。请遵守上游素材条款；公开分发时请署名上游项目；未经许可不得商用。

### 致谢

- **原始概念与静态姿势状态机：** [adarsh-67r/clawd](https://github.com/adarsh-67r/clawd)（MIT）。这三只桌宠最初是把 clawd 的状态机从静态 PNG 姿势移植到 10 秒 WebP 动画。
- **角色立绘与动画素材：** [hunmer/aboutme-pets](https://github.com/hunmer/aboutme-pets)。Claude Girl、DSH Girl、Codex Girl 分别使用该项目的 Claude、DSH、Codex 精灵图。
- **Rust 生态：** `wayland-client`、`wayland-protocols`、`wayland-protocols-wlr`、`zbus`、`image`、`wl-clipboard-rs`、`serde`、`chrono`。
- 三只同框海报由项目自己的动画帧合成。

---

## Thanks / 致谢

To **hunmer** and the [aboutme-pets](https://github.com/hunmer/aboutme-pets) project: thank you for drawing these characters and for sharing the animation library. Without your artwork, these little desktop companions would not exist.

To **adarsh-67r** and the [clawd](https://github.com/adarsh-67r/clawd) project: thank you for the original desktop-pet concept, the pose state machine, and the MIT-licensed foundation this engine grew from.

感谢 **hunmer** 和 [aboutme-pets](https://github.com/hunmer/aboutme-pets)：谢谢你画出这些角色，并公开整套动画素材。没有这些立绘和动画，这三只桌宠就不会存在。

感谢 **adarsh-67r** 和 [clawd](https://github.com/adarsh-67r/clawd)：谢谢你最初的桌宠概念、姿势状态机，以及这套引擎生长其上的 MIT 代码基础。
