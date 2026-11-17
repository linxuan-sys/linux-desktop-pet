mod theme;

use crate::{assets, character, control, settings};
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use theme::{CORAL, DIM, GOLD, GREEN, Theme};

const NAMES: &[&str] = &[
    "Claude", "DSH", "Codex", "Grok", "Qwen", "Gemini", "GLM", "Kimi",
];
const COLORS: &[(u8, u8, u8)] = &[
    (0xe4, 0xbf, 0x79),
    (0xae, 0xbd, 0xe8),
    (0xd8, 0xc9, 0xf0),
    (0xe3, 0x8c, 0x9a),
    (0x9c, 0xcf, 0xe8),
    (0xc9, 0xb6, 0xe4),
    (0xe8, 0xa6, 0xbf),
    (0xae, 0xbd, 0xe8),
];

#[derive(Clone, Copy, PartialEq)]
enum Focus {
    Pets,
    Settings,
}

#[derive(Clone)]
enum MediaState {
    Ready(assets::MediaLocation),
    Missing,
    Invalid(String),
}

#[derive(Clone, Copy)]
enum PathTarget {
    ManualMedia,
    AssetsRoot,
}

struct PathEdit {
    target: PathTarget,
    text: String,
}

struct Download {
    pet: &'static str,
    receiver: Receiver<Result<PathBuf, String>>,
    progress: Receiver<assets::Progress>,
    received: u64,
    total: u64,
    verifying: bool,
    start_after_download: bool,
}

struct App {
    selected: usize,
    field: usize,
    focus: Focus,
    theme: Theme,
    notice: String,
    download: Option<Download>,
    progress: f64,
    editing_path: Option<PathEdit>,
    media: BTreeMap<&'static str, MediaState>,
}

impl App {
    fn new() -> Self {
        let media = character::PETS
            .iter()
            .map(|pet| (*pet, Self::resolve_media(pet)))
            .collect();
        Self {
            selected: 0,
            field: 0,
            focus: Focus::Pets,
            theme: Theme::detect(),
            notice: "本地或手动素材优先；按 d 检查更新，只有内容变化时才重新下载。".into(),
            download: None,
            progress: 0.0,
            editing_path: None,
            media,
        }
    }

    fn resolve_media(pet: &str) -> MediaState {
        match assets::resolve_media_for(pet, None) {
            Ok(Some(media)) => MediaState::Ready(media),
            Ok(None) => MediaState::Missing,
            Err(error) => MediaState::Invalid(error),
        }
    }

    fn refresh_media(&mut self, pet: &'static str) {
        self.media.insert(pet, Self::resolve_media(pet));
    }

    fn pet(&self) -> &'static str {
        character::PETS[self.selected]
    }

    fn values(&self) -> Result<settings::Settings, String> {
        let bytes = std::fs::read(settings::config_path());
        let raw = match bytes {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error.to_string()),
        };
        settings::Settings::parse_for(&raw, self.pet())
    }

    fn change(&mut self, amount: i32) {
        let pet = self.pet();
        let result = (|| {
            let current = self.values()?;
            match self.field {
                0 => {
                    let percent =
                        ((current.scale * 100.0).round() as i32 + amount * 5).clamp(50, 150);
                    settings::update_for(pet, |raw| {
                        raw["pets"][pet]["scale"] = serde_json::json!(percent as f64 / 100.0)
                    })
                }
                1 => {
                    let mib = (current.cache_mib as i32 + amount * 16).clamp(16, 512);
                    settings::update_for(pet, |raw| {
                        raw["pets"][pet]["cache_mib"] = serde_json::json!(mib)
                    })
                }
                2 => settings::update_for(pet, |raw| {
                    raw["pets"][pet]["premultiply"] = serde_json::json!(!current.premultiply)
                }),
                _ => Ok(()),
            }
        })();
        self.notice = match result {
            Ok(()) => "设置已保存，运行中的宠物会在半秒内应用。".into(),
            Err(error) => error,
        };
    }

    fn begin_path_edit(&mut self, target: PathTarget) {
        let result = self.values().map(|values| match target {
            PathTarget::ManualMedia => values
                .media_dir
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            PathTarget::AssetsRoot => values.assets_root.display().to_string(),
        });
        match result {
            Ok(path) => {
                self.editing_path = Some(PathEdit { target, text: path });
                self.notice = match target {
                    PathTarget::ManualMedia => {
                        "编辑手动素材目录；输入 default 恢复自动选择，Esc 取消。".into()
                    }
                    PathTarget::AssetsRoot => {
                        "编辑下载存储目录；输入 default 恢复默认，Esc 取消。".into()
                    }
                };
            }
            Err(error) => self.notice = error,
        }
    }

    fn finish_path_edit(&mut self) {
        let Some(edit) = self.editing_path.take() else {
            return;
        };
        let reset = edit.text.trim() == "default";
        let pet = self.pet();
        let result = match edit.target {
            PathTarget::ManualMedia => {
                let result = if reset {
                    settings::set_media_dir(pet, "", true).map(|_| ())
                } else {
                    settings::resolve_media_dir_input(&edit.text)
                        .and_then(|path| assets::resolve_media_for(pet, Some(&path)).map(|_| ()))
                        .and_then(|()| settings::set_media_dir(pet, &edit.text, false).map(|_| ()))
                };
                result.map(|()| match reset {
                    true => "手动素材目录已恢复为自动选择。".to_string(),
                    false => "手动素材目录已保存；有素材时不会重复下载。".to_string(),
                })
            }
            PathTarget::AssetsRoot => settings::set_assets_root(&edit.text, reset).map(|root| {
                format!(
                    "下载存储目录已{}：{}",
                    if reset { "恢复默认" } else { "保存" },
                    root.display()
                )
            }),
        };
        self.refresh_media(pet);
        self.notice = match result {
            Ok(message) => message,
            Err(error) => error,
        };
    }

    fn download(&mut self, start_after_download: bool) {
        if self.download.is_some() {
            self.notice = "已有下载任务，请等待完成。".into();
            return;
        }
        let pet = self.pet();
        match self.media.get(pet).cloned() {
            Some(MediaState::Ready(media))
                if matches!(
                    media.source,
                    assets::MediaSource::Repository | assets::MediaSource::Explicit
                ) =>
            {
                self.refresh_media(pet);
                self.notice = format!(
                    "{} 正在使用本地素材，无需下载：{}",
                    NAMES[self.selected],
                    media.path.display()
                );
                return;
            }
            Some(MediaState::Invalid(error)) => {
                self.notice = format!("素材路径不可用，未触发下载：{error}");
                return;
            }
            _ => {}
        }
        let (sender, receiver) = mpsc::channel();
        let (progress_sender, progress) = mpsc::channel();
        std::thread::spawn(move || {
            let result = assets::download_with_progress_for(pet, progress_sender);
            let _ = sender.send(result);
        });
        self.progress = 0.0;
        self.download = Some(Download {
            pet,
            receiver,
            progress,
            received: 0,
            total: 0,
            verifying: false,
            start_after_download,
        });
        self.notice = if start_after_download {
            format!("{} 尚未下载；完成后会自动启动。", NAMES[self.selected])
        } else {
            format!("正在下载 {} 的原始 WebP…", NAMES[self.selected])
        };
    }

    fn toggle(&mut self) {
        let pet = self.pet();
        let result = if control::pet_running(pet) {
            control::stop_for(pet).map(|_| "已发送停止信号。".into())
        } else {
            match self.media.get(pet).cloned().unwrap_or(MediaState::Missing) {
                MediaState::Ready(media) => control::start_for_media(pet, &media.path),
                MediaState::Missing => {
                    self.download(true);
                    return;
                }
                MediaState::Invalid(error) => Err(error),
            }
        };
        self.notice = match result {
            Ok(message) => message,
            Err(error) => error,
        };
    }

    fn tick(&mut self) {
        let Some(download) = &mut self.download else {
            return;
        };
        while let Ok(progress) = download.progress.try_recv() {
            match progress {
                assets::Progress::Manifest => {}
                assets::Progress::Download { received, total } => {
                    download.received = received;
                    download.total = total;
                    self.progress = received as f64 / total.max(1) as f64;
                }
                assets::Progress::Verify => download.verifying = true,
            }
        }
        match download.receiver.try_recv() {
            Ok(Ok(path)) => {
                let pet = download.pet;
                let start_after_download = download.start_after_download;
                self.media.insert(
                    pet,
                    MediaState::Ready(assets::MediaLocation {
                        path: path.clone(),
                        source: assets::MediaSource::Managed,
                    }),
                );
                self.notice = if start_after_download {
                    match control::start_for_media(pet, &path) {
                        Ok(message) => {
                            format!("{pet} 下载完成：{}；{message}", path.display())
                        }
                        Err(error) => format!("{pet} 下载完成，但启动失败：{error}"),
                    }
                } else {
                    format!("{pet} 下载完成：{}", path.display())
                };
                self.progress = 1.0;
                self.download = None;
            }
            Ok(Err(error)) => {
                self.notice = format!("下载失败：{error}");
                self.download = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.notice = "下载线程意外退出，请重试。".into();
                self.download = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        if let Some(edit) = &mut self.editing_path {
            match key.code {
                KeyCode::Esc => self.editing_path = None,
                KeyCode::Enter => self.finish_path_edit(),
                KeyCode::Backspace => {
                    edit.text.pop();
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.editing_path = None;
                }
                KeyCode::Char(character)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    edit.text.push(character);
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Tab => {
                self.focus = if self.focus == Focus::Pets {
                    Focus::Settings
                } else {
                    Focus::Pets
                };
            }
            KeyCode::Up | KeyCode::Char('k') => match self.focus {
                Focus::Pets => self.selected = self.selected.saturating_sub(1),
                Focus::Settings => self.field = self.field.saturating_sub(1),
            },
            KeyCode::Down | KeyCode::Char('j') => match self.focus {
                Focus::Pets => self.selected = (self.selected + 1).min(character::PETS.len() - 1),
                Focus::Settings => self.field = (self.field + 1).min(5),
            },
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-')
                if self.focus == Focus::Settings =>
            {
                self.change(-1)
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=')
                if self.focus == Focus::Settings =>
            {
                self.change(1)
            }
            KeyCode::Enter | KeyCode::Char(' ') if self.focus == Focus::Settings => {
                match self.field {
                    2 => self.change(1),
                    4 if key.code == KeyCode::Enter => {
                        self.begin_path_edit(PathTarget::ManualMedia)
                    }
                    5 if key.code == KeyCode::Enter => self.begin_path_edit(PathTarget::AssetsRoot),
                    _ => {}
                }
            }
            KeyCode::Enter | KeyCode::Char('s') => self.toggle(),
            KeyCode::Char('d') => self.download(false),
            _ => {}
        }
        true
    }
}

struct TerminalGuard;
impl TerminalGuard {
    fn new() -> Result<Self, String> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err("TUI 需要交互终端；直接运行请使用 pet run <宠物>。".into());
        }
        crossterm::terminal::enable_raw_mode().map_err(|error| error.to_string())?;
        execute!(io::stdout(), EnterAlternateScreen, Hide).map_err(|error| error.to_string())?;
        Ok(Self)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

pub fn run() -> Result<(), String> {
    let _guard = TerminalGuard::new()?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|error| error.to_string())?;
    terminal.clear().map_err(|error| error.to_string())?;
    let mut app = App::new();
    let mut last_running_poll = Instant::now() - Duration::from_secs(2);
    let mut running = BTreeMap::new();
    loop {
        app.tick();
        if last_running_poll.elapsed() >= Duration::from_secs(1) {
            last_running_poll = Instant::now();
            running = character::PETS
                .iter()
                .map(|pet| (*pet, control::pet_running(pet)))
                .collect();
        }
        terminal
            .draw(|frame| draw(frame, &app, &running))
            .map_err(|error| error.to_string())?;
        if event::poll(Duration::from_millis(100)).map_err(|error| error.to_string())? {
            match event::read().map_err(|error| error.to_string())? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if !app.key(key) {
                        return Ok(());
                    }
                }
                Event::Resize(_, _) => terminal.clear().map_err(|error| error.to_string())?,
                _ => {}
            }
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &App, running: &BTreeMap<&str, bool>) {
    let area = centered(frame.area(), 94, 29);
    let theme = app.theme;
    let vertical = Layout::vertical([
        Constraint::Length(5),
        Constraint::Min(11),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    let title = Paragraph::new(vec![
        Line::from(Span::styled(
            "  ✦  PET",
            theme.fg(GOLD).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "    一位伙伴，一整个小宇宙。",
            theme.fg(theme::BLUE),
        )),
        Line::from(Span::styled(
            "    原生 Wayland  ·  原始画质  ·  按需下载",
            theme.dim(DIM),
        )),
    ]);
    frame.render_widget(title, vertical[0]);
    let body = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
        .spacing(1)
        .split(vertical[1]);
    draw_pets(frame, body[0], app, running);
    draw_settings(frame, body[1], app);
    let footer = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(vertical[2]);
    frame.render_widget(
        Paragraph::new(app.notice.as_str())
            .style(theme.fg(GREEN))
            .wrap(Wrap { trim: true }),
        footer[0],
    );
    if let Some(download) = &app.download {
        frame.render_widget(
            Gauge::default()
                .label(if download.verifying {
                    format!("{} · 正在逐文件校验", download.pet)
                } else if download.total > 0 {
                    format!(
                        "{} · {:.1}/{:.1} MiB",
                        download.pet,
                        download.received as f64 / 1048576.0,
                        download.total as f64 / 1048576.0
                    )
                } else {
                    format!("{} · 获取资源清单", download.pet)
                })
                .ratio(app.progress)
                .gauge_style(theme.fg(theme::BLUE)),
            footer[1],
        );
    }
    frame.render_widget(
        Paragraph::new(
            "↑↓ 选项   Tab 面板   ←→/-/+ 调整   Enter 编辑/确认   d 更新素材   s 启停   q 退出",
        )
        .alignment(Alignment::Center)
        .style(theme.dim(DIM)),
        vertical[3],
    );
}

fn draw_pets(frame: &mut ratatui::Frame, area: Rect, app: &App, running: &BTreeMap<&str, bool>) {
    let items = character::PETS.iter().enumerate().map(|(index, pet)| {
        let media = app.media.get(pet);
        let media_status: &str = if app
            .download
            .as_ref()
            .is_some_and(|download| download.pet == *pet)
        {
            "下载中"
        } else {
            match media {
                Some(MediaState::Ready(media)) => match media.source {
                    assets::MediaSource::Repository => "本地仓库",
                    assets::MediaSource::Explicit => "手动素材",
                    assets::MediaSource::Managed => "受管缓存",
                },
                Some(MediaState::Invalid(_)) => "路径错误",
                Some(MediaState::Missing) | None => "待下载",
            }
        };
        let is_running = running.get(pet).copied().unwrap_or(false);
        ListItem::new(Line::from(vec![
            Span::styled(
                format!(
                    " {} {:<8} {:<4}",
                    if is_running { "●" } else { "○" },
                    NAMES[index],
                    if is_running { "运行中" } else { "已停止" }
                ),
                app.theme.fg(COLORS[index]),
            ),
            Span::styled(
                media_status,
                app.theme.dim(match media {
                    Some(MediaState::Ready(_)) => GREEN,
                    Some(MediaState::Invalid(_)) => CORAL,
                    Some(MediaState::Missing) | None => DIM,
                }),
            ),
        ]))
    });
    let border = if app.focus == Focus::Pets {
        app.theme.fg(theme::BLUE)
    } else {
        app.theme.dim(DIM)
    };
    let mut state = ListState::default().with_selected(Some(app.selected));
    frame.render_stateful_widget(
        List::new(items.collect::<Vec<_>>())
            .block(
                Block::default()
                    .title(" 伙伴 ")
                    .borders(Borders::ALL)
                    .border_style(border),
            )
            .highlight_style(app.theme.selection().add_modifier(Modifier::BOLD))
            .highlight_symbol("❯ "),
        area,
        &mut state,
    );
}

fn draw_settings(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let border = if app.focus == Focus::Settings {
        app.theme.fg(theme::BLUE)
    } else {
        app.theme.dim(DIM)
    };
    let block = Block::default()
        .title(format!(" {} 的设置 ", NAMES[app.selected]))
        .borders(Borders::ALL)
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let values = app.values();
    let labels = [
        "大小",
        "动作缓存",
        "预乘 Alpha",
        "实际素材",
        "手动素材",
        "存储目录",
    ];
    let mut display = values
        .as_ref()
        .map(|values| {
            let actual = match app.media.get(app.pet()) {
                Some(MediaState::Ready(media)) => media.path.display().to_string(),
                Some(MediaState::Invalid(error)) => format!("不可用：{error}"),
                Some(MediaState::Missing) | None => "尚未准备（启动时下载）".into(),
            };
            [
                format!("[-]  {:.0}%  [+]", values.scale * 100.0),
                format!("[-]  {} MiB  [+]", values.cache_mib),
                if values.premultiply {
                    "[切换]  开启 · 推荐".into()
                } else {
                    "[切换]  关闭 · 保留原色".into()
                },
                actual,
                values
                    .media_dir
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "未指定（自动选择）".into()),
                values.assets_root.display().to_string(),
            ]
        })
        .unwrap_or_else(|_| {
            [
                "配置有误".into(),
                "请检查 JSON".into(),
                "—".into(),
                "—".into(),
                "—".into(),
                "—".into(),
            ]
        });
    if let Some(edit) = &app.editing_path {
        display[match edit.target {
            PathTarget::ManualMedia => 4,
            PathTarget::AssetsRoot => 5,
        }] = format!("> {}▏", edit.text);
    }
    let lines = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            let selected = app.focus == Focus::Settings && app.field == index;
            Line::from(vec![
                Span::styled(
                    format!("{} {:<10}", if selected { "❯" } else { " " }, label),
                    app.theme.fg(if selected { GOLD } else { theme::BLUE }),
                ),
                Span::styled(
                    &display[index],
                    app.theme.fg(if selected { CORAL } else { DIM }),
                ),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    #[test]
    fn names_match_the_runtime_registry() {
        assert_eq!(NAMES.len(), character::PETS.len());
        assert_eq!(COLORS.len(), character::PETS.len());
        assert!(character::PETS.iter().all(|name| name.ends_with("-girl")));
    }
    #[test]
    fn unknown_settings_are_not_silently_replaced() {
        let app = App::new();
        assert_eq!(app.selected, 0);
        assert_eq!(app.field, 0);
    }

    #[test]
    fn compact_screen_contains_all_controls_and_selected_values() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.selected = 7;
        app.focus = Focus::Settings;
        app.field = 1;
        app.media.insert(
            character::PETS[0],
            MediaState::Ready(assets::MediaLocation {
                path: PathBuf::from("/fork/linux-desktop-pet/claude-girl/media"),
                source: assets::MediaSource::Repository,
            }),
        );
        let running = BTreeMap::from([(character::PETS[0], true)]);
        terminal.draw(|frame| draw(frame, &app, &running)).unwrap();
        let buffer = terminal.backend().buffer();
        let content = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        let compact = content.split_whitespace().collect::<String>();
        for text in [
            "PET",
            "Claude",
            "Kimi",
            "动作缓存",
            "[-]",
            "[+]",
            "MiB",
            "实际素材",
            "手动素材",
            "存储目录",
            "Tab",
            "下载",
            "运行中",
            "已停止",
            "本地仓库",
        ] {
            let needle = text.split_whitespace().collect::<String>();
            assert!(compact.contains(&needle), "missing {text}\n{content}");
        }
        assert!(
            compact.contains("/home/tingfeng/code/self_project/")
                && compact.contains("kimi-girl/media"),
            "missing resolved media path\n{content}"
        );
    }

    #[test]
    fn interface_does_not_paint_a_full_panel_background() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let app = App::new();
        let running = BTreeMap::new();
        terminal.draw(|frame| draw(frame, &app, &running)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .all(|cell| cell.bg != ratatui::style::Color::Rgb(0x14, 0x15, 0x1c))
        );
    }

    #[test]
    fn update_action_keeps_usable_local_media_without_downloading() {
        let mut app = App::new();
        app.media.insert(
            character::PETS[0],
            MediaState::Ready(assets::MediaLocation {
                path: PathBuf::from("/fork/linux-desktop-pet/claude-girl/media"),
                source: assets::MediaSource::Repository,
            }),
        );
        app.download(false);
        assert!(app.download.is_none());
        assert!(app.notice.contains("无需下载"));
    }

    #[test]
    fn selected_pet_row_has_a_background_highlight() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new();
        app.theme = Theme::True;
        let running = BTreeMap::new();
        terminal.draw(|frame| draw(frame, &app, &running)).unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.bg == ratatui::style::Color::Rgb(0x24, 0x27, 0x33))
        );
    }
}
