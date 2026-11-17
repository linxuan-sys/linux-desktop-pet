mod theme;
mod usage;

use crate::{assets, character, control, settings};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Flex, Layout, Rect},
    style::Modifier,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    collections::BTreeSet,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};
use theme::{BLUE, CORAL, DIM, GOLD, GREEN, Theme};

const NAMES: &[&str] = &[
    "Claude", "DSH", "Codex", "Grok", "Qwen", "Gemini", "GLM", "Kimi",
];
const COLORS: &[(u8, u8, u8)] = &[
    (228, 191, 121),
    (174, 189, 232),
    (216, 201, 240),
    (227, 140, 154),
    (156, 207, 232),
    (201, 182, 228),
    (232, 166, 191),
    (174, 189, 232),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Page {
    Home,
    Actions,
    Settings,
    Advanced,
    Library,
}
#[derive(Clone, Copy, Debug)]
enum Action {
    Start,
    Stop,
    Update,
}
#[derive(Clone, Copy, PartialEq)]
enum Field {
    Scale,
    Cache,
    Alpha,
}
enum Editor {
    Number {
        field: Field,
        value: String,
    },
    Path(String),
    ConfirmPath {
        path: PathBuf,
        single: bool,
        count: usize,
    },
}
enum JobEvent {
    Begin(&'static str),
    Done(&'static str, Result<String, String>),
}
struct Job {
    events: Receiver<JobEvent>,
    progress: Receiver<assets::Progress>,
    pet: &'static str,
    finished: usize,
    errors: usize,
    total: usize,
    detail: String,
}
struct App {
    page: Page,
    cursor: usize,
    selected: BTreeSet<usize>,
    query: String,
    searching: bool,
    targets: Vec<usize>,
    menu: usize,
    editor: Option<Editor>,
    media: Vec<Result<Option<assets::MediaLocation>, String>>,
    monitor: usage::Monitor,
    job: Option<Job>,
    notice: String,
    theme: Theme,
}
impl App {
    fn new() -> Self {
        let mut app = Self {
            page: Page::Home,
            cursor: 0,
            selected: BTreeSet::new(),
            query: String::new(),
            searching: false,
            targets: vec![0],
            menu: 0,
            editor: None,
            media: Vec::new(),
            monitor: usage::Monitor::new(),
            job: None,
            notice: "Tab 勾选多位伙伴；Enter 选择操作。CPU 按整机总算力计算，100% 表示整机满载。"
                .into(),
            theme: Theme::detect(),
        };
        app.refresh_media();
        app.monitor.sample();
        app
    }
    fn refresh_media(&mut self) {
        self.media = character::PETS
            .iter()
            .map(|pet| assets::resolve_media_for(pet, None))
            .collect();
    }
    fn remember_media(&mut self) {
        for (pet, location) in character::PETS.iter().zip(&self.media) {
            if let Ok(Some(media)) = location
                && media.source == assets::MediaSource::Repository
                && let Err(error) = settings::remember_repository_media(pet, &media.path)
            {
                self.notice = format!("无法记住本地素材位置：{error}");
            }
        }
    }
    fn visible(&self) -> Vec<usize> {
        (0..NAMES.len())
            .filter(|index| fuzzy(&self.query, NAMES[*index]))
            .collect()
    }
    fn current(&self) -> Option<usize> {
        self.visible().get(self.cursor).copied()
    }
    fn capture_targets(&mut self) {
        self.targets = if self.selected.is_empty() {
            self.current().into_iter().collect()
        } else {
            self.selected.iter().copied().collect()
        };
    }
    fn open(&mut self, page: Page) {
        self.capture_targets();
        if self.targets.is_empty() {
            self.notice = "没有匹配的伙伴；Esc 清除搜索。".into();
            return;
        }
        self.page = page;
        self.menu = 0;
    }
    fn values(&self, index: usize) -> Result<settings::Settings, String> {
        let raw = match std::fs::read(settings::config_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(error.to_string()),
        };
        settings::Settings::parse_for(&raw, character::PETS[index])
    }
    fn setting_value(&self, field: Field) -> String {
        let values: Result<Vec<_>, _> = self
            .targets
            .iter()
            .map(|index| self.values(*index))
            .collect();
        let Ok(values) = values else {
            return "配置错误".into();
        };
        let texts: Vec<_> = values
            .iter()
            .map(|value| match field {
                Field::Scale => format!("{:.0}%", value.scale * 100.0),
                Field::Cache => format!("{} MiB", value.cache_mib),
                Field::Alpha => {
                    if value.premultiply {
                        "开启".into()
                    } else {
                        "关闭".into()
                    }
                }
            })
            .collect();
        if texts.windows(2).any(|pair| pair[0] != pair[1]) {
            "多个值".into()
        } else {
            texts.first().cloned().unwrap_or_default()
        }
    }
    fn save_value(&mut self, field: Field, value: f64) -> Result<(), String> {
        for index in &self.targets {
            self.values(*index)?;
        }
        let key = match field {
            Field::Scale => "scale",
            Field::Cache => "cache_mib",
            Field::Alpha => "premultiply",
        };
        let json = match field {
            Field::Scale => serde_json::json!(value / 100.0),
            Field::Cache => serde_json::json!(value as usize),
            Field::Alpha => serde_json::json!(value != 0.0),
        };
        let first = *self.targets.first().ok_or("尚未选择伙伴")?;
        settings::update_for(character::PETS[first], |raw| {
            for index in &self.targets {
                raw["pets"][character::PETS[*index]][key] = json.clone();
            }
        })?;
        self.notice = format!(
            "已保存，应用到 {} 位伙伴；运行中的伙伴会自动应用。",
            self.targets.len()
        );
        Ok(())
    }
    fn launch_job(&mut self, action: Action) {
        if self.job.is_some() {
            self.notice = "已有任务正在执行，请等待完成。".into();
            return;
        }
        let targets: Vec<_> = self
            .targets
            .iter()
            .map(|index| character::PETS[*index])
            .collect();
        let Some(&first) = targets.first() else {
            return;
        };
        let total = targets.len();
        let (sender, events) = mpsc::channel();
        let (progress_sender, progress) = mpsc::channel();
        std::thread::spawn(move || {
            let mut start_failed = false;
            for pet in targets {
                let _ = sender.send(JobEvent::Begin(pet));
                let result = if start_failed {
                    Err("前一个伙伴未成功就绪，已跳过后续启动；请检查后重试。".into())
                } else {
                    perform(action, pet, progress_sender.clone())
                };
                start_failed |= matches!(action, Action::Start) && result.is_err();
                if sender.send(JobEvent::Done(pet, result)).is_err() {
                    break;
                }
            }
        });
        self.job = Some(Job {
            events,
            progress,
            pet: first,
            finished: 0,
            errors: 0,
            total,
            detail: "正在准备…".into(),
        });
        self.page = Page::Home;
    }
    fn tick(&mut self) -> bool {
        let mut changed = false;
        let mut complete = false;
        let mut redraw = false;
        if let Some(job) = &mut self.job {
            let disconnected = loop {
                let event = match job.events.try_recv() {
                    Ok(event) => event,
                    Err(mpsc::TryRecvError::Empty) => break false,
                    Err(mpsc::TryRecvError::Disconnected) => break true,
                };
                redraw = true;
                match event {
                    JobEvent::Begin(pet) => {
                        job.pet = pet;
                        job.detail = "正在检查素材…".into();
                    }
                    JobEvent::Done(pet, result) => {
                        job.finished += 1;
                        match result {
                            Ok(message) => self.notice = format!("{pet}：{message}"),
                            Err(error) => {
                                job.errors += 1;
                                self.notice = format!("{pet}：{error}");
                            }
                        }
                        changed = true;
                    }
                }
            };
            while let Ok(progress) = job.progress.try_recv() {
                redraw = true;
                job.detail = match progress {
                    assets::Progress::Manifest => "检查远端清单…".into(),
                    assets::Progress::Verify => "下载完成，校验文件…".into(),
                    assets::Progress::Download { received, total } => {
                        let ratio = received as f64 / total.max(1) as f64;
                        let filled = (ratio * 16.0).clamp(0.0, 16.0) as usize;
                        format!(
                            "{}{} {:.0}%  {:.1}/{:.1} MiB",
                            "━".repeat(filled),
                            "─".repeat(16 - filled),
                            ratio * 100.0,
                            received as f64 / 1048576.0,
                            total as f64 / 1048576.0
                        )
                    }
                };
            }
            if job.finished == job.total {
                self.notice = format!(
                    "完成 {}/{}，失败 {}。{}",
                    job.finished, job.total, job.errors, self.notice
                );
                complete = true;
            } else if disconnected {
                self.notice = "任务线程意外退出；已有素材保持不变。".into();
                complete = true;
            }
        }
        if complete {
            self.job = None;
        }
        if changed {
            self.refresh_media();
            self.monitor.sample();
        }
        redraw || complete
    }
    fn edit_key(&mut self, key: KeyEvent) {
        let Some(editor) = self.editor.take() else {
            return;
        };
        if key.code == KeyCode::Esc {
            return;
        }
        match editor {
            Editor::Number { field, mut value } => {
                if key.code == KeyCode::Enter {
                    let bounds = if field == Field::Scale {
                        (50, 150)
                    } else {
                        (16, 512)
                    };
                    let result = value
                        .parse::<u32>()
                        .ok()
                        .filter(|v| (bounds.0..=bounds.1).contains(v));
                    if let Some(number) = result {
                        if let Err(error) = self.save_value(field, number as f64) {
                            self.notice = error;
                        }
                        return;
                    }
                    self.notice = format!("请输入 {}–{} 之间的整数。", bounds.0, bounds.1);
                } else {
                    match key.code {
                        KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Char('+')
                        | KeyCode::Char('-') => {
                            let step = if field == Field::Scale { 5 } else { 16 };
                            let max = if field == Field::Scale { 150 } else { 512 };
                            let min = if field == Field::Scale { 50 } else { 16 };
                            let delta = if matches!(key.code, KeyCode::Left | KeyCode::Char('-')) {
                                -step
                            } else {
                                step
                            };
                            value = (value.parse::<i32>().unwrap_or(min) + delta)
                                .clamp(min, max)
                                .to_string();
                        }
                        KeyCode::Backspace => {
                            value.pop();
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            value.clear()
                        }
                        KeyCode::Char(c) if c.is_ascii_digit() && value.len() < 4 => value.push(c),
                        _ => {}
                    }
                }
                self.editor = Some(Editor::Number { field, value });
            }
            Editor::Path(mut value) => {
                match key.code {
                    KeyCode::Enter => match settings::resolve_assets_root_input(&value) {
                        Ok(path) => {
                            let single = assets::usable_local_media(&path);
                            let count = if single {
                                1
                            } else {
                                character::PETS
                                    .iter()
                                    .filter(|pet| {
                                        assets::library_media(&path, pet).is_some()
                                            || managed_in(&path, pet)
                                    })
                                    .count()
                            };
                            if single && self.targets.len() != 1 {
                                self.notice = "单角色 media 目录需要先只选择一位伙伴。".into();
                            } else if count == 0
                                && std::fs::read_dir(&path)
                                    .is_ok_and(|mut entries| entries.next().is_some())
                            {
                                self.notice = "目录非空，但未识别到角色素材；请选择仓库根目录、素材库或空目录。".into();
                            } else {
                                self.editor = Some(Editor::ConfirmPath {
                                    path,
                                    single,
                                    count,
                                });
                                return;
                            }
                        }
                        Err(error) => self.notice = error,
                    },
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        value.clear()
                    }
                    KeyCode::Char(c)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        value.push(c)
                    }
                    _ => {}
                }
                self.editor = Some(Editor::Path(value));
            }
            Editor::ConfirmPath {
                path,
                single,
                count,
            } => {
                if key.code == KeyCode::Enter {
                    let result = if single {
                        settings::set_media_dir(
                            character::PETS[self.targets[0]],
                            &path.to_string_lossy(),
                            false,
                        )
                        .map(|_| ())
                    } else {
                        settings::set_assets_root(&path.to_string_lossy(), false).map(|_| ())
                    };
                    self.notice = match result {
                        Ok(()) => format!(
                            "素材目录已保存，识别到 {count} 位伙伴；运行中的伙伴重启后生效。"
                        ),
                        Err(error) => error,
                    };
                    self.refresh_media();
                    self.page = Page::Settings;
                    self.menu = 3;
                } else {
                    self.editor = Some(Editor::ConfirmPath {
                        path,
                        single,
                        count,
                    });
                }
            }
        }
    }
    fn key(&mut self, key: KeyEvent) -> bool {
        if self.editor.is_some() {
            self.edit_key(key);
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return false;
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.query.clear();
                    self.searching = false;
                    self.cursor = 0;
                }
                KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                    self.cursor = 0;
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.query.push(c);
                    self.cursor = 0;
                }
                _ => self.home_key(key),
            }
            return true;
        }
        if self.page == Page::Home {
            match key.code {
                KeyCode::Char('a' | 'd') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.home_key(key)
                }
                KeyCode::Char('q') => return false,
                KeyCode::Esc if !self.query.is_empty() => {
                    self.query.clear();
                    self.cursor = 0;
                }
                KeyCode::Esc if !self.selected.is_empty() => self.selected.clear(),
                KeyCode::Esc => return false,
                KeyCode::Char('/') => self.searching = true,
                KeyCode::Char('?') => self.notice = "Tab/Shift+Tab 勾选并移动；Ctrl+A 全选筛选结果，Ctrl+D 清空；CPU 以整机总算力为 100%；Enter 操作，, 设置，/ 搜索，Esc 返回。".into(),
                KeyCode::Char(',') => self.open(Page::Settings),
                KeyCode::Enter => self.open(Page::Actions),
                KeyCode::Char('s') | KeyCode::Char('d') => {
                    self.capture_targets();
                    self.launch_job(if key.code == KeyCode::Char('s') {
                        Action::Start
                    } else {
                        Action::Update
                    });
                }
                _ => self.home_key(key),
            }
        } else {
            let count = match self.page {
                Page::Actions => 5,
                Page::Settings => 6,
                Page::Advanced => 2,
                Page::Library => 3,
                Page::Home => 0,
            };
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.page = if matches!(self.page, Page::Library | Page::Advanced) {
                        Page::Settings
                    } else {
                        Page::Home
                    };
                    self.menu = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => self.menu = self.menu.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.menu = (self.menu + 1).min(count - 1),
                KeyCode::Enter => self.activate(),
                _ => {}
            }
        }
        true
    }
    fn home_key(&mut self, key: KeyEvent) {
        let visible = self.visible();
        let max = visible.len().saturating_sub(1);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(max),
            KeyCode::Tab | KeyCode::BackTab => {
                if let Some(index) = self.current()
                    && !self.selected.remove(&index)
                {
                    self.selected.insert(index);
                }
                self.cursor = if key.code == KeyCode::BackTab {
                    self.cursor.saturating_sub(1)
                } else {
                    (self.cursor + 1).min(max)
                };
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected.extend(visible)
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected.clear()
            }
            _ => {}
        }
    }
    fn activate(&mut self) {
        match (self.page, self.menu) {
            (Page::Actions, 0) => self.launch_job(Action::Start),
            (Page::Actions, 1) => self.launch_job(Action::Stop),
            (Page::Actions, 2) => {
                self.page = Page::Settings;
                self.menu = 0;
            }
            (Page::Actions, 3) | (Page::Settings, 4) => self.launch_job(Action::Update),
            (Page::Settings, 0..=1) => {
                let field = if self.menu == 0 {
                    Field::Scale
                } else {
                    Field::Cache
                };
                let value = self
                    .targets
                    .first()
                    .and_then(|index| self.values(*index).ok())
                    .map(|s| {
                        if field == Field::Scale {
                            (s.scale * 100.0).round() as usize
                        } else {
                            s.cache_mib
                        }
                    })
                    .unwrap_or(if field == Field::Scale { 100 } else { 32 });
                self.editor = Some(Editor::Number {
                    field,
                    value: value.to_string(),
                });
                self.notice = format!(
                    "编辑后应用到 {} 位伙伴；←→ 调整，Ctrl+U 清空后输入，Enter 保存，Esc 取消。",
                    self.targets.len()
                );
            }
            (Page::Settings, 2) => {
                self.page = Page::Advanced;
                self.menu = 0;
            }
            (Page::Settings, 3) => {
                if self.job.is_some() {
                    self.notice = "请等待当前任务完成后修改素材目录。".into();
                    return;
                }
                self.page = Page::Library;
                self.menu = 0;
            }
            (Page::Advanced, 0) => {
                let enabled = self.setting_value(Field::Alpha) != "开启";
                if let Err(error) = self.save_value(Field::Alpha, if enabled { 1.0 } else { 0.0 }) {
                    self.notice = error;
                }
            }
            (Page::Library, 0) => {
                self.notice = match settings::set_assets_root("", true) {
                    Ok(_) => "已使用自动管理；本地素材优先，缺失时按需下载。".into(),
                    Err(e) => e,
                };
                self.refresh_media();
                self.page = Page::Settings;
                self.menu = 3;
            }
            (Page::Library, 1) => {
                self.editor = Some(Editor::Path(String::new()));
                self.notice =
                    "输入素材库、仓库根目录、单角色 media 或空目录；Enter 预览，Esc 取消。".into();
            }
            _ => {
                self.page = if matches!(self.page, Page::Library | Page::Advanced) {
                    Page::Settings
                } else {
                    Page::Home
                };
                self.menu = 0;
            }
        }
    }
}

fn managed_in(root: &Path, pet: &str) -> bool {
    std::fs::read_dir(root.join(pet).join("assets")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join(".ready.json").is_file())
    })
}
fn perform(
    action: Action,
    pet: &'static str,
    progress: assets::ProgressSender,
) -> Result<String, String> {
    match action {
        Action::Stop => {
            if !control::pet_running(pet) {
                return Ok("已停止，跳过。".into());
            }
            control::stop_for(pet)?;
            Ok("已发送停止信号。".into())
        }
        Action::Start => {
            if control::pet_running(pet) {
                return Ok("已运行，跳过。".into());
            }
            let media = match assets::resolve_media_for(pet, None)? {
                Some(media) => {
                    if media.source == assets::MediaSource::Repository {
                        settings::remember_repository_media(pet, &media.path)?;
                    }
                    media.path
                }
                None => assets::download_with_progress_for(pet, progress)?,
            };
            let result = control::start_for_media(pet, &media)?;
            // Serialize expensive first-clip decoding; also detect a failed launch.
            for _ in 0..100 {
                std::thread::sleep(Duration::from_millis(100));
                if !control::pet_running(pet) {
                    return Err("启动后退出；请查看 ~/.local/state/pet 下的日志。".into());
                }
                if control::host_ready(pet) {
                    return Ok(result);
                }
            }
            Err("启动尚未就绪；请查看进程状态和日志后重试。".into())
        }
        Action::Update => {
            if let Some(media) = assets::resolve_media_for(pet, None)?
                && media.source != assets::MediaSource::Managed
            {
                return Ok("正在使用本地素材；通过更新仓库或替换文件更新，无需下载。".into());
            }
            if control::pet_running(pet) {
                return Err("请先停止此伙伴再更新素材，避免运行中混用新旧文件。".into());
            }
            assets::download_with_progress_for(pet, progress)?;
            Ok("已检查，素材已是最新。".into())
        }
    }
}
fn fuzzy(query: &str, value: &str) -> bool {
    let value = value.to_lowercase();
    let mut chars = value.chars();
    query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .all(|needle| chars.by_ref().any(|c| c == needle))
}
fn short_path(path: &Path, width: usize) -> String {
    let mut text = path.display().to_string();
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(Path::new(&home))
    {
        text = format!("~/{}", rest.display());
    }
    let chars: Vec<_> = text.chars().collect();
    // A Chinese character may take two terminal cells.
    let width = width.max(5);
    let cells = |s: &[char]| {
        s.iter()
            .map(|c| if c.is_ascii() { 1 } else { 2 })
            .sum::<usize>()
    };
    if cells(&chars) <= width {
        return text;
    }
    let mut left = 0;
    let mut right = chars.len();
    while left < chars.len() && cells(&chars[..left + 1]) <= width / 3 {
        left += 1;
    }
    while right > left && cells(&chars[right - 1..]) < width - width / 3 {
        right -= 1;
    }
    format!(
        "{}…{}",
        chars[..left].iter().collect::<String>(),
        chars[right..].iter().collect::<String>()
    )
}
fn source_label(media: &Result<Option<assets::MediaLocation>, String>) -> &'static str {
    match media {
        Ok(Some(_)) => "素材就绪",
        Ok(None) => "未下载",
        Err(_) => "路径错误",
    }
}
fn draw(frame: &mut ratatui::Frame, app: &App) {
    let [area] = Layout::horizontal([Constraint::Length(96.min(frame.area().width))])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Layout::vertical([Constraint::Length(28.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let area = Rect {
        x: area.x + 2,
        y: area.y + 1,
        width: area.width.saturating_sub(4),
        height: area.height.saturating_sub(2),
    };
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    let title = match app.page {
        Page::Home => "PET",
        Page::Actions => "PET / 操作",
        Page::Settings => "PET / 设置",
        Page::Advanced => "PET / 高级设置",
        Page::Library => "PET / 素材目录",
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                title,
                app.theme.fg(GOLD).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled("一位伙伴，一整个小宇宙。", app.theme.fg(BLUE))),
        ]),
        sections[0],
    );
    if app.editor.is_some() {
        draw_editor(frame, sections[1], app);
    } else if app.page == Page::Home {
        draw_home(frame, sections[1], app);
    } else {
        draw_menu(frame, sections[1], app);
    }
    let mut notices = vec![Line::from(Span::styled(&app.notice, app.theme.fg(GREEN)))];
    if let Some(job) = &app.job {
        notices.push(Line::from(Span::styled(
            format!(
                "{}/{} · {} · {}",
                job.finished, job.total, job.pet, job.detail
            ),
            app.theme.fg(BLUE),
        )));
    }
    frame.render_widget(
        Paragraph::new(notices).wrap(Wrap { trim: true }),
        sections[2],
    );
    let help = if app.editor.is_some() {
        "Enter 保存/确认   Esc 取消   Ctrl+U 清空输入"
    } else if app.page == Page::Home {
        "↑↓/jk 移动  Tab 多选  Enter 操作  , 设置  / 搜索  q 退出"
    } else {
        "↑↓/jk 移动   Enter 选择   Esc 返回"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from("─".repeat(area.width as usize)),
            Line::from(help),
        ])
        .style(app.theme.dim(DIM)),
        sections[3],
    );
}
fn draw_home(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "伙伴     已选 {} / 8     / {}{}",
                app.selected.len(),
                app.query,
                if app.searching { "▏" } else { "" }
            )),
            Line::from("      名称        状态    素材      CPU（整机）    内存"),
        ])
        .style(app.theme.dim(DIM)),
        regions[0],
    );
    let visible = app.visible();
    if visible.is_empty() {
        frame.render_widget(Paragraph::new("没有匹配的伙伴；Esc 清除搜索。"), regions[1]);
    } else {
        let items: Vec<_> = visible
            .iter()
            .map(|index| {
                let usage = app
                    .monitor
                    .current
                    .get(character::PETS[*index])
                    .copied()
                    .unwrap_or_default();
                let cpu = usage
                    .cpu
                    .map(|value| format!("{value:.2}%"))
                    .unwrap_or_else(|| "—".into());
                let rss = usage
                    .rss
                    .map(|value| format!("{:.1} MiB", value as f64 / 1048576.0))
                    .unwrap_or_else(|| "—".into());
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(
                            "[{}] {:<9}",
                            if app.selected.contains(index) {
                                "✓"
                            } else {
                                " "
                            },
                            NAMES[*index]
                        ),
                        app.theme.fg(COLORS[*index]),
                    ),
                    Span::styled(
                        format!(
                            " {} {}  ",
                            if usage.pid.is_some() { "●" } else { "○" },
                            if usage.pid.is_some() {
                                "运行中"
                            } else {
                                "已停止"
                            }
                        ),
                        app.theme.fg(if usage.pid.is_some() { GREEN } else { DIM }),
                    ),
                    Span::styled(
                        format!("{}  {cpu:>7}  {rss:>10}", source_label(&app.media[*index])),
                        app.theme.fg(if app.media[*index].is_err() {
                            CORAL
                        } else {
                            DIM
                        }),
                    ),
                ]))
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("› ")
                .highlight_style(app.theme.selection().add_modifier(Modifier::BOLD)),
            regions[1],
            &mut ListState::default().with_selected(Some(app.cursor)),
        );
    }
    let mut detail = Vec::new();
    if let Some(index) = app.current() {
        let value = app
            .values(index)
            .map(|s| format!("大小 {:.0}% · 缓存 {} MiB", s.scale * 100.0, s.cache_mib))
            .unwrap_or_else(|error| error);
        detail.push(Line::from(format!("{} · {value}", NAMES[index])));
        let actual = control::running_media(character::PETS[index]).or_else(|| {
            app.media[index]
                .as_ref()
                .ok()
                .and_then(|m| m.as_ref())
                .map(|m| m.path.clone())
        });
        detail.push(Line::from(match actual {
            Some(path) => format!(
                "素材：{}",
                short_path(&path, regions[2].width.saturating_sub(6) as usize)
            ),
            None => match &app.media[index] {
                Err(error) => error.clone(),
                _ => "素材：尚未准备，启动时按需下载".into(),
            },
        }));
    }
    let cpu = app
        .monitor
        .current
        .values()
        .filter_map(|v| v.cpu)
        .fold(0.0, |total, value| total + value);
    let memory: u64 = app.monitor.current.values().filter_map(|v| v.rss).sum();
    let running = app
        .monitor
        .current
        .values()
        .filter(|v| v.pid.is_some())
        .count();
    detail.push(Line::from(format!(
        "总计 {running} 位运行 · CPU（整机） {cpu:.2}% · 内存 {:.1} MiB · 每秒刷新",
        memory as f64 / 1048576.0
    )));
    frame.render_widget(Paragraph::new(detail).style(app.theme.dim(DIM)), regions[2]);
}
fn draw_menu(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let names = app
        .targets
        .iter()
        .map(|i| NAMES[*i])
        .collect::<Vec<_>>()
        .join("、");
    let running = app
        .targets
        .iter()
        .filter(|i| {
            app.monitor
                .current
                .get(character::PETS[**i])
                .is_some_and(|v| v.pid.is_some())
        })
        .count();
    let mut root = settings::custom_assets_root()
        .map(|root| {
            root.map(|p| short_path(&p, 40))
                .unwrap_or_else(|| "自动管理".into())
        })
        .unwrap_or_else(|e| e);
    let overrides: Vec<_> = app
        .targets
        .iter()
        .filter_map(|index| app.values(*index).ok().and_then(|values| values.media_dir))
        .collect();
    if app.targets.len() == 1
        && let Some(path) = overrides.first()
    {
        root = short_path(path, 40);
    } else if !overrides.is_empty() {
        root = "按角色设置".into();
    }
    let rows: Vec<(String, String)> = match app.page {
        Page::Actions => vec![
            (
                "启动".into(),
                format!(
                    "启动 {} 位，{running} 位已运行",
                    app.targets.len() - running
                ),
            ),
            ("停止".into(), format!("停止 {running} 位")),
            ("调整设置".into(), "应用到所选伙伴".into()),
            ("检查素材更新".into(), "本地素材直接复用".into()),
            ("返回".into(), String::new()),
        ],
        Page::Settings => vec![
            ("大小".into(), app.setting_value(Field::Scale)),
            ("动作缓存".into(), app.setting_value(Field::Cache)),
            ("高级设置".into(), "›".into()),
            ("素材目录".into(), root),
            ("检查素材更新".into(), "›".into()),
            ("返回".into(), String::new()),
        ],
        Page::Advanced => vec![
            ("预乘 Alpha".into(), app.setting_value(Field::Alpha)),
            ("返回".into(), String::new()),
        ],
        Page::Library => vec![
            ("自动管理".into(), "本地素材优先，缺失时下载".into()),
            ("选择目录…".into(), "素材库 / 仓库 / media / 空目录".into()),
            ("返回".into(), String::new()),
        ],
        Page::Home => Vec::new(),
    };
    let regions = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!("所选伙伴 · {names}"))
            .style(app.theme.fg(BLUE))
            .wrap(Wrap { trim: true }),
        regions[0],
    );
    let items = rows
        .into_iter()
        .map(|(label, value)| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(
                        "{label}{}",
                        " ".repeat(
                            24usize.saturating_sub(
                                label
                                    .chars()
                                    .map(|c| if c.is_ascii() { 1 } else { 2 })
                                    .sum()
                            )
                        )
                    ),
                    app.theme.fg(BLUE),
                ),
                Span::styled(value, app.theme.dim(DIM)),
            ]))
        })
        .collect::<Vec<_>>();
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("› ")
            .highlight_style(app.theme.selection()),
        regions[1],
        &mut ListState::default().with_selected(Some(app.menu)),
    );
    frame.render_widget(Paragraph::new(if app.page==Page::Library {"素材库目录是全局设置；单角色 media 只关联所选伙伴。\n已有文件不复制、不删除；选择目录后先预览，再确认。"}else{"选择项目并按 Enter 修改；设置会应用到所选伙伴。"}).style(app.theme.dim(DIM)).wrap(Wrap{trim:true}),regions[2]);
}
fn draw_editor(frame: &mut ratatui::Frame, area: Rect, app: &App) {
    let Some(editor) = &app.editor else {
        return;
    };
    let lines = match editor {
        Editor::Number { field, value } => vec![
            format!(
                "{} · {} 位伙伴",
                if *field == Field::Scale {
                    "大小"
                } else {
                    "动作缓存"
                },
                app.targets.len()
            ),
            String::new(),
            format!(
                "    −     {value}▏ {}     +",
                if *field == Field::Scale { "%" } else { "MiB" }
            ),
            String::new(),
            "←→ 调整，或 Ctrl+U 清空后输入；Enter 保存。".into(),
        ],
        Editor::Path(value) => vec![
            "选择素材目录".into(),
            String::new(),
            format!("> {value}▏"),
            String::new(),
            "支持 ~/、仓库根目录、素材库或单角色 media。".into(),
            "空目录将用作下载位置，已有文件不会移动。".into(),
        ],
        Editor::ConfirmPath {
            path,
            single,
            count,
        } => vec![
            format!("已发现 {count} 位伙伴的素材"),
            String::new(),
            path.display().to_string(),
            String::new(),
            if *single {
                format!("仅关联 {}", NAMES[app.targets[0]])
            } else {
                "应用到全部伙伴；缺失的角色在此目录按需下载。".into()
            },
            String::new(),
            "› Enter 使用此目录    Esc 返回".into(),
        ],
    };
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>())
            .style(app.theme.fg(BLUE))
            .wrap(Wrap { trim: true }),
        area,
    );
}
struct TerminalGuard;
impl TerminalGuard {
    fn new() -> Result<Self, String> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err("TUI 需要交互终端；请使用 pet tui。".into());
        }
        crossterm::terminal::enable_raw_mode().map_err(|e| e.to_string())?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, Hide).map_err(|e| e.to_string())?;
        Ok(guard)
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
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|e| e.to_string())?;
    terminal.clear().map_err(|e| e.to_string())?;
    let mut app = App::new();
    app.remember_media();
    let mut poll = Instant::now();
    let mut redraw = true;
    loop {
        redraw |= app.tick();
        if poll.elapsed() >= Duration::from_secs(1) {
            app.monitor.sample();
            poll = Instant::now();
            redraw = true;
        }
        if redraw {
            terminal
                .draw(|frame| draw(frame, &app))
                .map_err(|e| e.to_string())?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    redraw = true;
                    if !app.key(key) {
                        return Ok(());
                    }
                }
                Event::Resize(_, _) => {
                    terminal.clear().map_err(|e| e.to_string())?;
                    redraw = true;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        text.split_whitespace().collect()
    }
    #[test]
    fn tab_selection_survives_filtering_and_targets_selected_pets() {
        let mut app = App::new();
        app.key(key(KeyCode::Tab));
        app.key(key(KeyCode::Tab));
        assert_eq!(app.selected, BTreeSet::from([0, 1]));
        assert_eq!(app.cursor, 2);
        app.key(key(KeyCode::Char('/')));
        for c in "cdx".chars() {
            app.key(key(KeyCode::Char(c)));
        }
        assert_eq!(app.visible(), vec![2]);
        app.key(key(KeyCode::Tab));
        app.key(key(KeyCode::Enter)); // Leave search.
        app.key(key(KeyCode::Enter)); // Open actions for checked items.
        assert_eq!(app.page, Page::Actions);
        assert_eq!(app.targets, vec![0, 1, 2]);
        app.key(key(KeyCode::Esc));
        app.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        assert!(app.selected.is_empty());
        assert!(app.job.is_none()); // Ctrl+D must never start a download.
        app.open(Page::Settings);
        assert_eq!(app.targets, vec![2]);
    }
    #[test]
    fn select_all_is_scoped_to_results_and_no_match_has_no_action_target() {
        let mut app = App::new();
        app.query = "g".into();
        let expected = BTreeSet::from([3, 5, 6]);
        app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(app.selected, expected);
        app.selected.clear();
        app.query = "no-such-pet".into();
        app.open(Page::Actions);
        assert_eq!(app.page, Page::Home);
        assert!(app.targets.is_empty());
    }
    #[test]
    fn compact_home_displays_live_usage_and_single_directory_setting() {
        let mut app = App::new();
        app.theme = Theme::True;
        app.monitor.current.insert(
            character::PETS[0],
            usage::Usage {
                pid: Some(123),
                cpu: Some(12.5),
                rss: Some(64 * 1048576),
            },
        );
        let text = screen(&app, 80, 24);
        for expected in [
            "Claude",
            "Kimi",
            "12.50%",
            "64.0 MiB",
            "每秒刷新",
            "CPU",
            "Tab",
        ] {
            assert!(
                text.contains(&expected.split_whitespace().collect::<String>()),
                "missing {expected}: {text}"
            );
        }
        app.open(Page::Settings);
        let text = screen(&app, 80, 24);
        assert!(text.contains("素材目录"));
        assert!(!text.contains("手动素材"));
        assert!(!text.contains("存储目录"));
    }
    #[test]
    fn cancelled_numeric_edit_never_saves() {
        let mut app = App::new();
        app.open(Page::Settings);
        app.activate();
        assert!(matches!(
            app.editor,
            Some(Editor::Number {
                field: Field::Scale,
                ..
            })
        ));
        app.key(key(KeyCode::Right));
        app.key(key(KeyCode::Esc));
        assert!(app.editor.is_none());
        assert_eq!(app.page, Page::Settings);
    }
    #[test]
    fn finished_queue_retains_error_feedback_and_returns_to_idle() {
        let mut app = App::new();
        let (sender, events) = mpsc::channel();
        let (_progress_sender, progress) = mpsc::channel();
        app.job = Some(Job {
            events,
            progress,
            pet: character::PETS[0],
            finished: 0,
            errors: 0,
            total: 2,
            detail: String::new(),
        });
        sender.send(JobEvent::Begin(character::PETS[0])).unwrap();
        sender
            .send(JobEvent::Done(character::PETS[0], Ok("跳过".into())))
            .unwrap();
        sender
            .send(JobEvent::Done(character::PETS[1], Err("缺少素材".into())))
            .unwrap();
        drop(sender);
        app.tick();
        assert!(app.job.is_none());
        assert!(app.notice.contains("失败 1"));
        assert!(app.notice.contains("缺少素材"));
    }
    #[test]
    fn current_row_has_background_but_panels_are_transparent() {
        let mut app = App::new();
        app.theme = Theme::True;
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let cells = terminal.backend().buffer().content();
        assert!(
            cells
                .iter()
                .any(|cell| cell.bg == ratatui::style::Color::Rgb(0x24, 0x27, 0x33))
        );
        assert!(
            cells
                .iter()
                .filter(|cell| cell.bg == ratatui::style::Color::Reset)
                .count()
                > cells.len() / 2
        );
    }
}
