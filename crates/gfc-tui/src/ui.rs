use std::io::{self};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use gfc_config::{Config, LaunchTarget, Paths};
use gfc_daemon::rpc::call;
use gfc_daemon::{is_running, launch_repo};
use gfc_git::scan_inventory;
use gfc_schema::{CompactLocal, CompactRemote, Inventory, RepositoryHealth};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use serde_json::json;

use crate::cockpit::HealthBar;
use crate::filter::{SortKey, compact_local_label, compact_remote_label, visible_repos};
use crate::identity::{GithubIdentity, fetch_github_identity, load_avatar_image};
use crate::readme::{ReadmePreview, load_readme_preview};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Settings,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneFocus {
    List,
    Readme,
}

impl PaneFocus {
    fn toggle(self) -> Self {
        match self {
            Self::List => Self::Readme,
            Self::Readme => Self::List,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsField {
    Roots,
    Concurrency,
    LocalInterval,
    RemoteInterval,
    DirtyAttention,
    FetchStale,
    UnfinishedStale,
    LocalCurrent,
    RemoteCurrent,
    Editor,
    Terminal,
    Browser,
    Lazygit,
    DefaultLaunch,
    WebhookEnabled,
    WebhookBind,
    GithubEnabled,
    GitlabEnabled,
    OriginEnabled,
    OriginApp,
    Summarizer,
}

impl SettingsField {
    const ALL: [SettingsField; 21] = [
        Self::Roots,
        Self::Concurrency,
        Self::LocalInterval,
        Self::RemoteInterval,
        Self::DirtyAttention,
        Self::FetchStale,
        Self::UnfinishedStale,
        Self::LocalCurrent,
        Self::RemoteCurrent,
        Self::Editor,
        Self::Terminal,
        Self::Browser,
        Self::Lazygit,
        Self::DefaultLaunch,
        Self::WebhookEnabled,
        Self::WebhookBind,
        Self::GithubEnabled,
        Self::GitlabEnabled,
        Self::OriginEnabled,
        Self::OriginApp,
        Self::Summarizer,
    ];

    fn next(self) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(i + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

struct App {
    paths: Paths,
    config: Config,
    inventory: Inventory,
    selected: usize,
    filter: String,
    filter_mode: bool,
    attention_only: bool,
    sort: SortKey,
    screen: Screen,
    focus: PaneFocus,
    readme: ReadmePreview,
    readme_repo: Option<String>,
    readme_scroll: u16,
    settings_field: SettingsField,
    edit: Option<String>,
    status: String,
    last_network: String,
    last_auth: String,
    github: Option<GithubIdentity>,
    picker: Picker,
    avatar: Option<StatefulProtocol>,
    avatar_login: Option<String>,
    opened_at: Instant,
    socket: PathBuf,
}

pub fn run() -> io::Result<()> {
    let paths = Paths::resolve().map_err(|e| io::Error::other(e.to_string()))?;
    paths
        .ensure_dirs()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let config =
        Config::load_or_default(&paths.config_file).map_err(|e| io::Error::other(e.to_string()))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let inventory = load_inventory(&paths, &config, &rt);
    let mut app = App {
        socket: paths.socket_file.clone(),
        paths,
        config,
        inventory,
        selected: 0,
        filter: String::new(),
        filter_mode: false,
        attention_only: true,
        sort: SortKey::Attention,
        screen: Screen::Main,
        focus: PaneFocus::List,
        readme: ReadmePreview::missing(),
        readme_repo: None,
        readme_scroll: 0,
        settings_field: SettingsField::Roots,
        edit: None,
        status: "ready".into(),
        last_network: "none".into(),
        last_auth: "none".into(),
        github: None,
        picker: fallback_picker(),
        avatar: None,
        avatar_login: None,
        opened_at: Instant::now(),
    };

    let mut terminal = ratatui::init();
    if let Ok(picker) = Picker::from_query_stdio() {
        app.picker = picker;
    }
    let result = app_loop(&mut terminal, &mut app, &rt);
    ratatui::restore();
    result
}

fn fallback_picker() -> Picker {
    Picker::from_fontsize((10, 20))
}

fn load_inventory(paths: &Paths, config: &Config, rt: &tokio::runtime::Runtime) -> Inventory {
    if is_running(&paths.socket_file) {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&paths.socket_file) {
            if let Ok(value) = call(&mut stream, "inventory.snapshot", json!({})) {
                if let Ok(inv) = serde_json::from_value(value) {
                    return inv;
                }
            }
        }
    }
    rt.block_on(scan_inventory(config, &[]))
}

fn app_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rt: &tokio::runtime::Runtime,
) -> io::Result<()> {
    let mut last_poll = Instant::now();
    loop {
        terminal.draw(|f| draw(f, app))?;
        if last_poll.elapsed() > Duration::from_millis(750) {
            refresh(app, rt);
            last_poll = Instant::now();
        }
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if handle_key(app, key, rt)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn refresh(app: &mut App, rt: &tokio::runtime::Runtime) {
    if is_running(&app.socket) {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&app.socket) {
            if let Ok(value) = call(&mut stream, "inventory.snapshot", json!({})) {
                if let Ok(inv) = serde_json::from_value(value) {
                    app.inventory = inv;
                }
            }
            if let Ok(value) = call(&mut stream, "auth.status", json!({})) {
                app.last_network = value
                    .get("last_network")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none")
                    .to_string();
                if let Some(arr) = value.get("credentials").and_then(|v| v.as_array()) {
                    app.last_auth = arr
                        .iter()
                        .filter_map(|v| v.get("label").and_then(|x| x.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if app.last_auth.is_empty() {
                        app.last_auth = "none".into();
                    }
                }
                app.github = GithubIdentity::from_auth_status(&value);
            }
            return;
        }
    }
    app.inventory = rt.block_on(scan_inventory(&app.config, &[]));
    if app.github.is_none() {
        app.github = rt.block_on(fetch_github_identity(&app.config, &app.paths));
    }
}

fn rows(app: &App) -> Vec<&RepositoryHealth> {
    visible_repos(
        &app.inventory,
        &app.config.health,
        &app.filter,
        app.attention_only,
        app.sort,
    )
}

fn handle_key(app: &mut App, key: KeyEvent, rt: &tokio::runtime::Runtime) -> io::Result<bool> {
    if let Some(buf) = app.edit.as_mut() {
        match key.code {
            KeyCode::Esc => app.edit = None,
            KeyCode::Enter => {
                let value = buf.clone();
                app.edit = None;
                apply_edit(app, &value);
            }
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            _ => {}
        }
        return Ok(false);
    }
    if app.filter_mode {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.filter_mode = false,
            KeyCode::Backspace => {
                app.filter.pop();
            }
            KeyCode::Char(c) => app.filter.push(c),
            _ => {}
        }
        return Ok(false);
    }
    match app.screen {
        Screen::Help => {
            if matches!(key.code, KeyCode::Char('q' | '?') | KeyCode::Esc) {
                app.screen = Screen::Main;
            }
            return Ok(false);
        }
        Screen::Settings => return handle_settings_key(app, key),
        Screen::Main => {}
    }
    match key.code {
        KeyCode::Char('q') if key.modifiers.is_empty() => return Ok(true),
        KeyCode::Char('?') => app.screen = Screen::Help,
        KeyCode::Char('S') => app.screen = Screen::Settings,
        KeyCode::Char('/') => {
            app.filter_mode = true;
            app.filter.clear();
        }
        KeyCode::Char('s') => app.sort = app.sort.next(),
        KeyCode::Char('a') => app.attention_only = !app.attention_only,
        KeyCode::Tab | KeyCode::BackTab => {
            app.focus = app.focus.toggle();
        }
        KeyCode::Char('J') => scroll_readme(app, 1),
        KeyCode::Char('K') => scroll_readme(app, -1),
        KeyCode::PageDown => scroll_readme(app, 10),
        KeyCode::PageUp => scroll_readme(app, -10),
        KeyCode::Char('j') | KeyCode::Down => {
            if key.modifiers.contains(KeyModifiers::SHIFT) || app.focus == PaneFocus::Readme {
                scroll_readme(app, 1);
            } else {
                let n = rows(app).len();
                if n > 0 {
                    app.selected = (app.selected + 1).min(n - 1);
                }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if key.modifiers.contains(KeyModifiers::SHIFT) || app.focus == PaneFocus::Readme {
                scroll_readme(app, -1);
            } else {
                app.selected = app.selected.saturating_sub(1);
            }
        }
        KeyCode::Char('r') => {
            if let Some(path) = app.readme_repo.clone() {
                app.readme = load_readme_preview(Path::new(&path));
            }
            if is_running(&app.socket) {
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&app.socket) {
                    let _ = call(&mut stream, "scan.trigger", json!({}));
                }
            } else {
                refresh(app, rt);
            }
        }
        KeyCode::Enter => launch(app, "default"),
        KeyCode::Char('e') => launch(app, "editor"),
        KeyCode::Char('t') => launch(app, "terminal"),
        KeyCode::Char('b') => launch(app, "browser"),
        KeyCode::Char('g') => launch(app, "lazygit"),
        KeyCode::Char('c') => launch(app, "custom"),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            record_done(app);
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

fn handle_settings_key(app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Main,
        KeyCode::Char('j') | KeyCode::Down => app.settings_field = app.settings_field.next(),
        KeyCode::Char('k') | KeyCode::Up => app.settings_field = app.settings_field.prev(),
        KeyCode::Char(' ') => toggle_setting(app),
        KeyCode::Char('+') | KeyCode::Char('=') => bump_setting(app, 1),
        KeyCode::Char('-') => bump_setting(app, -1),
        KeyCode::Enter => {
            app.edit = Some(setting_value(app));
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            save_config(app);
        }
        _ => {}
    }
    Ok(false)
}

fn setting_value(app: &App) -> String {
    setting_value_for(app, app.settings_field)
}

fn apply_edit(app: &mut App, value: &str) {
    match app.settings_field {
        SettingsField::Roots => {
            app.config.roots = value
                .split(',')
                .map(|s| PathBuf::from(s.trim()))
                .filter(|p| !p.as_os_str().is_empty())
                .collect();
        }
        SettingsField::Concurrency => {
            if let Ok(n) = value.parse() {
                app.config.scan.concurrency = n;
            }
        }
        SettingsField::LocalInterval => {
            if let Ok(n) = value.parse() {
                app.config.poll.local_interval_ms = n;
            }
        }
        SettingsField::RemoteInterval => {
            if let Ok(n) = value.parse() {
                app.config.poll.remote_interval_secs = n;
            }
        }
        SettingsField::FetchStale => {
            if let Ok(n) = value.parse() {
                app.config.health.fetch_stale_after_secs = n;
            }
        }
        SettingsField::UnfinishedStale => {
            if let Ok(n) = value.parse() {
                app.config.health.unfinished_work_stale_after_secs = n;
            }
        }
        SettingsField::LocalCurrent => {
            if let Ok(n) = value.parse() {
                app.config.health.local_current_within_secs = n;
            }
        }
        SettingsField::RemoteCurrent => {
            if let Ok(n) = value.parse() {
                app.config.health.remote_current_within_secs = n;
            }
        }
        SettingsField::Editor => app.config.launch.editor = value.into(),
        SettingsField::Terminal => app.config.launch.terminal = value.into(),
        SettingsField::Browser => app.config.launch.browser = value.into(),
        SettingsField::Lazygit => app.config.launch.lazygit = value.into(),
        SettingsField::WebhookBind => app.config.webhook.bind = value.into(),
        SettingsField::DefaultLaunch => {
            app.config.launch.default_target = match value {
                "terminal" => LaunchTarget::Terminal,
                "browser" => LaunchTarget::Browser,
                "lazygit" => LaunchTarget::Lazygit,
                "custom" => LaunchTarget::Custom,
                _ => LaunchTarget::Editor,
            };
        }
        SettingsField::DirtyAttention
        | SettingsField::WebhookEnabled
        | SettingsField::GithubEnabled
        | SettingsField::GitlabEnabled
        | SettingsField::OriginEnabled
        | SettingsField::OriginApp
        | SettingsField::Summarizer => toggle_setting(app),
    }
    save_config(app);
}

fn toggle_setting(app: &mut App) {
    match app.settings_field {
        SettingsField::DirtyAttention => {
            app.config.health.dirty_needs_attention = !app.config.health.dirty_needs_attention;
        }
        SettingsField::WebhookEnabled => app.config.webhook.enabled = !app.config.webhook.enabled,
        SettingsField::GithubEnabled => {
            app.config.providers.github.enabled = !app.config.providers.github.enabled;
        }
        SettingsField::GitlabEnabled => {
            app.config.providers.gitlab.enabled = !app.config.providers.gitlab.enabled;
        }
        SettingsField::OriginEnabled => {
            app.config.providers.origin.enabled = !app.config.providers.origin.enabled;
        }
        SettingsField::OriginApp => {
            app.config.providers.origin.app_credentials_configured =
                !app.config.providers.origin.app_credentials_configured;
        }
        SettingsField::Summarizer => {
            app.config.plugins.summarizer_enabled = !app.config.plugins.summarizer_enabled;
        }
        _ => {}
    }
    save_config(app);
}

fn bump_setting(app: &mut App, delta: i64) {
    let bump = |v: &mut u64, step: u64| {
        if delta > 0 {
            *v = v.saturating_add(step);
        } else {
            *v = v.saturating_sub(step).max(1);
        }
    };
    match app.settings_field {
        SettingsField::Concurrency => {
            let mut n = app.config.scan.concurrency as u64;
            bump(&mut n, 1);
            app.config.scan.concurrency = n.max(1) as usize;
        }
        SettingsField::LocalInterval => bump(&mut app.config.poll.local_interval_ms, 100),
        SettingsField::RemoteInterval => bump(&mut app.config.poll.remote_interval_secs, 10),
        SettingsField::FetchStale => bump(&mut app.config.health.fetch_stale_after_secs, 60),
        SettingsField::UnfinishedStale => {
            bump(&mut app.config.health.unfinished_work_stale_after_secs, 60)
        }
        SettingsField::LocalCurrent => bump(&mut app.config.health.local_current_within_secs, 5),
        SettingsField::RemoteCurrent => bump(&mut app.config.health.remote_current_within_secs, 15),
        _ => {}
    }
    save_config(app);
}

fn save_config(app: &mut App) {
    match app.config.validate() {
        Ok(()) => {
            if let Err(err) = app.config.save(&app.paths.config_file) {
                app.status = format!("save failed: {err}");
                return;
            }
            if is_running(&app.socket) {
                if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&app.socket) {
                    if let Ok(text) = toml::to_string_pretty(&app.config) {
                        let _ = call(&mut stream, "config.set", json!({"toml": text}));
                    }
                }
            }
            app.status = format!("saved {}", app.paths.config_file.display());
        }
        Err(err) => app.status = format!("invalid: {err}"),
    }
}

fn launch(app: &mut App, target: &str) {
    let vis = rows(app);
    let Some(repo) = vis.get(app.selected) else {
        return;
    };
    let target = if target == "default" {
        match app.config.launch.default_target {
            LaunchTarget::Editor => "editor",
            LaunchTarget::Terminal => "terminal",
            LaunchTarget::Browser => "browser",
            LaunchTarget::Lazygit => "lazygit",
            LaunchTarget::Custom => "custom",
        }
    } else {
        target
    };
    let elapsed = app.opened_at.elapsed().as_millis();
    if is_running(&app.socket) {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&app.socket) {
            let _ = call(
                &mut stream,
                "repo.launch",
                json!({"path": repo.identity.path, "target": target}),
            );
            let _ = call(&mut stream, "metrics.local", json!({}));
        }
    } else if let Err(err) = launch_repo(&app.config.launch, target, repo) {
        app.status = format!("launch failed: {err}");
        return;
    }
    app.status = format!("opened {} in {elapsed}ms", repo.identity.name);
}

fn record_done(app: &mut App) {
    app.status = format!(
        "triage complete in {}ms",
        app.opened_at.elapsed().as_millis()
    );
}

fn scroll_readme(app: &mut App, delta: i32) {
    if delta >= 0 {
        app.readme_scroll = app.readme_scroll.saturating_add(delta as u16);
    } else {
        app.readme_scroll = app
            .readme_scroll
            .saturating_sub(delta.unsigned_abs() as u16);
    }
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    match app.screen {
        Screen::Help => draw_help(frame),
        Screen::Settings => draw_settings(frame, app),
        Screen::Main => draw_main(frame, app),
    }
}

fn draw_main(frame: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let vis = visible_repos(
        &app.inventory,
        &app.config.health,
        &app.filter,
        app.attention_only,
        app.sort,
    );
    if app.selected >= vis.len() {
        app.selected = vis.len().saturating_sub(1);
    }

    let list_items: Vec<ListItem> = vis
        .iter()
        .map(|r| {
            repo_list_item(
                r.identity.name.as_str(),
                r.needs_attention(&app.config.health),
            )
        })
        .collect();
    let selected_path = vis.get(app.selected).map(|r| r.identity.path.clone());
    let health = vis
        .get(app.selected)
        .map(|r| HealthBar::from_repo(r, &app.config.health));
    let visible_len = vis.len();
    let total_len = app.inventory.repositories.len();
    drop(vis);

    match selected_path {
        Some(path) => {
            if app.readme_repo.as_deref() != Some(path.as_str()) {
                app.readme = load_readme_preview(Path::new(&path));
                app.readme_repo = Some(path);
                app.readme_scroll = 0;
            }
        }
        None => {
            if app.readme_repo.take().is_some() {
                app.readme = ReadmePreview::missing();
                app.readme_scroll = 0;
            }
        }
    }

    sync_avatar(app);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(20)])
        .split(chunks[0]);
    frame.render_widget(
        Paragraph::new(header_line(app, visible_len)).wrap(Wrap { trim: true }),
        top[0],
    );
    draw_identity(frame, app, top[1]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(22), Constraint::Percentage(78)])
        .split(chunks[1]);
    draw_repo_list(frame, app, list_items, visible_len, total_len, body[0]);
    draw_cockpit(frame, app, health, body[1]);
    draw_status(frame, app, chunks[2]);
}

fn header_line(app: &App, visible: usize) -> String {
    let filter = if app.filter_mode {
        format!("{}_", app.filter)
    } else if app.filter.is_empty() {
        "off".into()
    } else {
        app.filter.clone()
    };
    format!(
        " gfc  sort={}  filter={}  attention={}  visible {}  {}/{} needing attention",
        app.sort.label(),
        filter,
        app.attention_only,
        visible,
        app.inventory.attention_count(&app.config.health),
        app.inventory.repositories.len()
    )
}

fn sync_avatar(app: &mut App) {
    let Some(id) = app.github.clone() else {
        app.avatar = None;
        app.avatar_login = None;
        return;
    };
    let unchanged = app.avatar_login.as_deref() == Some(id.login.as_str());
    if unchanged && (app.avatar.is_some() || id.avatar_path.is_none()) {
        return;
    }
    app.avatar = id
        .avatar_path
        .as_deref()
        .and_then(load_avatar_image)
        .map(|img| app.picker.new_resize_protocol(img));
    app.avatar_login = Some(id.login);
}

fn draw_identity(frame: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let github = app.github.clone();
    let block = pane_block("github", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match github {
        None => {
            frame.render_widget(
                Paragraph::new("not signed in").style(Style::new().fg(Color::DarkGray)),
                inner,
            );
        }
        Some(id) => {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(3), Constraint::Length(2)])
                .split(inner);
            if let Some(protocol) = app.avatar.as_mut() {
                frame.render_stateful_widget(
                    StatefulImage::new().resize(Resize::Fit(None)),
                    rows[0],
                    protocol,
                );
            }
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        id.display_name,
                        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        format!("@{}", id.login),
                        Style::new().fg(Color::DarkGray),
                    )),
                ]),
                rows[1],
            );
        }
    }
}

fn draw_repo_list(
    frame: &mut ratatui::Frame,
    app: &App,
    items: Vec<ListItem>,
    visible_len: usize,
    total_len: usize,
    area: Rect,
) {
    let mut state = ListState::default();
    if visible_len > 0 {
        state.select(Some(app.selected));
    }
    let title = format!("repos {visible_len}/{total_len}");
    let list = List::new(items)
        .block(pane_block(&title, app.focus == PaneFocus::List))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::REVERSED),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn repo_list_item(name: &str, attention: bool) -> ListItem<'static> {
    let mark = if attention { "!" } else { " " };
    let mark_style = if attention {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    ListItem::new(Line::from(vec![
        Span::styled(mark.to_string(), mark_style),
        Span::raw(" "),
        Span::raw(name.to_string()),
    ]))
}

fn draw_cockpit(frame: &mut ratatui::Frame, app: &mut App, health: Option<HealthBar>, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(8)])
        .split(area);

    let health_lines = match health {
        Some(bar) => health_bar_lines(&bar),
        None => vec![Line::from("no repository selected")],
    };
    frame.render_widget(
        Paragraph::new(health_lines).block(pane_block("repo", false)),
        panes[0],
    );

    let inner_h = panes[1].height.saturating_sub(2);
    let line_count = app.readme.text.lines().count() as u16;
    let max_scroll = line_count.saturating_sub(inner_h.max(1));
    app.readme_scroll = app.readme_scroll.min(max_scroll);
    let title = app.readme.source.clone().unwrap_or_else(|| "README".into());
    frame.render_widget(
        Paragraph::new(app.readme.text.as_str())
            .wrap(Wrap { trim: false })
            .scroll((app.readme_scroll, 0))
            .block(pane_block(&title, app.focus == PaneFocus::Readme)),
        panes[1],
    );
}

fn draw_status(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let hints = if app.filter_mode {
        format!(" filter: {}_   Esc/Enter apply", app.filter)
    } else {
        match app.focus {
            PaneFocus::List => {
                " j/k list  Tab README  J/K·PgUp/PgDn README  / filter  a attention  s sort  e/t/b/g/c launch  S settings  ? help  q quit"
                    .into()
            }
            PaneFocus::Readme => {
                " j/k README  Tab list  J/K·PgUp/PgDn README  / filter  a attention  s sort  e/t/b/g/c launch  S settings  ? help  q quit"
                    .into()
            }
        }
    };
    let status = format!(
        " {}  net:{}  auth:{}  errors:{}",
        app.status,
        app.last_network,
        app.last_auth,
        app.inventory.errors.len()
    );
    frame.render_widget(
        Paragraph::new(vec![Line::from(hints), Line::from(status)]),
        area,
    );
}

fn health_bar_lines(bar: &HealthBar) -> Vec<Line<'static>> {
    let attn_style = if bar.attention {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Green)
    };
    let local_style = match bar.local {
        CompactLocal::Clean => Style::new().fg(Color::Green),
        CompactLocal::Dirty
        | CompactLocal::Ahead
        | CompactLocal::Behind
        | CompactLocal::Diverged => Style::new().fg(Color::Yellow),
        CompactLocal::Conflicted => Style::new().fg(Color::Red),
    };
    let remote_style = match bar.remote {
        CompactRemote::CiPassing => Style::new().fg(Color::Green),
        CompactRemote::CiFailing => Style::new().fg(Color::Red),
        CompactRemote::CiPending => Style::new().fg(Color::Yellow),
        CompactRemote::Unknown
        | CompactRemote::Unsupported
        | CompactRemote::Unavailable
        | CompactRemote::Invalid => Style::new().fg(Color::DarkGray),
    };
    debug_assert_eq!(bar.plain_lines().len(), 3);
    vec![
        Line::from(vec![
            Span::styled(bar.name.clone(), Style::new().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(bar.attention_label().to_string(), attn_style),
            Span::raw("  "),
            Span::styled(bar.path.clone(), Style::new().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::raw(bar.branch.clone()),
            Span::raw("  "),
            Span::styled(compact_local_label(bar.local).to_string(), local_style),
            Span::raw(format!("  ahead {}  behind {}", bar.ahead, bar.behind)),
        ]),
        Line::from(vec![
            Span::styled(compact_remote_label(bar.remote).to_string(), remote_style),
            Span::raw(format!("  {}  {}", bar.freshness, bar.provider)),
        ]),
    ]
}

fn pane_block(title: &str, focused: bool) -> Block<'static> {
    let title = if focused {
        format!(" {title} ● ")
    } else {
        format!(" {title} ")
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    if focused {
        block.border_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        block.border_style(Style::default().fg(Color::DarkGray))
    }
}

fn draw_help(frame: &mut ratatui::Frame) {
    let text = "\
Git Forge Cockpit — keyboard reference\n\
\n\
  j/k       move in repo list (scroll README when focused)\n\
  J/K       scroll README\n\
  PgUp/PgDn scroll README\n\
  Tab       focus list or README\n\
  /         filter\n\
  s         cycle sort\n\
  a         toggle attention-only (default on)\n\
  r         rescan\n\
  Enter     default launch\n\
  e/t/b/g/c editor / terminal / browser / lazygit / custom\n\
  S         settings (same keys as config.toml)\n\
  Ctrl-d    mark daily triage done\n\
  q         quit\n";
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title(" help ")),
        centered(frame.area(), 78, 22),
    );
}

fn draw_settings(frame: &mut ratatui::Frame, app: &App) {
    let mut lines = Vec::new();
    for field in SettingsField::ALL {
        let marker = if field == app.settings_field {
            ">"
        } else {
            " "
        };
        let label = format!("{field:?}");
        let mut value = setting_value_for(app, field);
        if field == app.settings_field {
            if let Some(edit) = &app.edit {
                value = format!("{edit}_");
            }
        }
        lines.push(Line::from(vec![
            Span::raw(format!("{marker} {label}: ")),
            Span::raw(value),
        ]));
    }
    lines.push(Line::from(
        "Enter edit  space toggle  +/- bump  Ctrl-s save  Esc back",
    ));
    frame.render_widget(Clear, frame.area());
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" settings {} ", app.paths.config_file.display())),
        ),
        frame.area(),
    );
}

fn setting_value_for(app: &App, field: SettingsField) -> String {
    match field {
        SettingsField::Roots => app
            .config
            .roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        SettingsField::Concurrency => app.config.scan.concurrency.to_string(),
        SettingsField::LocalInterval => app.config.poll.local_interval_ms.to_string(),
        SettingsField::RemoteInterval => app.config.poll.remote_interval_secs.to_string(),
        SettingsField::DirtyAttention => app.config.health.dirty_needs_attention.to_string(),
        SettingsField::FetchStale => app.config.health.fetch_stale_after_secs.to_string(),
        SettingsField::UnfinishedStale => app
            .config
            .health
            .unfinished_work_stale_after_secs
            .to_string(),
        SettingsField::LocalCurrent => app.config.health.local_current_within_secs.to_string(),
        SettingsField::RemoteCurrent => app.config.health.remote_current_within_secs.to_string(),
        SettingsField::Editor => app.config.launch.editor.clone(),
        SettingsField::Terminal => app.config.launch.terminal.clone(),
        SettingsField::Browser => app.config.launch.browser.clone(),
        SettingsField::Lazygit => app.config.launch.lazygit.clone(),
        SettingsField::DefaultLaunch => format!("{:?}", app.config.launch.default_target),
        SettingsField::WebhookEnabled => app.config.webhook.enabled.to_string(),
        SettingsField::WebhookBind => app.config.webhook.bind.clone(),
        SettingsField::GithubEnabled => app.config.providers.github.enabled.to_string(),
        SettingsField::GitlabEnabled => app.config.providers.gitlab.enabled.to_string(),
        SettingsField::OriginEnabled => app.config.providers.origin.enabled.to_string(),
        SettingsField::OriginApp => app
            .config
            .providers
            .origin
            .app_credentials_configured
            .to_string(),
        SettingsField::Summarizer => app.config.plugins.summarizer_enabled.to_string(),
    }
}

fn centered(area: Rect, percent_x: u16, height: u16) -> Rect {
    let h = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height.min(100)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(h[1])[1]
}
