use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

use chrono::{DateTime, Local, NaiveTime, TimeZone};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};

mod actions;
mod config;
mod discord;
mod timer;

use crate::actions::{
    ActionCommand, build_action_commands, escape_terminal_text, preview_commands,
};
use crate::config::{
    Config, default_config_path, ensure_config, normalize_config, parse_duration, save_config,
    validate_config,
};
use crate::discord::DiscordPresence;
use crate::timer::{
    HistoryEntry, RunMode, TIMER_STOPPED_MSG, TimerOptions, TimerReport, format_duration,
    run_timer, unix_timestamp,
};

#[derive(Parser)]
#[command(
    name = "gosleep-timer",
    about = "Configure and run post-countdown desktop actions"
)]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        no_actions: bool,
        duration: Option<String>,
    },
    Status,
    Validate,
    Edit,
    History,
    Stats,
    Preview,
    Init,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(default_config_path);
    let mut config = ensure_config(&path)?;

    match cli.command {
        Some(Commands::Run {
            dry_run,
            no_actions,
            duration,
        }) => {
            if let Some(duration) = duration {
                config.duration = duration;
            }
            let mut stdout = io::stdout();
            let options = TimerOptions {
                dry_run,
                no_actions,
            };
            let (report, _sleep_presence) = run_timer(
                &config,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicU64::new(0)),
                RunMode::Cli,
                options,
                Some(&mut stdout),
            )?;
            // `_sleep_presence` stays alive through the wake-confirmation wait
            // so the green elapsed timestamp remains visible; it is
            // disarmed, so dropping it at scope end closes the IPC connection
            // and lets Discord clear the status on process exit/poweroff.
            let slept_seconds = prompt_wake_confirmation(&config, options)?;
            let status = if report.failures.is_empty() {
                "finished"
            } else {
                "partial"
            };
            append_history(
                &history_path(&path),
                report.to_history(status, slept_seconds),
            )?;
            if !report.failures.is_empty() {
                bail!(
                    "{} action(s) failed: {}",
                    report.failures.len(),
                    report.failures.join("; ")
                );
            }
            Ok(())
        }
        Some(Commands::Status) => {
            print_status(&path, &config);
            Ok(())
        }
        Some(Commands::Validate) => {
            validate_config(&config)?;
            println!("valid {}", path.display());
            Ok(())
        }
        Some(Commands::Edit) => edit_config(&path),
        Some(Commands::History) => {
            print_history(&history_path(&path))?;
            Ok(())
        }
        Some(Commands::Stats) => {
            print_stats(&history_path(&path))?;
            Ok(())
        }
        Some(Commands::Preview) => {
            for command in preview_commands(&config) {
                println!("{command}");
            }
            Ok(())
        }
        Some(Commands::Init) => {
            println!("{}", path.display());
            Ok(())
        }
        None => run_tui(path, config),
    }
}

fn history_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("history.yaml")
}

fn read_history(path: &Path) -> Result<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    noyalib::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn append_history(path: &Path, entry: HistoryEntry) -> Result<()> {
    let mut entries = read_history(path)?;
    entries.push(entry);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = noyalib::to_string(&entries).context("serialize history")?;
    fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn print_status(path: &Path, config: &Config) {
    println!("config: {}", path.display());
    println!("history: {}", history_path(path).display());
    println!("duration: {}", config.duration);
    println!("actions:");
    let commands = build_action_commands(config);
    if commands.is_empty() {
        println!("  none");
    } else {
        for command in commands {
            let danger = if command.is_dangerous() { " !" } else { "" };
            println!("  - [{}{}] {}", command.label, danger, command.display());
        }
    }
}

fn print_history(path: &Path) -> Result<()> {
    let entries = read_history(path)?;
    if entries.is_empty() {
        println!("no history");
        return Ok(());
    }
    for entry in entries.iter().rev().take(20) {
        let slept = entry
            .slept_seconds
            .map(|seconds| format!(" slept={}", format_duration(Duration::from_secs(seconds))))
            .unwrap_or_default();
        println!(
            "{} status={} duration={} actions={} failures={}{}",
            entry.started_at,
            entry.status,
            format_duration(Duration::from_secs(entry.duration_seconds)),
            entry.actions.len(),
            entry.failures.len(),
            slept
        );
    }
    Ok(())
}

fn print_stats(path: &Path) -> Result<()> {
    let entries = read_history(path)?;
    let finished = entries
        .iter()
        .filter(|entry| entry.status == "finished")
        .count();
    let total_duration: u64 = entries.iter().map(|entry| entry.duration_seconds).sum();
    let total_slept: u64 = entries.iter().filter_map(|entry| entry.slept_seconds).sum();
    let failures: usize = entries.iter().map(|entry| entry.failures.len()).sum();
    println!("sessions: {}", entries.len());
    println!("finished: {finished}");
    println!(
        "scheduled: {}",
        format_duration(Duration::from_secs(total_duration))
    );
    println!(
        "slept: {}",
        format_duration(Duration::from_secs(total_slept))
    );
    println!("action failures: {failures}");
    Ok(())
}

fn edit_config(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let program = parts.next().context("empty editor command")?;
    let mut cmd = Command::new(program);
    cmd.args(parts);
    cmd.arg(path);
    let status = cmd.status().context("open editor")?;
    if !status.success() {
        bail!("editor exited with {status}");
    }
    let _ = ensure_config(path)?;
    Ok(())
}

fn prompt_wake_confirmation(config: &Config, options: TimerOptions) -> Result<Option<u64>> {
    if options.dry_run
        || options.no_actions
        || matches!(config.actions.power.mode.as_str(), "poweroff" | "reboot")
    {
        return Ok(None);
    }
    print!("timer finished. Press Enter when awake to record sleep time...");
    io::stdout().flush().ok();
    let started = Instant::now();
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("read wake confirmation")?;
    Ok(Some(started.elapsed().as_secs()))
}

fn run_tui(path: PathBuf, config: Config) -> Result<()> {
    enable_raw_mode()?;
    let result = (|| {
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let mut app = App::new(path, config);
        let result = app.run(&mut terminal);
        terminal.show_cursor().ok();
        result
    })();
    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();
    result
}

// Only complete lines cross into the UI thread. Command output stays off the terminal.
struct UpdateWriter {
    sender: mpsc::Sender<String>,
    pending: String,
}

impl Write for UpdateWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.push_str(&String::from_utf8_lossy(bytes));
        while let Some(end) = self.pending.find('\n') {
            let line = self.pending.drain(..=end).collect::<String>();
            let _ = self.sender.send(line.trim_end().to_string());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

const GROUPS: &[&str] = &["Timer", "Audio", "Screen", "Session", "More"];

fn field_group(key: FieldKey) -> usize {
    match key {
        FieldKey::Duration | FieldKey::UntilTime | FieldKey::InhibitSleep => 0,
        FieldKey::MediaEnabled
        | FieldKey::MediaAction
        | FieldKey::MuteEnabled
        | FieldKey::MuteInputEnabled => 1,
        FieldKey::BrightnessEnabled
        | FieldKey::BrightnessValue
        | FieldKey::MonitorEnabled
        | FieldKey::MonitorBackend
        | FieldKey::MonitorCommand => 2,
        FieldKey::WorkspaceEnabled
        | FieldKey::WorkspaceBackend
        | FieldKey::WorkspaceNumber
        | FieldKey::WorkspaceCommand
        | FieldKey::LockEnabled
        | FieldKey::LockBackend
        | FieldKey::LockCommand => 3,
        _ => 4,
    }
}

fn field_hint(field: Field) -> &'static str {
    match field.key {
        FieldKey::Duration => "Duration: 30m, 1h30m, 90s; clears until time",
        FieldKey::UntilTime => "HH:MM local time; next occurrence; blank clears",
        FieldKey::BrightnessValue => "Brightness: 0..100 percent",
        FieldKey::WorkspaceNumber => "Workspace number: integer",
        FieldKey::InhibitSleep => "Keep system awake while timer runs",
        FieldKey::PowerMode => "System action after countdown",
        _ => match field.kind {
            FieldKind::Bool => "Space or Enter switches on/off",
            FieldKind::Cycle(_) => "Space or Enter selects the next option",
            FieldKind::Csv => "Separate process names with commas",
            FieldKind::Semi => "Separate shell commands with semicolons",
            _ => "Text: Enter applies; Esc discards the edit",
        },
    }
}

fn action_description(command: &ActionCommand, config: &Config) -> String {
    match command.label {
        "workspace" => format!("Switch to workspace {}", config.actions.workspace.number),
        "media" => format!("Media: {}", config.actions.media.action),
        "brightness" => format!("Set brightness to {}%", config.actions.brightness.value),
        "mute" => "Mute speakers".to_string(),
        "mute_input" => "Mute microphone".to_string(),
        "monitor" => "Turn off monitors".to_string(),
        "lock" => "Lock session".to_string(),
        "notify" => "Send notification".to_string(),
        "kill" => "Stop matching processes".to_string(),
        "power" => format!("System: {}", config.actions.power.mode),
        "custom" => "Run custom shell command".to_string(),
        label => label.to_string(),
    }
}

fn parse_clock_time(value: &str) -> Result<NaiveTime> {
    let value = value.trim();
    if value.len() != 5 || value.as_bytes().get(2).is_none_or(|ch| *ch != b':') {
        bail!("expected local time HH:MM (00:00..23:59)");
    }
    NaiveTime::parse_from_str(value, "%H:%M").context("expected local time HH:MM (00:00..23:59)")
}

fn duration_until<T: TimeZone>(time: NaiveTime, now: DateTime<T>) -> Result<Duration> {
    let zone = now.timezone();
    for day in 0..=1 {
        let date = now
            .date_naive()
            .checked_add_days(chrono::Days::new(day))
            .context("date outside calendar range")?;
        let local = date.and_time(time);
        let resolved = zone.from_local_datetime(&local);
        let missing = matches!(resolved, chrono::LocalResult::None);
        if let Some(target) = [resolved.clone().earliest(), resolved.latest()]
            .into_iter()
            .flatten()
            .filter(|target| target > &now)
            .min_by_key(|target| target.timestamp())
        {
            return (target - now)
                .to_std()
                .context("end time must be in the future");
        }
        if missing && local > now.naive_local() {
            bail!(
                "that local time does not exist on {date} due to a clock change; choose another time"
            );
        }
    }
    bail!("cannot resolve the next local end time")
}

struct App {
    path: PathBuf,
    config: Config,
    focus: usize,
    offset: usize,
    status: String,
    timer: Option<TimerState>,
    edit: Option<EditState>,
    group: usize,
    until: Option<NaiveTime>,
    preview_offset: usize,
    show_log: bool,
    action_log: Vec<String>,
    sleep_presence: Option<DiscordPresence>,
    sleep_started_at: Option<u64>,
}

struct TimerState {
    duration: Duration,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    remaining_ms: Arc<AtomicU64>,
    handle: thread::JoinHandle<Result<(TimerReport, DiscordPresence)>>,
    updates: mpsc::Receiver<String>,
}

struct EditState {
    field: Field,
    value: String,
    cursor: usize,
}

fn edit_view(edit: &EditState, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "▌".to_string();
    }
    let value = escape_terminal_text(&edit.value);
    let cursor = edit
        .value
        .get(..edit.cursor)
        .map(|prefix| escape_terminal_text(prefix).len())
        .unwrap_or(value.len());
    let mut start = 0;
    while Line::from(&value[start..cursor]).width() + 1 + usize::from(start > 0) > width {
        start += value[start..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
    }
    let mut view = format!(
        "{}{}▌",
        if start > 0 { "‹" } else { "" },
        &value[start..cursor]
    );
    for ch in value[cursor..].chars() {
        if Line::from(format!("{view}{ch}")).width() > width {
            break;
        }
        view.push(ch);
    }
    view
}

impl App {
    fn new(path: PathBuf, config: Config) -> Self {
        Self {
            status: format!("loaded config: {}", path.display()),
            path,
            config,
            focus: 0,
            offset: 0,
            timer: None,
            edit: None,
            group: 0,
            until: None,
            preview_offset: 0,
            show_log: false,
            action_log: Vec::new(),
            sleep_presence: None,
            sleep_started_at: None,
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            self.reap_timer();
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(250))? {
                let Event::Key(key) = event::read()? else {
                    continue;
                };
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.stop_timer();
                    return Ok(());
                }
                if self.edit.is_some() {
                    self.handle_edit_key(key.code);
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            self.stop_timer();
                            return Ok(());
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            self.focus = self.focus.saturating_sub(1)
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let last = self.visible_fields().len().saturating_sub(1);
                            self.focus = (self.focus + 1).min(last);
                        }
                        KeyCode::Left => self.change_group(GROUPS.len() - 1),
                        KeyCode::Right => self.change_group(1),
                        KeyCode::Tab => self.change_group(1),
                        KeyCode::BackTab => self.change_group(GROUPS.len() - 1),
                        KeyCode::Char('1') => self.set_preset("30m"),
                        KeyCode::Char('2') => self.set_preset("1h"),
                        KeyCode::Char('3') => self.set_preset("2h"),
                        KeyCode::Char('l') => {
                            self.show_log = !self.show_log;
                            self.preview_offset = 0;
                        }
                        KeyCode::PageDown => {
                            self.preview_offset = self.preview_offset.saturating_add(3)
                        }
                        KeyCode::PageUp => {
                            self.preview_offset = self.preview_offset.saturating_sub(3)
                        }
                        KeyCode::Char(' ') | KeyCode::Enter if self.timer.is_none() => {
                            self.activate()?
                        }
                        KeyCode::Char('s') => self.save(),
                        KeyCode::Char('r') => self.start_timer(),
                        KeyCode::Char('p') => self.toggle_pause(),
                        KeyCode::Char('x') => self.stop_timer(),
                        _ => {}
                    }
                }
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 36 || area.height < 20 {
            frame.render_widget(Paragraph::new("terminal too small (need 36x20)"), area);
            return;
        }
        if self.timer.is_some() {
            self.draw_running(frame, area);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .split(area);

        self.draw_header(frame, chunks[0]);
        self.draw_body(frame, chunks[1]);
        let footer = if let Some(edit) = &self.edit {
            format!(
                "Editing {} | {}\nEnter apply | Esc cancel | Left/Right move | Home/End",
                edit.field.label,
                field_hint(edit.field)
            )
        } else {
            let action = self
                .focused_field()
                .map(|field| match field.kind {
                    FieldKind::Bool => "Space toggle",
                    FieldKind::Cycle(_) => "Enter next value",
                    _ => "Enter edit",
                })
                .unwrap_or("Enter edit");
            format!(
                "Left/Right group | Tab group | {action} | r run | s save | q quit\n1 30m | 2 1h | 3 2h | l preview/results | PgUp/PgDn scroll\n{}",
                self.focused_field().map(field_hint).unwrap_or("")
            )
        };
        let footer = if area.width < 75 {
            if let Some(edit) = &self.edit {
                format!(
                    "Editing: {}\nEnter apply | Esc cancel\nLeft/Right Home/End | Backspace/Del",
                    edit.field.label
                )
            } else {
                "Left/Right group | Enter edit | r run\n1/2/3 presets | l log | PgUp/PgDn\ns save | q quit"
                    .to_string()
            }
        } else {
            footer
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
            chunks[2],
        );
    }

    fn draw_running(&mut self, frame: &mut Frame, area: Rect) {
        let (_, remaining, progress) = self.timer_snapshot();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(1),
                Constraint::Length(8),
                Constraint::Length(2),
            ])
            .split(area);

        let paused = self
            .timer
            .as_ref()
            .is_some_and(|timer| timer.pause.load(Ordering::Relaxed));
        let state = if remaining.is_zero() {
            "running actions"
        } else if paused {
            "paused"
        } else {
            "running"
        };
        let end = chrono::Duration::from_std(remaining)
            .ok()
            .and_then(|delta| Local::now().checked_add_signed(delta))
            .map(|time| time.format("%a %H:%M:%S %Z").to_string())
            .unwrap_or_else(|| "outside calendar range".to_string());
        let heading = if paused {
            state.to_string()
        } else {
            format!("{state} | ends {end}")
        };
        frame.render_widget(
            Paragraph::new(heading).alignment(Alignment::Center),
            chunks[0],
        );
        let text = format_compact_duration(remaining);
        let time_lines = ascii_time(&text)
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>();
        let time_width = time_lines
            .iter()
            .map(|line| line.width() as u16)
            .max()
            .unwrap_or(0);
        if time_width > chunks[1].width || chunks[1].height < 7 {
            frame.render_widget(
                Paragraph::new(format_time_left_text(
                    self.timer.as_ref(),
                    remaining,
                    chunks[1].width,
                ))
                .alignment(Alignment::Center),
                chunks[1],
            );
        } else {
            let timer_area = center_rect(time_width, 7, chunks[1]);
            frame.render_widget(Paragraph::new(time_lines), timer_area);
        }

        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Magenta))
                .label(format_gauge_label(progress))
                .ratio(progress),
            chunks[2],
        );

        self.draw_preview(frame, chunks[3]);

        let footer = if self
            .timer
            .as_ref()
            .is_some_and(|timer| timer.pause.load(Ordering::Relaxed))
        {
            "paused | p resume | x stop | l results | PgUp/PgDn | q quit"
        } else {
            "running | p pause | x stop | l results | PgUp/PgDn | q quit"
        };
        let footer = if area.width < 75 {
            "p pause/resume | x stop | q quit\nl preview/results | PgUp/PgDn"
        } else {
            footer
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
            chunks[4],
        );
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let header = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);
        frame.render_widget(
            Paragraph::new("gosleep-timer")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            header[0],
        );
        let status = self.sleep_started_at.map_or_else(
            || self.status.clone(),
            |started_at| {
                format!(
                    "slept for {}",
                    format_duration(Duration::from_secs(
                        unix_timestamp().saturating_sub(started_at)
                    ))
                )
            },
        );
        frame.render_widget(
            Paragraph::new(status)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Yellow))
                .wrap(Wrap { trim: false }),
            header[1],
        );
        let ready = match self.until {
            Some(time) => format!("not running | next {} local time", time.format("%H:%M")),
            None => format!("not running | duration {}", self.config.duration),
        };
        frame.render_widget(
            Paragraph::new(ready).alignment(Alignment::Center),
            header[2],
        );
        let presets = if area.width < 75 {
            "1: 30m  2: 1h  3: 2h"
        } else {
            "1: 30m    2: 1h    3: 2h    Timer tab: until HH:MM"
        };
        frame.render_widget(
            Paragraph::new(presets)
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray)),
            header[3],
        );
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
        let active_tab_style = Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let inactive_tab_style = Style::default().fg(Color::Gray);
        let tabs = if area.width < 58 {
            vec![
                Span::raw("  "),
                Span::styled("‹ ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!(" {} ", GROUPS[self.group]), active_tab_style),
                Span::styled(" ›", Style::default().fg(Color::DarkGray)),
            ]
        } else {
            let mut tabs = vec![Span::raw("  ")];
            for (index, name) in GROUPS.iter().enumerate() {
                if index > 0 {
                    tabs.push(Span::raw("  "));
                }
                tabs.push(Span::styled(
                    format!(" {name} "),
                    if index == self.group {
                        active_tab_style
                    } else {
                        inactive_tab_style
                    },
                ));
            }
            tabs
        };
        frame.render_widget(Paragraph::new(Line::from(tabs)), chunks[0]);
        let area = chunks[1];
        if area.width >= 118 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
                .split(area);
            self.draw_fields(frame, chunks[0]);
            self.draw_preview(frame, chunks[1]);
        } else {
            let settings_height = (self.visible_fields().len() as u16 + 2)
                .clamp(4, 12)
                .min(area.height.saturating_sub(6));
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(settings_height), Constraint::Min(6)])
                .split(area);
            self.draw_fields(frame, chunks[0]);
            self.draw_preview(frame, chunks[1]);
        }
    }

    fn draw_fields(&mut self, frame: &mut Frame, area: Rect) {
        let fields = self.visible_fields();
        let visible = area.height.saturating_sub(2) as usize;
        self.focus = self.focus.min(fields.len().saturating_sub(1));
        self.offset = self.offset.min(fields.len().saturating_sub(visible));
        if self.focus < self.offset {
            self.offset = self.focus;
        }
        if self.focus >= self.offset + visible {
            self.offset = self.focus.saturating_sub(visible.saturating_sub(1));
        }

        let items = fields
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(visible)
            .map(|(index, field)| {
                let selected = index == self.focus;
                let marker = if selected { ">" } else { " " };
                let editing = self
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.field.key == field.key);
                let prefix = format!("  {marker} {}: ", field.label);
                let width = usize::from(area.width.saturating_sub(2)).saturating_sub(prefix.len());
                let value = if editing {
                    edit_view(self.edit.as_ref().expect("edit checked"), width)
                } else {
                    escape_terminal_text(&self.field_value(field))
                };
                let line = format!("{prefix}{value}");
                let style = if editing {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else if selected {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(line).style(style)
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            List::new(items).block(
                Block::default()
                    .title(format!(
                        " {} settings ({}/{}) ",
                        GROUPS[self.group],
                        self.focus + 1,
                        fields.len()
                    ))
                    .borders(Borders::ALL)
                    .border_style(Color::Blue),
            ),
            area,
        );
    }

    fn draw_preview(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines = Vec::new();
        if self.show_log {
            for message in &self.action_log {
                let color = if message.contains("failed") {
                    Color::Red
                } else {
                    Color::Green
                };
                lines.extend(
                    wrap_command(message, area.width.saturating_sub(4) as usize)
                        .into_iter()
                        .map(|line| Line::styled(line, Style::default().fg(color))),
                );
            }
            if lines.is_empty() {
                lines.push(Line::from("No results yet. Press r to start."));
            }
        } else {
            lines = action_plan_lines(&self.config, usize::from(area.width.saturating_sub(2)));
            if lines.is_empty() {
                lines.push(Line::from("No actions enabled; countdown only."));
            }
        }
        let title = if self.show_log {
            "results"
        } else if area.width < 60 {
            "plan / commands"
        } else {
            "action plan / commands"
        };
        let base_block = Block::default()
            .title(format!(" {title} "))
            .borders(Borders::ALL)
            .border_style(Color::Blue);
        let inner = base_block.inner(area);

        // Use the unused space below a short plan or results list for a timer.
        // If the content is long, keep the existing scrollable view unchanged.
        let show_elapsed = self.show_log && self.sleep_started_at.is_some();
        let show_card = (!self.show_log || show_elapsed)
            && inner.width >= 50
            && usize::from(inner.height) >= lines.len().saturating_add(10);
        let range = if show_card {
            String::new()
        } else {
            let height = usize::from(inner.height);
            self.preview_offset = self.preview_offset.min(lines.len().saturating_sub(height));
            if lines.len() > height {
                format!(
                    " {}-{}/{}",
                    self.preview_offset + 1,
                    (self.preview_offset + height).min(lines.len()),
                    lines.len()
                )
            } else {
                String::new()
            }
        };
        let block = Block::default()
            .title(format!(" {title}{range} "))
            .borders(Borders::ALL)
            .border_style(Color::Blue);
        frame.render_widget(block, area);

        if show_card {
            let plan_height = u16::try_from(lines.len()).unwrap_or(inner.height);
            frame.render_widget(
                Paragraph::new(lines),
                Rect {
                    x: inner.x,
                    y: inner.y,
                    width: inner.width,
                    height: plan_height,
                },
            );

            let card_area = Rect {
                x: inner.x,
                y: inner.y + plan_height,
                width: inner.width,
                height: inner.height.saturating_sub(plan_height),
            };
            let (card_title, duration_text, footer) =
                if let Some(started_at) = self.sleep_started_at.filter(|_| show_elapsed) {
                    let elapsed = Duration::from_secs(unix_timestamp().saturating_sub(started_at));
                    let started_local =
                        DateTime::<Local>::from(UNIX_EPOCH + Duration::from_secs(started_at));
                    (
                        "SLEPT FOR",
                        format_compact_duration(elapsed),
                        format!("since timer ended at {}", started_local.format("%H:%M")),
                    )
                } else {
                    let now = Local::now();
                    let duration = self.until.map_or_else(
                        || parse_duration(&self.config.duration).ok(),
                        |time| duration_until(time, now).ok(),
                    );
                    let duration_text = duration
                        .map(format_compact_duration)
                        .unwrap_or_else(|| "--:--".to_string());
                    let finish_text = duration
                        .and_then(|value| chrono::Duration::from_std(value).ok())
                        .and_then(|value| now.checked_add_signed(value))
                        .map(|time| time.format("%H:%M").to_string())
                        .unwrap_or_else(|| "--:--".to_string());
                    let action_count = build_action_commands(&self.config).len();
                    let action_label = if action_count == 1 {
                        "action"
                    } else {
                        "actions"
                    };
                    (
                        "TIME LEFT",
                        duration_text,
                        format!("ENDS AT {finish_text}  ·  {action_count} {action_label}"),
                    )
                };

            let mut card_lines = vec![Line::styled(
                card_title,
                Style::default().fg(Color::DarkGray),
            )];
            let large_timer = ascii_time(&duration_text);
            if let Some(sample) = large_timer.first()
                && sample.len() <= usize::from(card_area.width.saturating_sub(2))
                && card_area.height >= 10
            {
                card_lines.extend(large_timer.into_iter().map(|row| {
                    Line::styled(
                        row,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                }));
            } else {
                card_lines.push(Line::styled(
                    duration_text,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            card_lines.push(Line::styled(footer, Style::default().fg(Color::Gray)));
            let card_height = u16::try_from(card_lines.len()).unwrap_or(card_area.height);
            frame.render_widget(
                Paragraph::new(card_lines)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(Color::Gray)),
                center_rect(card_area.width, card_height, card_area),
            );
            self.preview_offset = 0;
            return;
        }

        let height = usize::from(inner.height);
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(self.preview_offset)
                    .take(height)
                    .collect::<Vec<_>>(),
            ),
            inner,
        );
    }

    fn change_group(&mut self, step: usize) {
        if self.timer.is_some() {
            return;
        }
        self.group = (self.group + step) % GROUPS.len();
        self.focus = 0;
        self.offset = 0;
    }

    fn set_preset(&mut self, duration: &str) {
        if self.timer.is_some() {
            return;
        }
        self.until = None;
        self.config.duration = duration.to_string();
        self.status = format!("duration set to {duration}; press r to start");
    }

    fn activate(&mut self) -> Result<()> {
        let Some(field) = self.focused_field() else {
            return Ok(());
        };
        match field.kind {
            FieldKind::Bool => self.toggle_bool(field.key),
            FieldKind::Cycle(options) => self.cycle(field.key, options),
            FieldKind::Edit | FieldKind::Int | FieldKind::Csv | FieldKind::Semi => {
                self.edit = Some(EditState {
                    field,
                    cursor: self.field_value(&field).len(),
                    value: self.field_value(&field),
                });
            }
        }
        Ok(())
    }

    fn handle_edit_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => self.edit = None,
            KeyCode::Enter => {
                let (field, value) = match &self.edit {
                    Some(edit) => (edit.field, edit.value.clone()),
                    None => return,
                };
                match self.apply_edit(field, value) {
                    Ok(()) => self.edit = None,
                    Err(err) => self.status = format!("edit failed: {err}"),
                }
            }
            KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::Char(_) => {
                if let Some(edit) = &mut self.edit {
                    let previous = edit.value[..edit.cursor]
                        .char_indices()
                        .next_back()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    let next = edit.value[edit.cursor..]
                        .chars()
                        .next()
                        .map(|ch| edit.cursor + ch.len_utf8())
                        .unwrap_or(edit.cursor);
                    match key {
                        KeyCode::Backspace => {
                            edit.value.replace_range(previous..edit.cursor, "");
                            edit.cursor = previous;
                        }
                        KeyCode::Delete => {
                            edit.value.replace_range(edit.cursor..next, "");
                        }
                        KeyCode::Left => edit.cursor = previous,
                        KeyCode::Right => edit.cursor = next,
                        KeyCode::Home => edit.cursor = 0,
                        KeyCode::End => edit.cursor = edit.value.len(),
                        KeyCode::Char(ch) => {
                            edit.value.insert(edit.cursor, ch);
                            edit.cursor += ch.len_utf8();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn apply_edit(&mut self, field: Field, value: String) -> Result<()> {
        match field.kind {
            FieldKind::Edit => match field.key {
                FieldKey::Duration => {
                    parse_duration(&value)?;
                    self.config.duration = value;
                    self.until = None;
                }
                FieldKey::UntilTime => {
                    self.until = if value.trim().is_empty() {
                        None
                    } else {
                        let time = parse_clock_time(&value)?;
                        duration_until(time, Local::now())?;
                        Some(time)
                    };
                }
                FieldKey::WorkspaceCommand => self.config.actions.workspace.command = value,
                FieldKey::LockCommand => self.config.actions.lock.command = value,
                FieldKey::MonitorCommand => self.config.actions.monitor.command = value,
                FieldKey::NotifyMessage => self.config.actions.notify.message = value,
                FieldKey::DiscordApplicationId => {
                    self.config.discord.application_id = value.trim().to_string()
                }
                _ => {}
            },
            FieldKind::Int => {
                let number: u32 = value.trim().parse().context("expected integer")?;
                match field.key {
                    FieldKey::WorkspaceNumber => self.config.actions.workspace.number = number,
                    FieldKey::BrightnessValue => {
                        if number > 100 {
                            bail!("brightness must be between 0 and 100");
                        }
                        self.config.actions.brightness.value = number as u8
                    }
                    _ => {}
                }
            }
            FieldKind::Csv => {
                let values = split_list(&value, ',');
                if matches!(field.key, FieldKey::KillProcesses) {
                    self.config.actions.kill.processes = values;
                }
            }
            FieldKind::Semi => {
                let values = split_list(&value, ';');
                if matches!(field.key, FieldKey::CustomCommands) {
                    self.config.actions.custom.commands = values;
                }
            }
            FieldKind::Bool | FieldKind::Cycle(_) => {}
        }
        normalize_config(&mut self.config);
        self.status = format!("updated {}", field.label);
        Ok(())
    }

    fn save(&mut self) {
        match save_config(&self.path, &self.config) {
            Ok(()) => self.status = format!("saved {}", self.path.display()),
            Err(err) => self.status = format!("save failed: {err}"),
        }
    }

    fn start_timer(&mut self) {
        self.stop_timer();
        if let Err(err) = validate_config(&self.config) {
            self.status = format!("timer failed: {err}");
            return;
        }
        let duration = match self
            .until
            .map(|time| duration_until(time, Local::now()))
            .unwrap_or_else(|| parse_duration(&self.config.duration))
        {
            Ok(duration) => duration,
            Err(err) => {
                self.status = format!("timer failed: {err}");
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let remaining_ms = Arc::new(AtomicU64::new(
            duration.as_millis().min(u128::from(u64::MAX)) as u64,
        ));
        let thread_stop = Arc::clone(&stop);
        let thread_pause = Arc::clone(&pause);
        let thread_remaining_ms = Arc::clone(&remaining_ms);
        let mut config = self.config.clone();
        if self.until.is_some() {
            config.duration = format!("{}ns", duration.as_nanos());
        }
        let (sender, updates) = mpsc::channel();
        self.action_log.clear();
        self.preview_offset = 0;
        let handle = thread::spawn(move || {
            let mut output = UpdateWriter {
                sender,
                pending: String::new(),
            };
            run_timer(
                &config,
                &thread_stop,
                &thread_pause,
                &thread_remaining_ms,
                RunMode::Tui,
                TimerOptions::default(),
                Some(&mut output),
            )
        });
        self.timer = Some(TimerState {
            duration,
            stop,
            pause,
            remaining_ms,
            handle,
            updates,
        });
        self.status = "timer running".to_string();
    }

    fn toggle_pause(&mut self) {
        let Some(timer) = &self.timer else {
            return;
        };
        let paused = !timer.pause.load(Ordering::Relaxed);
        timer.pause.store(paused, Ordering::Relaxed);
        self.status = if paused {
            "timer paused".to_string()
        } else {
            "timer running".to_string()
        };
    }

    fn stop_timer(&mut self) {
        // A new run replaces any retained post-countdown status; stopping
        // clears it explicitly so Discord never shows stale sleep state.
        if let Some(presence) = self.sleep_presence.take() {
            presence.clear();
        }
        if let Some(timer) = self.timer.take() {
            timer.stop.store(true, Ordering::Relaxed);
            let result = timer.handle.join();
            self.action_log.extend(timer.updates.try_iter());
            self.apply_timer_result(result);
        } else {
            self.status = "timer stopped".to_string();
        }
        self.sleep_started_at = None;
    }

    fn reap_timer(&mut self) {
        if let Some(timer) = &self.timer {
            self.action_log.extend(timer.updates.try_iter());
        }
        if self
            .timer
            .as_ref()
            .is_some_and(|timer| timer.handle.is_finished())
        {
            let Some(timer) = self.timer.take() else {
                return;
            };
            let result = timer.handle.join();
            self.action_log.extend(timer.updates.try_iter());
            self.apply_timer_result(result);
        }
    }

    fn apply_timer_result(
        &mut self,
        result: thread::Result<Result<(TimerReport, DiscordPresence)>>,
    ) {
        match result {
            Ok(Ok((report, sleep_presence))) => {
                // Retain the disarmed post-countdown guard while the TUI stays
                // open so the green elapsed timer (or `finished`) remains visible.
                // Dropping `App` closes the IPC connection and lets Discord
                // clear it; `stop_timer` clears it explicitly on stop/quit.
                self.sleep_presence = Some(sleep_presence);
                self.sleep_started_at = report.sleep_started_at;
                self.show_log = true;
                self.preview_offset = 0;
                let status = if report.failures.is_empty() {
                    "finished"
                } else {
                    "partial"
                };
                if let Err(err) =
                    append_history(&history_path(&self.path), report.to_history(status, None))
                {
                    self.status = format!("history failed: {err}");
                } else if !report.failures.is_empty() {
                    self.status = format!(
                        "timer finished ({} action(s) failed)",
                        report.failures.len()
                    );
                } else {
                    self.status = "timer finished".to_string();
                }
            }
            Ok(Err(err)) if err.to_string() == TIMER_STOPPED_MSG => {
                self.status = "timer stopped".to_string()
            }
            Ok(Err(err)) => {
                self.status = format!("timer failed: {err}");
                self.action_log.push(self.status.clone());
                self.show_log = true;
            }
            Err(_) => self.status = "timer failed: thread panicked".to_string(),
        }
    }

    fn timer_snapshot(&self) -> (Duration, Duration, f64) {
        let Some(timer) = &self.timer else {
            return (Duration::ZERO, Duration::ZERO, 0.0);
        };
        let remaining = Duration::from_millis(timer.remaining_ms.load(Ordering::Relaxed));
        let elapsed = timer.duration.saturating_sub(remaining).min(timer.duration);
        let progress = if timer.duration.is_zero() {
            0.0
        } else {
            elapsed.as_secs_f64() / timer.duration.as_secs_f64()
        };
        (elapsed, remaining, progress.clamp(0.0, 1.0))
    }

    fn field_value(&self, field: &Field) -> String {
        match field.key {
            FieldKey::Duration => self.config.duration.clone(),
            FieldKey::UntilTime => self
                .until
                .map(|time| time.format("%H:%M").to_string())
                .unwrap_or_default(),
            FieldKey::InhibitSleep => on_off(self.config.inhibit_sleep),
            FieldKey::WorkspaceEnabled => on_off(self.config.actions.workspace.enabled),
            FieldKey::WorkspaceBackend => self.config.actions.workspace.backend.clone(),
            FieldKey::WorkspaceNumber => self.config.actions.workspace.number.to_string(),
            FieldKey::WorkspaceCommand => self.config.actions.workspace.command.clone(),
            FieldKey::MediaEnabled => on_off(self.config.actions.media.enabled),
            FieldKey::MediaAction => self.config.actions.media.action.clone(),
            FieldKey::BrightnessEnabled => on_off(self.config.actions.brightness.enabled),
            FieldKey::BrightnessValue => self.config.actions.brightness.value.to_string(),
            FieldKey::MuteEnabled => on_off(self.config.actions.mute.enabled),
            FieldKey::MuteInputEnabled => on_off(self.config.actions.mute_input.enabled),
            FieldKey::LockEnabled => on_off(self.config.actions.lock.enabled),
            FieldKey::LockBackend => self.config.actions.lock.backend.clone(),
            FieldKey::LockCommand => self.config.actions.lock.command.clone(),
            FieldKey::MonitorEnabled => on_off(self.config.actions.monitor.enabled),
            FieldKey::MonitorBackend => self.config.actions.monitor.backend.clone(),
            FieldKey::MonitorCommand => self.config.actions.monitor.command.clone(),
            FieldKey::NotifyEnabled => on_off(self.config.actions.notify.enabled),
            FieldKey::NotifyMessage => self.config.actions.notify.message.clone(),
            FieldKey::KillEnabled => on_off(self.config.actions.kill.enabled),
            FieldKey::KillProcesses => self.config.actions.kill.processes.join(","),
            FieldKey::PowerMode => self.config.actions.power.mode.clone(),
            FieldKey::CustomEnabled => on_off(self.config.actions.custom.enabled),
            FieldKey::CustomCommands => self.config.actions.custom.commands.join(";"),
            FieldKey::DiscordEnabled => on_off(self.config.discord.enabled),
            FieldKey::DiscordApplicationId => self.config.discord.application_id.clone(),
        }
    }

    fn toggle_bool(&mut self, key: FieldKey) {
        match key {
            FieldKey::InhibitSleep => self.config.inhibit_sleep = !self.config.inhibit_sleep,
            FieldKey::WorkspaceEnabled => {
                self.config.actions.workspace.enabled = !self.config.actions.workspace.enabled
            }
            FieldKey::MediaEnabled => {
                self.config.actions.media.enabled = !self.config.actions.media.enabled
            }
            FieldKey::BrightnessEnabled => {
                self.config.actions.brightness.enabled = !self.config.actions.brightness.enabled
            }
            FieldKey::MuteEnabled => {
                self.config.actions.mute.enabled = !self.config.actions.mute.enabled
            }
            FieldKey::MuteInputEnabled => {
                self.config.actions.mute_input.enabled = !self.config.actions.mute_input.enabled
            }
            FieldKey::LockEnabled => {
                self.config.actions.lock.enabled = !self.config.actions.lock.enabled
            }
            FieldKey::KillEnabled => {
                self.config.actions.kill.enabled = !self.config.actions.kill.enabled
            }
            FieldKey::MonitorEnabled => {
                self.config.actions.monitor.enabled = !self.config.actions.monitor.enabled
            }
            FieldKey::NotifyEnabled => {
                self.config.actions.notify.enabled = !self.config.actions.notify.enabled
            }
            FieldKey::CustomEnabled => {
                self.config.actions.custom.enabled = !self.config.actions.custom.enabled
            }
            FieldKey::DiscordEnabled => self.config.discord.enabled = !self.config.discord.enabled,
            _ => {}
        }
    }

    fn cycle(&mut self, key: FieldKey, options: &'static [&'static str]) {
        let current = match key {
            FieldKey::WorkspaceBackend => &mut self.config.actions.workspace.backend,
            FieldKey::MediaAction => &mut self.config.actions.media.action,
            FieldKey::PowerMode => &mut self.config.actions.power.mode,
            FieldKey::MonitorBackend => &mut self.config.actions.monitor.backend,
            FieldKey::LockBackend => &mut self.config.actions.lock.backend,
            _ => return,
        };
        let index = options
            .iter()
            .position(|option| *option == current)
            .unwrap_or(0);
        *current = options[(index + 1) % options.len()].to_string();
    }

    fn visible_fields(&self) -> Vec<&'static Field> {
        visible_fields(&self.config)
            .into_iter()
            .filter(|field| field_group(field.key) == self.group)
            .collect()
    }

    fn focused_field(&self) -> Option<Field> {
        self.visible_fields().get(self.focus).map(|field| **field)
    }
}

#[derive(Clone, Copy)]
struct Field {
    label: &'static str,
    key: FieldKey,
    kind: FieldKind,
}

#[derive(Clone, Copy)]
enum FieldKind {
    Edit,
    Bool,
    Cycle(&'static [&'static str]),
    Int,
    Csv,
    Semi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldKey {
    Duration,
    UntilTime,
    InhibitSleep,
    WorkspaceEnabled,
    WorkspaceBackend,
    WorkspaceNumber,
    WorkspaceCommand,
    MediaEnabled,
    MediaAction,
    BrightnessEnabled,
    BrightnessValue,
    MuteEnabled,
    MuteInputEnabled,
    LockEnabled,
    LockBackend,
    LockCommand,
    MonitorEnabled,
    MonitorBackend,
    MonitorCommand,
    NotifyEnabled,
    NotifyMessage,
    KillEnabled,
    KillProcesses,
    PowerMode,
    CustomEnabled,
    CustomCommands,
    DiscordEnabled,
    DiscordApplicationId,
}

const FIELDS: &[Field] = &[
    Field {
        label: "duration",
        key: FieldKey::Duration,
        kind: FieldKind::Edit,
    },
    Field {
        label: "until local time",
        key: FieldKey::UntilTime,
        kind: FieldKind::Edit,
    },
    Field {
        label: "media enabled",
        key: FieldKey::MediaEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "media key action",
        key: FieldKey::MediaAction,
        kind: FieldKind::Cycle(&["stop", "pause", "play-pause", "next", "previous", "none"]),
    },
    Field {
        label: "mute enabled",
        key: FieldKey::MuteEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "mic mute enabled",
        key: FieldKey::MuteInputEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "brightness enabled",
        key: FieldKey::BrightnessEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "brightness value",
        key: FieldKey::BrightnessValue,
        kind: FieldKind::Int,
    },
    Field {
        label: "workspace enabled",
        key: FieldKey::WorkspaceEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "workspace backend",
        key: FieldKey::WorkspaceBackend,
        kind: FieldKind::Cycle(&["auto", "niri", "hyprland", "sway", "i3", "kde"]),
    },
    Field {
        label: "workspace number",
        key: FieldKey::WorkspaceNumber,
        kind: FieldKind::Int,
    },
    Field {
        label: "workspace command",
        key: FieldKey::WorkspaceCommand,
        kind: FieldKind::Edit,
    },
    Field {
        label: "lock enabled",
        key: FieldKey::LockEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "lock backend",
        key: FieldKey::LockBackend,
        kind: FieldKind::Cycle(&[
            "auto",
            "loginctl",
            "hyprlock",
            "swaylock",
            "i3lock",
            "gnome",
            "kde",
            "xscreensaver",
            "noctalia-qs",
            "noctalia",
        ]),
    },
    Field {
        label: "lock command",
        key: FieldKey::LockCommand,
        kind: FieldKind::Edit,
    },
    Field {
        label: "screen poweroff",
        key: FieldKey::MonitorEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "screen poweroff backend",
        key: FieldKey::MonitorBackend,
        kind: FieldKind::Cycle(&["auto", "hyprland", "niri", "sway", "kde", "x11"]),
    },
    Field {
        label: "screen poweroff command",
        key: FieldKey::MonitorCommand,
        kind: FieldKind::Edit,
    },
    Field {
        label: "prevent sleep",
        key: FieldKey::InhibitSleep,
        kind: FieldKind::Bool,
    },
    Field {
        label: "notification",
        key: FieldKey::NotifyEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "notification message",
        key: FieldKey::NotifyMessage,
        kind: FieldKind::Edit,
    },
    Field {
        label: "kill processes",
        key: FieldKey::KillEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "kill processes",
        key: FieldKey::KillProcesses,
        kind: FieldKind::Csv,
    },
    Field {
        label: "power",
        key: FieldKey::PowerMode,
        kind: FieldKind::Cycle(&[
            "none",
            "suspend",
            "hibernate",
            "hybrid-sleep",
            "poweroff",
            "reboot",
        ]),
    },
    Field {
        label: "custom enabled",
        key: FieldKey::CustomEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "custom shell cmds",
        key: FieldKey::CustomCommands,
        kind: FieldKind::Semi,
    },
    Field {
        label: "discord enabled",
        key: FieldKey::DiscordEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "discord app id",
        key: FieldKey::DiscordApplicationId,
        kind: FieldKind::Edit,
    },
];

fn visible_fields(config: &Config) -> Vec<&'static Field> {
    FIELDS
        .iter()
        .filter(|field| field_visible(config, field.key))
        .collect()
}

fn field_visible(config: &Config, key: FieldKey) -> bool {
    match key {
        FieldKey::WorkspaceBackend | FieldKey::WorkspaceNumber => config.actions.workspace.enabled,
        FieldKey::WorkspaceCommand => {
            config.actions.workspace.enabled && config.actions.workspace.backend == "auto"
        }
        FieldKey::MediaAction => config.actions.media.enabled,
        FieldKey::BrightnessValue => config.actions.brightness.enabled,
        FieldKey::LockBackend => config.actions.lock.enabled,
        FieldKey::LockCommand => {
            config.actions.lock.enabled && config.actions.lock.backend == "auto"
        }
        FieldKey::MonitorBackend => config.actions.monitor.enabled,
        FieldKey::MonitorCommand => {
            config.actions.monitor.enabled && config.actions.monitor.backend == "auto"
        }
        FieldKey::NotifyMessage => config.actions.notify.enabled,
        FieldKey::KillProcesses => config.actions.kill.enabled,
        FieldKey::CustomCommands => config.actions.custom.enabled,
        FieldKey::Duration
        | FieldKey::UntilTime
        | FieldKey::InhibitSleep
        | FieldKey::WorkspaceEnabled
        | FieldKey::MediaEnabled
        | FieldKey::BrightnessEnabled
        | FieldKey::MuteEnabled
        | FieldKey::MuteInputEnabled
        | FieldKey::LockEnabled
        | FieldKey::MonitorEnabled
        | FieldKey::NotifyEnabled
        | FieldKey::KillEnabled
        | FieldKey::PowerMode
        | FieldKey::CustomEnabled
        | FieldKey::DiscordEnabled
        | FieldKey::DiscordApplicationId => true,
    }
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

fn format_compact_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_time_left_text(timer: Option<&TimerState>, remaining: Duration, width: u16) -> String {
    if timer.is_none() {
        return "not running".to_string();
    }

    let full = format!("time left: {}", format_duration(remaining));
    if full.len() <= width as usize {
        return full;
    }

    let compact = format!("left: {}", format_compact_duration(remaining));
    if compact.len() <= width as usize {
        return compact;
    }

    format_compact_duration(remaining)
}

fn format_gauge_label(progress: f64) -> String {
    format!("{:>3}%", (progress * 100.0).round() as u16)
}

fn ascii_time(value: &str) -> Vec<String> {
    const GLYPH_WIDTH: usize = 10;
    let mut rows = vec![String::new(); 7];
    for ch in value.chars() {
        let glyph = ascii_glyph(ch);
        for (row, part) in rows.iter_mut().zip(glyph) {
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(&format!("{part:<GLYPH_WIDTH$}"));
        }
    }
    rows
}

fn ascii_glyph(ch: char) -> [&'static str; 7] {
    match ch {
        '0' => [
            " ad8888ba ",
            "d8\"    \"8b",
            "8P      88",
            "8b      d8",
            "8b      d8",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        '1' => [
            "   88    ",
            "  888    ",
            "   88    ",
            "   88    ",
            "   88    ",
            "   88    ",
            " 888888  ",
        ],
        '2' => [
            " ad8888ba ",
            "d8\"    \"8b",
            "      ,8P",
            "    ,d8P ",
            "  ,d8P'  ",
            " d8P'    ",
            "888888888",
        ],
        '3' => [
            " ad8888ba ",
            "d8\"    \"8b",
            "      ,8P",
            "   aad8\" ",
            "      `8b",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        '4' => [
            "88     88",
            "88     88",
            "88     88",
            "888888888",
            "       88",
            "       88",
            "       88",
        ],
        '5' => [
            "888888888",
            "88       ",
            "88       ",
            "888888ba ",
            "       8b",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        '6' => [
            " ad8888ba ",
            "d8\"      ",
            "88       ",
            "88PPPPba ",
            "88     8b",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        '7' => [
            "888888888",
            "      ,8P",
            "     ,8P ",
            "    ,8P  ",
            "   ,8P   ",
            "  ,8P    ",
            " ,8P     ",
        ],
        '8' => [
            " ad8888ba ",
            "d8\"    \"8b",
            "Y8a.  .a8P",
            "  Y8888P ",
            "d8\"    \"8b",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        '9' => [
            " ad8888ba ",
            "d8\"    \"8b",
            "Y8a.  .a8P",
            " \"Y888888",
            "       88",
            "Y8a.  .a8P",
            " \"Y8888Y\" ",
        ],
        ':' => [
            "         ",
            "   88    ",
            "   88    ",
            "         ",
            "   88    ",
            "   88    ",
            "         ",
        ],
        _ => [
            "         ",
            "         ",
            "         ",
            "         ",
            "         ",
            "         ",
            "         ",
        ],
    }
}

fn center_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn action_plan_lines(config: &Config, width: usize) -> Vec<Line<'static>> {
    let description_width = (width / 3).clamp(8, 32);
    let command_width = width.saturating_sub(description_width + 3).max(1);
    let mut lines = Vec::new();
    for (index, command) in build_action_commands(config).iter().enumerate() {
        let descriptions = wrap_command(
            &format!("{}. {}", index + 1, action_description(command, config)),
            description_width,
        );
        let commands = wrap_command(&format!("$ {}", command.display()), command_width);
        let dangerous = command.is_dangerous();
        let description_style = Style::default()
            .fg(if dangerous { Color::Red } else { Color::Cyan })
            .add_modifier(Modifier::BOLD);
        let command_style = Style::default().fg(if dangerous { Color::Red } else { Color::Green });
        for row in 0..descriptions.len().max(commands.len()) {
            let description = descriptions.get(row).map(String::as_str).unwrap_or("");
            let padding = description_width.saturating_sub(Line::from(description).width());
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{description}{}", " ".repeat(padding)),
                    description_style,
                ),
                Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    commands.get(row).cloned().unwrap_or_default(),
                    command_style,
                ),
            ]));
        }
    }
    lines
}

fn wrap_command(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for source in text.split('\n') {
        let mut current = String::new();
        for ch in source.chars() {
            if !current.is_empty() && Line::from(format!("{current}{ch}")).width() > width {
                lines.push(std::mem::take(&mut current));
            }
            current.push(ch);
        }
        lines.push(current);
    }
    lines
}

fn split_list(value: &str, separator: char) -> Vec<String> {
    value
        .split(separator)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::{
        DiscordConfig, PresenceActivity, countdown_presence, post_countdown_presence,
    };

    fn render_app(app: &mut App, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn action_plan_pairs_descriptions_and_shell_in_aligned_columns() {
        let mut config = quiet_config("25m");
        config.actions.workspace.enabled = true;
        config.actions.workspace.backend = "niri".to_string();
        config.actions.workspace.number = 5;
        for width in [34, 78, 104, 158] {
            let lines = action_plan_lines(&config, width);
            let first = &lines[0];
            assert!(first.spans[0].content.starts_with("1. Switch"));
            assert!(first.spans[2].content.starts_with("$ niri"));
            assert_ne!(first.spans[0].style.fg, first.spans[2].style.fg);
            assert!(lines.iter().all(|line| line.width() <= width));
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.spans[2].content.as_ref())
                    .collect::<String>(),
                "$ niri msg action focus-workspace 5"
            );
            assert!(
                lines
                    .iter()
                    .all(|line| line.spans[0].width() == first.spans[0].width())
            );
        }
    }

    #[test]
    fn tui_settings_and_edit_view_escape_configured_terminal_controls() {
        let message = "line one\nline two\u{1b}]0;title\u{7}\u{2066}syst\u{200d}emctl猫🙂";
        let mut config = quiet_config("25m");
        config.actions.notify.enabled = true;
        config.actions.notify.message = message.to_string();
        let mut app = App::new(PathBuf::from("/tmp/layout.yaml"), config);
        let field = *FIELDS
            .iter()
            .find(|field| field.key == FieldKey::NotifyMessage)
            .unwrap();
        app.group = field_group(field.key);
        app.focus = app
            .visible_fields()
            .iter()
            .position(|visible| visible.key == field.key)
            .unwrap();
        let assert_safe_render = |rendered: &str| {
            assert!(rendered.contains("\\n"));
            assert!(rendered.contains("u{1b}]0;title"));
            assert!(rendered.contains("\\u{7}"));
            assert!(rendered.contains("\\u{2066}"));
            assert!(rendered.contains("\\u{200d}"));
            assert!(rendered.contains('猫'));
            assert!(rendered.contains('🙂'));
            assert!(!rendered.contains('\u{1b}'));
            assert!(!rendered.contains('\u{7}'));
            assert!(!rendered.contains('\u{2066}'));
            assert!(!rendered.contains('\u{200d}'));
        };

        let rendered = render_app(&mut app, 160, 40);
        assert_safe_render(&rendered);

        app.edit = Some(EditState {
            field,
            value: message.to_string(),
            cursor: message.len(),
        });
        let editing_rendered = render_app(&mut app, 160, 40);
        assert_safe_render(&editing_rendered);
    }

    #[test]
    fn preview_wrapping_preserves_spaces_unicode_and_long_tokens() {
        let command = format!("$ printf 'a  b猫' {}", "x".repeat(140));
        let lines = wrap_command(&command, 20);
        assert_eq!(lines.concat(), command);
        assert!(
            lines
                .iter()
                .all(|line| Line::from(line.as_str()).width() <= 20)
        );
    }

    #[test]
    fn grouped_layouts_show_controls_and_scrollable_commands() {
        for (width, height) in [(36, 20), (80, 24), (106, 49), (160, 40)] {
            let mut app = App::new(PathBuf::from("/tmp/layout.yaml"), Config::default());
            for group in 0..GROUPS.len() {
                let text = render_app(&mut app, width, height);
                assert!(text.contains("not running"), "{width}x{height}\n{text}");
                assert!(text.contains("q quit"), "{width}x{height}\n{text}");
                assert!(
                    text.contains(&format!("{} settings", GROUPS[group])),
                    "{text}"
                );
                assert!(text.contains("plan / commands"), "{text}");
                assert!(!text.contains("0%"));
                app.change_group(1);
            }
            app.preview_offset = usize::MAX;
            render_app(&mut app, width, height);
            assert!(app.preview_offset < usize::MAX);
            // After a resize, scrolling still produces a valid viewport.
            render_app(&mut app, 36, 20);
        }
    }

    #[test]
    fn narrow_countdown_uses_compact_digits_and_keeps_controls() {
        let mut app = App::new(PathBuf::from("/tmp/layout.yaml"), quiet_config("25m"));
        let mut timer = dummy_timer_state();
        timer.remaining_ms.store(65_000, Ordering::Relaxed);
        timer.duration = Duration::from_secs(90);
        app.timer = Some(timer);
        let text = render_app(&mut app, 36, 20);
        assert!(text.contains("time left: 1m5s"), "{text}");
        assert!(text.contains("ends"), "{text}");
        assert!(text.contains("q quit"), "{text}");
        app.timer
            .as_ref()
            .unwrap()
            .pause
            .store(true, Ordering::Relaxed);
        let text = render_app(&mut app, 36, 20);
        assert!(text.contains("paused"));
        assert!(!text.contains("ends"));
    }

    #[test]
    fn invalid_duration_and_brightness_edits_leave_config_intact() {
        let mut app = App::new(PathBuf::from("/tmp/edit.yaml"), quiet_config("25m"));
        for (key, value) in [
            (FieldKey::Duration, "bogus"),
            (FieldKey::BrightnessValue, "101"),
        ] {
            let field = *FIELDS.iter().find(|field| field.key == key).unwrap();
            let before = app.field_value(&field);
            assert!(app.apply_edit(field, value.to_string()).is_err());
            assert_eq!(app.field_value(&field), before);
        }
    }

    #[test]
    fn unicode_edits_move_and_delete_at_character_boundaries() {
        let mut app = App::new(PathBuf::from("/tmp/edit.yaml"), quiet_config("25m"));
        let field = *FIELDS
            .iter()
            .find(|field| field.key == FieldKey::NotifyMessage)
            .unwrap();
        app.edit = Some(EditState {
            field,
            value: "a猫я".to_string(),
            cursor: "a猫я".len(),
        });
        app.handle_edit_key(KeyCode::Left);
        app.handle_edit_key(KeyCode::Backspace);
        app.handle_edit_key(KeyCode::Char('🙂'));
        assert_eq!(app.edit.as_ref().unwrap().value, "a🙂я");
        assert!(edit_view(app.edit.as_ref().unwrap(), 4).contains('▌'));
        app.handle_edit_key(KeyCode::Home);
        app.handle_edit_key(KeyCode::Delete);
        assert_eq!(app.edit.as_ref().unwrap().value, "🙂я");
        app.handle_edit_key(KeyCode::End);
        app.handle_edit_key(KeyCode::Enter);
        assert_eq!(app.config.actions.notify.message, "🙂я");
    }

    #[test]
    fn clock_targets_roll_over_midnight_and_presets_clear_target() {
        let zone = chrono::FixedOffset::east_opt(3600).unwrap();
        let now = zone.with_ymd_and_hms(2026, 12, 31, 23, 45, 0).unwrap();
        assert_eq!(
            duration_until(parse_clock_time("00:15").unwrap(), now).unwrap(),
            Duration::from_secs(1800)
        );
        assert_eq!(
            duration_until(parse_clock_time("23:45").unwrap(), now).unwrap(),
            Duration::from_secs(86400)
        );
        for value in ["25:00", "12:60", "12:30:00", "1:30", "hello"] {
            assert!(parse_clock_time(value).is_err());
        }
        let mut app = App::new(PathBuf::from("/tmp/edit.yaml"), quiet_config("25m"));
        let field = *FIELDS
            .iter()
            .find(|field| field.key == FieldKey::UntilTime)
            .unwrap();
        app.apply_edit(field, "23:30".to_string()).unwrap();
        assert_eq!(app.config.duration, "25m");
        app.set_preset("1h");
        assert!(app.until.is_none());
        assert_eq!(app.config.duration, "1h");
        assert!(app.timer.is_none());
    }

    #[test]
    #[ignore = "run with TZ=Europe/Bratislava to check local daylight saving transitions"]
    fn local_deadlines_handle_daylight_saving_transitions() {
        let spring = Local.with_ymd_and_hms(2026, 3, 29, 1, 30, 0).unwrap();
        assert!(duration_until(parse_clock_time("02:30").unwrap(), spring).is_err());
        assert_eq!(
            duration_until(parse_clock_time("03:30").unwrap(), spring).unwrap(),
            Duration::from_secs(3600)
        );
        let fall = chrono::Utc
            .with_ymd_and_hms(2026, 10, 25, 0, 45, 0)
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(
            duration_until(parse_clock_time("02:30").unwrap(), fall).unwrap(),
            Duration::from_secs(2700)
        );
    }

    fn quiet_config(duration: &str) -> Config {
        let mut config = Config {
            duration: duration.to_string(),
            ..Config::default()
        };
        config.actions.workspace.enabled = false;
        config.actions.media.enabled = false;
        config.actions.brightness.enabled = false;
        config.actions.mute.enabled = false;
        config.actions.mute_input.enabled = false;
        config.actions.lock.enabled = false;
        config.actions.monitor.enabled = false;
        config.actions.notify.enabled = false;
        config.actions.kill.enabled = false;
        config.actions.power.mode = "none".to_string();
        config.actions.custom.enabled = false;
        config
    }

    #[test]
    fn failed_edit_keeps_input_for_retry() {
        let mut app = App::new(
            PathBuf::from("/tmp/gosleep-timer-test.yaml"),
            quiet_config("25m"),
        );
        app.edit = Some(EditState {
            field: Field {
                label: "workspace number",
                key: FieldKey::WorkspaceNumber,
                kind: FieldKind::Int,
            },
            value: "not-a-number".to_string(),
            cursor: 12,
        });

        app.handle_edit_key(KeyCode::Enter);

        assert_eq!(app.edit.as_ref().expect("edit kept").value, "not-a-number");
        assert!(app.status.starts_with("edit failed:"));
    }

    #[test]
    fn successful_edit_clears_input() {
        let mut app = App::new(
            PathBuf::from("/tmp/gosleep-timer-test.yaml"),
            quiet_config("25m"),
        );
        app.edit = Some(EditState {
            field: Field {
                label: "workspace number",
                key: FieldKey::WorkspaceNumber,
                kind: FieldKind::Int,
            },
            value: "5".to_string(),
            cursor: 1,
        });

        app.handle_edit_key(KeyCode::Enter);

        assert!(app.edit.is_none());
        assert_eq!(app.config.actions.workspace.number, 5);
    }

    #[test]
    fn start_timer_rejects_invalid_config_without_spawning() {
        let mut config = quiet_config("25m");
        config.actions.power.mode = "bogus".to_string();
        let mut app = App::new(PathBuf::from("/tmp/gosleep-timer-test.yaml"), config);

        app.start_timer();

        assert!(app.timer.is_none());
        assert!(app.status.starts_with("timer failed:"));
    }

    #[test]
    fn wrap_command_keeps_lines_under_width() {
        let lines = wrap_command(
            "if command -v niri >/dev/null 2>&1; then niri msg action focus-workspace 3",
            24,
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 24));
    }

    #[test]
    fn split_list_trims_empty_values() {
        assert_eq!(
            split_list("firefox, telegram-desktop, ", ','),
            vec!["firefox", "telegram-desktop"]
        );
    }

    #[test]
    fn timer_header_text_compacts_for_narrow_widths() {
        let remaining = Duration::from_secs(65);

        assert_eq!(format_time_left_text(None, remaining, 20), "not running");
        assert_eq!(
            format_time_left_text(Some(&dummy_timer_state()), remaining, 40),
            "time left: 1m5s"
        );
        assert_eq!(
            format_time_left_text(Some(&dummy_timer_state()), remaining, 12),
            "left: 01:05"
        );
        assert_eq!(
            format_time_left_text(Some(&dummy_timer_state()), remaining, 5),
            "01:05"
        );
    }

    #[test]
    fn gauge_label_is_short_percent_only() {
        assert_eq!(format_gauge_label(0.0), "  0%");
        assert_eq!(format_gauge_label(0.58), " 58%");
        assert_eq!(format_gauge_label(1.0), "100%");
    }

    #[test]
    fn visible_fields_hide_disabled_action_settings() {
        let mut config = quiet_config("25m");

        let keys = visible_field_keys(&config);
        assert!(keys.contains(&FieldKey::Duration));
        assert!(keys.contains(&FieldKey::InhibitSleep));
        assert!(keys.contains(&FieldKey::MediaEnabled));
        assert!(keys.contains(&FieldKey::MonitorEnabled));
        assert!(keys.contains(&FieldKey::NotifyEnabled));
        assert!(!keys.contains(&FieldKey::MediaAction));
        assert!(!keys.contains(&FieldKey::WorkspaceBackend));
        assert!(!keys.contains(&FieldKey::WorkspaceNumber));
        assert!(!keys.contains(&FieldKey::WorkspaceCommand));
        assert!(!keys.contains(&FieldKey::BrightnessValue));
        assert!(!keys.contains(&FieldKey::LockBackend));
        assert!(!keys.contains(&FieldKey::LockCommand));
        assert!(!keys.contains(&FieldKey::MonitorBackend));
        assert!(!keys.contains(&FieldKey::MonitorCommand));
        assert!(!keys.contains(&FieldKey::NotifyMessage));
        assert!(!keys.contains(&FieldKey::KillProcesses));
        assert!(!keys.contains(&FieldKey::CustomCommands));

        config.actions.media.enabled = true;
        config.actions.workspace.enabled = true;
        config.actions.brightness.enabled = true;
        config.actions.lock.enabled = true;
        config.actions.monitor.enabled = true;
        config.actions.notify.enabled = true;
        config.actions.kill.enabled = true;
        config.actions.custom.enabled = true;

        let keys = visible_field_keys(&config);
        assert!(keys.contains(&FieldKey::MediaAction));
        assert!(keys.contains(&FieldKey::WorkspaceBackend));
        assert!(keys.contains(&FieldKey::WorkspaceNumber));
        assert!(keys.contains(&FieldKey::WorkspaceCommand));
        assert!(keys.contains(&FieldKey::BrightnessValue));
        assert!(keys.contains(&FieldKey::LockBackend));
        assert!(keys.contains(&FieldKey::LockCommand));
        assert!(keys.contains(&FieldKey::MonitorBackend));
        assert!(keys.contains(&FieldKey::MonitorCommand));
        assert!(keys.contains(&FieldKey::NotifyMessage));
        assert!(keys.contains(&FieldKey::KillProcesses));
        assert!(keys.contains(&FieldKey::CustomCommands));
    }

    #[test]
    fn ascii_time_rows_have_equal_width() {
        let rows = ascii_time("14:45:27");
        let width = rows.first().map(String::len).unwrap_or(0);

        assert_eq!(rows.len(), 7);
        assert!(width > 0);
        assert!(rows.iter().all(|row| row.len() == width));
    }

    #[test]
    fn discord_fields_edit_and_apply_through_tui() {
        let mut app = App::new(PathBuf::from("/tmp/discord.yaml"), Config::default());
        let keys = visible_field_keys(&app.config);
        assert!(keys.contains(&FieldKey::DiscordEnabled));
        assert!(keys.contains(&FieldKey::DiscordApplicationId));
        assert_eq!(field_group(FieldKey::DiscordEnabled), 4);
        assert_eq!(field_group(FieldKey::DiscordApplicationId), 4);

        app.toggle_bool(FieldKey::DiscordEnabled);
        assert!(!app.config.discord.enabled);
        assert!(!app.config.discord.should_connect());
        app.toggle_bool(FieldKey::DiscordEnabled);
        assert!(app.config.discord.enabled);

        let field = *FIELDS
            .iter()
            .find(|field| field.key == FieldKey::DiscordApplicationId)
            .unwrap();
        app.apply_edit(field, "  123456  ".to_string()).unwrap();
        assert_eq!(app.config.discord.application_id, "123456");
        assert!(app.config.discord.should_connect());
        assert_eq!(app.field_value(&field), "123456");

        let round_trip = noyalib::to_string(&app.config).unwrap();
        let loaded: Config = noyalib::from_str(&round_trip).unwrap();
        assert!(loaded.discord.enabled);
        assert_eq!(loaded.discord.application_id, "123456");
    }

    #[test]
    fn discord_missing_client_does_not_fail_or_stall_timer() {
        // No Discord IPC socket exists in the test sandbox; the worker must
        // swallow the connection failure in the background.
        let mut config = quiet_config("10ms");
        config.discord.enabled = true;
        config.discord.application_id = "123456789012345678".to_string();
        assert!(config.discord.should_connect());

        let started = Instant::now();
        let presence = DiscordPresence::start(&config.discord);
        assert!(presence.is_active());
        presence.update(PresenceActivity {
            details: "Sleep timer".to_string(),
            state: String::new(),
            start_timestamp: None,
            end_timestamp: None,
        });
        presence.clear();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "presence start/update/clear must not block, got {:?}",
            started.elapsed()
        );
        drop(presence);

        let started = Instant::now();
        let (report, _presence) = run_timer(
            &config,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions {
                no_actions: true,
                ..TimerOptions::default()
            },
            None,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(report.failures.is_empty());
    }

    #[test]
    fn stopped_run_clears_presence_without_stalling() {
        let mut config = quiet_config("10ms");
        config.discord.enabled = true;
        config.discord.application_id = "123456789012345678".to_string();
        let stop = Arc::new(AtomicBool::new(true));
        let started = Instant::now();
        let err = run_timer(
            &config,
            &stop,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions {
                no_actions: true,
                ..TimerOptions::default()
            },
            None,
        )
        .unwrap_err();
        assert_eq!(err.to_string(), TIMER_STOPPED_MSG);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "stopped run with Discord configured must not stall, got {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn successful_runs_transition_to_post_countdown_instead_of_clearing() {
        let finished = post_countdown_presence();
        assert_eq!(finished.details, "Sleep timer");
        assert_eq!(finished.state, "finished");
        assert_eq!(finished.start_timestamp, None);
        assert_eq!(finished.end_timestamp, None);

        // Disarmed guards retain the post-countdown state (no drop-clear).
        let mut presence = DiscordPresence::start(&DiscordConfig {
            enabled: true,
            application_id: "123456789012345678".to_string(),
        });
        assert!(presence.is_active());
        presence.update(countdown_presence(Duration::from_secs(60), "none", 1000));
        presence.update(post_countdown_presence());
        presence.disarm();
        drop(presence);

        // CLI and TUI runs share `run_timer`, so countdown publishing covers
        // --no-actions and dry-run behavior on both paths. These runs have no
        // enabled actions, so they keep the accurate `finished` state instead
        // of claiming a sleep interval (US-004).
        for (mode, options) in [
            (
                RunMode::Cli,
                TimerOptions {
                    no_actions: true,
                    ..TimerOptions::default()
                },
            ),
            (
                RunMode::Cli,
                TimerOptions {
                    dry_run: true,
                    ..TimerOptions::default()
                },
            ),
            (RunMode::Tui, TimerOptions::default()),
        ] {
            let mut config = quiet_config("10ms");
            config.discord.enabled = true;
            config.discord.application_id = "123456789012345678".to_string();
            let started = Instant::now();
            let (report, presence) = run_timer(
                &config,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicU64::new(0)),
                mode,
                options,
                None,
            )
            .unwrap();
            assert!(started.elapsed() < Duration::from_secs(10));
            assert!(report.failures.is_empty());
            // Returned guard keeps the IPC connection open for retention.
            assert!(presence.is_active());
        }
    }

    #[test]
    fn successful_real_actions_return_sleeping_guard_while_others_stay_finished() {
        // Real successful action run transitions to the sleeping state and
        // returns a disarmed, still-active guard for retention.
        let mut sleep_config = quiet_config("10ms");
        sleep_config.discord.enabled = true;
        sleep_config.discord.application_id = "123456789012345678".to_string();
        sleep_config.actions.custom.enabled = true;
        sleep_config.actions.custom.commands = vec!["true".to_string()];
        let (report, presence) = run_timer(
            &sleep_config,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions::default(),
            None,
        )
        .unwrap();
        assert!(report.failures.is_empty());
        assert!(!report.actions.is_empty());
        assert!(presence.is_active());
        drop(presence);

        // Failed actions must not claim a sleep interval.
        let mut failed_config = quiet_config("10ms");
        failed_config.discord.enabled = true;
        failed_config.discord.application_id = "123456789012345678".to_string();
        failed_config.actions.custom.enabled = true;
        failed_config.actions.custom.commands = vec!["false".to_string()];
        let (failed_report, failed_presence) = run_timer(
            &failed_config,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(failed_report.failures.len(), 1);
        assert!(failed_presence.is_active());
        drop(failed_presence);

        // --no-actions and dry-run never claim a sleep interval, even with
        // enabled actions configured.
        for options in [
            TimerOptions {
                no_actions: true,
                ..TimerOptions::default()
            },
            TimerOptions {
                dry_run: true,
                ..TimerOptions::default()
            },
        ] {
            let mut config = quiet_config("10ms");
            config.discord.enabled = true;
            config.discord.application_id = "123456789012345678".to_string();
            config.actions.custom.enabled = true;
            config.actions.custom.commands = vec!["true".to_string()];
            let (report, presence) = run_timer(
                &config,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicU64::new(0)),
                RunMode::Cli,
                options,
                None,
            )
            .unwrap();
            assert!(report.failures.is_empty());
            assert!(presence.is_active());
            drop(presence);
        }
    }

    #[test]
    fn tui_retains_sleeping_guard_until_stop_and_clears_on_stop() {
        // Completion stores the disarmed guard on the App for retention while
        // the TUI stays open; stopping clears it explicitly.
        let mut app = App::new(PathBuf::from("/tmp/sleep-retain.yaml"), quiet_config("25m"));
        assert!(app.sleep_presence.is_none());
        let guard = DiscordPresence::start(&DiscordConfig {
            enabled: true,
            application_id: "123456789012345678".to_string(),
        });
        assert!(guard.is_active());
        app.apply_timer_result(Ok(Ok((
            TimerReport {
                started_at: 0,
                duration: Duration::from_secs(60),
                sleep_started_at: Some(1000),
                actions: vec!["true".to_string()],
                failures: Vec::new(),
                dry_run: false,
                no_actions: false,
            },
            guard,
        ))));
        assert!(app.sleep_presence.is_some());
        assert!(app.sleep_presence.as_ref().is_some_and(|p| p.is_active()));
        // `stop_timer` with no running timer clears the retained guard.
        app.stop_timer();
        assert!(app.sleep_presence.is_none());
        assert_eq!(app.status, "timer stopped");
    }

    fn visible_field_keys(config: &Config) -> Vec<FieldKey> {
        visible_fields(config)
            .into_iter()
            .map(|field| field.key)
            .collect()
    }

    fn dummy_timer_state() -> TimerState {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(|| {
            Ok((
                TimerReport {
                    started_at: 0,
                    duration: Duration::from_secs(1),
                    sleep_started_at: None,
                    actions: Vec::new(),
                    failures: Vec::new(),
                    dry_run: false,
                    no_actions: false,
                },
                DiscordPresence::inactive(),
            ))
        });
        let (_, updates) = mpsc::channel();
        TimerState {
            updates,
            duration: Duration::from_secs(1),
            stop,
            pause: Arc::new(AtomicBool::new(false)),
            remaining_ms: Arc::new(AtomicU64::new(1000)),
            handle,
        }
    }
}
