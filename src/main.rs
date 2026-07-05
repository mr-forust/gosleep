use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    duration: String,
    actions: Actions,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct Actions {
    workspace: WorkspaceAction,
    media: MediaAction,
    brightness: BrightnessAction,
    mute: ToggleAction,
    lock: LockAction,
    kill: KillAction,
    power: PowerAction,
    custom: CustomAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct WorkspaceAction {
    enabled: bool,
    backend: String,
    number: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct MediaAction {
    enabled: bool,
    action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct BrightnessAction {
    enabled: bool,
    value: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct ToggleAction {
    enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct LockAction {
    enabled: bool,
    command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct KillAction {
    enabled: bool,
    processes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PowerAction {
    mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct CustomAction {
    enabled: bool,
    commands: Vec<String>,
}

#[derive(Debug, Clone)]
struct ActionCommand {
    label: &'static str,
    args: Vec<String>,
    shell: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    started_at: u64,
    duration_seconds: u64,
    slept_seconds: Option<u64>,
    status: String,
    actions: Vec<String>,
    failures: Vec<String>,
    dry_run: bool,
    no_actions: bool,
}

#[derive(Debug, Clone)]
struct TimerReport {
    started_at: u64,
    duration: Duration,
    actions: Vec<String>,
    failures: Vec<String>,
    dry_run: bool,
    no_actions: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct TimerOptions {
    dry_run: bool,
    no_actions: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration: "25m".to_string(),
            actions: Actions::default(),
        }
    }
}

impl Default for WorkspaceAction {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "auto".to_string(),
            number: 3,
        }
    }
}

impl Default for MediaAction {
    fn default() -> Self {
        Self {
            enabled: true,
            action: "stop".to_string(),
        }
    }
}

impl Default for BrightnessAction {
    fn default() -> Self {
        Self {
            enabled: false,
            value: 30,
        }
    }
}

impl Default for LockAction {
    fn default() -> Self {
        Self {
            enabled: false,
            command: "loginctl lock-session".to_string(),
        }
    }
}

impl Default for PowerAction {
    fn default() -> Self {
        Self {
            mode: "none".to_string(),
        }
    }
}

impl ActionCommand {
    fn display(&self) -> String {
        self.shell.clone().unwrap_or_else(|| self.args.join(" "))
    }

    fn is_dangerous(&self) -> bool {
        if matches!(self.label, "kill" | "power") {
            return true;
        }
        let command = self.display().to_lowercase();
        [
            "rm -rf",
            "mkfs",
            "dd if=",
            "shutdown",
            "reboot",
            "poweroff",
            "systemctl poweroff",
            "systemctl reboot",
            "wipefs",
            "shred",
            "killall",
            "pkill",
        ]
        .iter()
        .any(|needle| command.contains(needle))
    }
}

impl TimerReport {
    fn to_history(&self, status: &str, slept_seconds: Option<u64>) -> HistoryEntry {
        HistoryEntry {
            started_at: self.started_at,
            duration_seconds: self.duration.as_secs(),
            slept_seconds,
            status: status.to_string(),
            actions: self.actions.clone(),
            failures: self.failures.clone(),
            dry_run: self.dry_run,
            no_actions: self.no_actions,
        }
    }
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
            let report = run_timer(
                &config,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(AtomicU64::new(0)),
                RunMode::Cli,
                options,
                Some(&mut stdout),
            )?;
            let slept_seconds = prompt_wake_confirmation(&config, options)?;
            append_history(
                &history_path(&path),
                report.to_history("finished", slept_seconds),
            )
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

fn default_config_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("gosleep-timer").join("config.yaml");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("gosleep-timer")
        .join("config.yaml")
}

fn ensure_config(path: &Path) -> Result<Config> {
    if !path.exists() {
        let config = Config::default();
        save_config(path, &config)?;
        return Ok(config);
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut config: Config = serde_yaml::from_str(&data).context("parse config")?;
    normalize_config(&mut config);
    validate_config(&config)?;
    Ok(config)
}

fn save_config(path: &Path, config: &Config) -> Result<()> {
    let mut config = config.clone();
    normalize_config(&mut config);
    validate_config(&config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = serde_yaml::to_string(&config).context("serialize config")?;
    fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn history_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("history.yaml")
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_history(path: &Path) -> Result<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_yaml::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

fn append_history(path: &Path, entry: HistoryEntry) -> Result<()> {
    let mut entries = read_history(path)?;
    entries.push(entry);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = serde_yaml::to_string(&entries).context("serialize history")?;
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
    let status = Command::new(editor)
        .arg(path)
        .status()
        .context("open editor")?;
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

fn normalize_config(config: &mut Config) {
    if config.duration.is_empty() {
        config.duration = "25m".to_string();
    }
    if config.actions.workspace.backend.is_empty() {
        config.actions.workspace.backend = "auto".to_string();
    }
    if config.actions.workspace.number == 0 {
        config.actions.workspace.number = 1;
    }
    if config.actions.media.action.is_empty() {
        config.actions.media.action = "stop".to_string();
    }
    if config.actions.brightness.value == 0 {
        config.actions.brightness.value = 30;
    }
    if config.actions.brightness.value > 100 {
        config.actions.brightness.value = 100;
    }
    if config.actions.lock.command.is_empty() {
        config.actions.lock.command = "loginctl lock-session".to_string();
    }
    if config.actions.power.mode.is_empty() {
        config.actions.power.mode = "none".to_string();
    }
}

fn validate_config(config: &Config) -> Result<()> {
    parse_duration(&config.duration)?;
    match config.actions.power.mode.as_str() {
        "none" | "poweroff" | "reboot" => {}
        mode => bail!("invalid power mode {mode:?}"),
    }
    match config.actions.workspace.backend.as_str() {
        "auto" | "niri" | "hyprland" | "kde" => {}
        backend => bail!("invalid workspace backend {backend:?}"),
    }
    match config.actions.media.action.as_str() {
        "none" | "stop" | "pause" | "play-pause" | "next" | "previous" => {}
        action => bail!("invalid media action {action:?}"),
    }
    Ok(())
}

fn parse_duration(value: &str) -> Result<Duration> {
    if value.is_empty() {
        bail!("duration is required");
    }

    let mut total = 0.0_f64;
    let chars: Vec<char> = value.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
            index += 1;
        }
        if start == index {
            bail!("invalid duration {value:?}");
        }
        let number: f64 = chars[start..index].iter().collect::<String>().parse()?;

        let unit_start = index;
        while index < chars.len() && !chars[index].is_ascii_digit() && chars[index] != '.' {
            index += 1;
        }
        let unit: String = chars[unit_start..index].iter().collect();
        let multiplier = match unit.as_str() {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 0.001,
            "us" | "µs" => 0.000001,
            "ns" => 0.000000001,
            _ => bail!("invalid duration unit {unit:?}"),
        };
        total += number * multiplier;
    }

    if total <= 0.0 {
        bail!("duration must be greater than zero");
    }
    Ok(Duration::from_secs_f64(total))
}

fn build_action_commands(config: &Config) -> Vec<ActionCommand> {
    let mut config = config.clone();
    normalize_config(&mut config);
    let actions = &config.actions;
    let mut commands = Vec::new();

    if actions.workspace.enabled {
        commands.push(workspace_command(&actions.workspace));
    }
    if actions.media.enabled && actions.media.action != "none" {
        commands.push(ActionCommand {
            label: "media",
            args: vec!["playerctl".to_string(), actions.media.action.clone()],
            shell: None,
        });
    }
    if actions.brightness.enabled {
        let value = actions.brightness.value;
        commands.push(ActionCommand {
            label: "brightness",
            args: vec![],
            shell: Some(format!("brightnessctl set {value}% || light -S {value}")),
        });
    }
    if actions.mute.enabled {
        commands.push(ActionCommand {
            label: "mute",
            args: vec![],
            shell: Some(
                "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle || pactl set-sink-mute @DEFAULT_SINK@ toggle".to_string(),
            ),
        });
    }
    if actions.lock.enabled {
        commands.push(ActionCommand {
            label: "lock",
            args: vec![],
            shell: Some(actions.lock.command.clone()),
        });
    }
    if actions.kill.enabled {
        for process in &actions.kill.processes {
            let process = process.trim();
            if !process.is_empty() {
                commands.push(ActionCommand {
                    label: "kill",
                    args: vec!["pkill".to_string(), process.to_string()],
                    shell: None,
                });
            }
        }
    }
    match actions.power.mode.as_str() {
        "poweroff" => commands.push(ActionCommand {
            label: "power",
            args: vec!["systemctl".to_string(), "poweroff".to_string()],
            shell: None,
        }),
        "reboot" => commands.push(ActionCommand {
            label: "power",
            args: vec!["systemctl".to_string(), "reboot".to_string()],
            shell: None,
        }),
        _ => {}
    }
    if actions.custom.enabled {
        for command in &actions.custom.commands {
            let command = command.trim();
            if !command.is_empty() {
                commands.push(ActionCommand {
                    label: "custom",
                    args: vec![],
                    shell: Some(command.to_string()),
                });
            }
        }
    }
    commands
}

fn workspace_command(action: &WorkspaceAction) -> ActionCommand {
    let number = action.number.to_string();
    match action.backend.as_str() {
        "niri" => ActionCommand {
            label: "workspace",
            args: vec!["niri", "msg", "action", "focus-workspace", &number]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "hyprland" => ActionCommand {
            label: "workspace",
            args: vec!["hyprctl", "dispatch", "workspace", &number]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "kde" => ActionCommand {
            label: "workspace",
            args: vec![
                "qdbus",
                "org.kde.KWin",
                "/KWin",
                "org.kde.KWin.setCurrentDesktop",
                &number,
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            shell: None,
        },
        _ => ActionCommand {
            label: "workspace",
            args: vec![],
            shell: Some(format!(
                "if command -v niri >/dev/null 2>&1; then niri msg action focus-workspace {0}; elif command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch workspace {0}; elif command -v qdbus >/dev/null 2>&1; then qdbus org.kde.KWin /KWin org.kde.KWin.setCurrentDesktop {0}; fi",
                number
            )),
        },
    }
}

fn preview_commands(config: &Config) -> Vec<String> {
    build_action_commands(config)
        .iter()
        .map(ActionCommand::display)
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunMode {
    Cli,
    Tui,
}

fn write_timer_line(output: &mut dyn Write, message: &str) {
    writeln!(output, "{message}").ok();
}

fn write_timer_progress(output: &mut dyn Write, message: &str) {
    write!(output, "\r{message}").ok();
    output.flush().ok();
}

fn run_timer(
    config: &Config,
    stop: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
    remaining_ms: &Arc<AtomicU64>,
    mode: RunMode,
    options: TimerOptions,
    mut output: Option<&mut dyn Write>,
) -> Result<TimerReport> {
    validate_config(config)?;
    let duration = parse_duration(&config.duration)?;
    let started_at = unix_timestamp();
    let started = Instant::now();
    let mut remaining = duration;
    let mut last_tick = started;
    remaining_ms.store(
        duration.as_millis().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
    if let Some(output) = output.as_deref_mut() {
        write_timer_line(output, &format!("timer started: {}", config.duration));
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            if let Some(output) = output.as_deref_mut() {
                write_timer_line(output, "stopped");
            }
            bail!("timer stopped");
        }

        if pause.load(Ordering::Relaxed) {
            last_tick = Instant::now();
            thread::sleep(Duration::from_millis(250));
            continue;
        }

        let now = Instant::now();
        let elapsed = now.saturating_duration_since(last_tick);
        last_tick = now;
        remaining = remaining.saturating_sub(elapsed);
        remaining_ms.store(
            remaining.as_millis().min(u128::from(u64::MAX)) as u64,
            Ordering::Relaxed,
        );

        if remaining.is_zero() {
            break;
        }
        if let Some(output) = output.as_deref_mut() {
            write_timer_progress(
                output,
                &format!("remaining: {}", format_duration(remaining)),
            );
        }
        thread::sleep(Duration::from_millis(250));
    }
    remaining_ms.store(0, Ordering::Relaxed);

    if stop.load(Ordering::Relaxed) {
        if let Some(output) = output.as_deref_mut() {
            write_timer_line(output, "stopped");
        }
        bail!("timer stopped");
    }

    if let Some(output) = output.as_deref_mut() {
        write_timer_line(output, "finished");
    }
    let mut failures = Vec::new();
    let actions = if options.no_actions {
        Vec::new()
    } else {
        build_action_commands(config)
    };
    for command in &actions {
        if let Some(output) = output.as_deref_mut() {
            let prefix = if options.dry_run { "would run" } else { "run" };
            write_timer_line(output, &format!("{prefix}: {}", command.display()));
        }
        if options.dry_run {
            continue;
        }
        if let Err(err) = run_command(command, mode) {
            failures.push(format!("{} failed: {err}", command.label));
        }
    }
    let report = TimerReport {
        started_at,
        duration,
        actions: actions.iter().map(ActionCommand::display).collect(),
        failures: failures.clone(),
        dry_run: options.dry_run,
        no_actions: options.no_actions,
    };
    if !failures.is_empty() {
        bail!(
            "{} action(s) failed: {}",
            failures.len(),
            failures.join("; ")
        );
    }
    Ok(report)
}

fn prepare_command_stdio(process: &mut Command, mode: RunMode) {
    if mode == RunMode::Tui {
        process.stdout(Stdio::null()).stderr(Stdio::null());
    }
}

fn run_command(command: &ActionCommand, mode: RunMode) -> Result<()> {
    let status = if let Some(shell) = &command.shell {
        let mut process = Command::new("sh");
        process.arg("-c").arg(shell);
        prepare_command_stdio(&mut process, mode);
        process.status()?
    } else {
        let Some(program) = command.args.first() else {
            return Ok(());
        };
        let mut process = Command::new(program);
        process.args(&command.args[1..]);
        prepare_command_stdio(&mut process, mode);
        process.status()?
    };
    if !status.success() {
        bail!("command exited with {status}");
    }
    Ok(())
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

struct App {
    path: PathBuf,
    config: Config,
    focus: usize,
    offset: usize,
    status: String,
    timer: Option<TimerState>,
    edit: Option<EditState>,
}

struct TimerState {
    duration: Duration,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    remaining_ms: Arc<AtomicU64>,
    handle: thread::JoinHandle<Result<TimerReport>>,
}

struct EditState {
    field: Field,
    value: String,
}

impl App {
    fn new(path: PathBuf, config: Config) -> Self {
        Self {
            status: format!("loaded {}", path.display()),
            path,
            config,
            focus: 0,
            offset: 0,
            timer: None,
            edit: None,
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
                if self.edit.is_some() {
                    self.handle_edit_key(key.code);
                } else {
                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.stop_timer();
                            return Ok(());
                        }
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
                        KeyCode::Char(' ') | KeyCode::Enter => self.activate()?,
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
        if area.width < 36 || area.height < 11 {
            frame.render_widget(Paragraph::new("terminal too small"), area);
            return;
        }
        if self.timer.is_some() {
            self.draw_running(frame, area);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(5),
                Constraint::Length(1),
            ])
            .split(area);

        self.draw_header(frame, chunks[0]);
        self.draw_body(frame, chunks[1]);
        let footer = if let Some(edit) = &self.edit {
            format!(
                "editing {}: {} | enter apply | esc cancel",
                edit.field.label, edit.value
            )
        } else {
            "space/enter edit-toggle | r run | p pause | x stop | s save | q quit".to_string()
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
            chunks[2],
        );
    }

    fn draw_running(&self, frame: &mut Frame, area: Rect) {
        let (_, remaining, progress) = self.timer_snapshot();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(9),
                Constraint::Min(1),
                Constraint::Length(2),
                Constraint::Length(8),
                Constraint::Length(1),
            ])
            .split(area);

        let time_lines = ascii_time(&format_compact_duration(remaining))
            .into_iter()
            .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
            .collect::<Vec<_>>();
        let time_width = time_lines
            .iter()
            .map(|line| line.width() as u16)
            .max()
            .unwrap_or(0);
        let timer_area = center_rect(time_width.min(chunks[1].width), 7, chunks[1]);
        frame.render_widget(Paragraph::new(time_lines), timer_area);

        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Magenta))
                .label(format_gauge_label(progress))
                .ratio(progress),
            chunks[3],
        );

        let commands = build_action_commands(&self.config);
        let lines = if commands.is_empty() {
            vec![Line::from("no post-timer actions enabled")]
        } else {
            commands
                .iter()
                .flat_map(|command| {
                    let style = if command.is_dangerous() {
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Green)
                    };
                    wrap_command(
                        &command.display(),
                        chunks[4].width.saturating_sub(4) as usize,
                    )
                    .into_iter()
                    .map(move |line| Line::from(Span::styled(line, style)))
                })
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .title(" commands after countdown ")
                        .borders(Borders::ALL)
                        .border_style(Color::Blue),
                )
                .wrap(Wrap { trim: false }),
            chunks[4],
        );

        let footer = if self
            .timer
            .as_ref()
            .is_some_and(|timer| timer.pause.load(Ordering::Relaxed))
        {
            "paused | p resume | x stop | q quit"
        } else {
            "running | p pause | x stop | q quit"
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
            chunks[5],
        );
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let (_, remaining, progress) = self.timer_snapshot();
        let header = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(area);
        frame.render_widget(
            Paragraph::new("gosleep-timer").style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            header[0],
        );
        frame.render_widget(
            Paragraph::new(format!("config: {}", self.path.display()))
                .style(Style::default().fg(Color::DarkGray)),
            header[1],
        );
        frame.render_widget(
            Paragraph::new(self.status.clone()).style(Style::default().fg(Color::Yellow)),
            header[2],
        );
        frame.render_widget(
            Paragraph::new(format_time_left_text(
                self.timer.as_ref(),
                remaining,
                header[3].width,
            ))
            .style(Style::default().fg(Color::White)),
            header[3],
        );
        frame.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Magenta))
                .label(format_gauge_label(progress))
                .ratio(progress),
            header[4],
        );
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        if area.width >= 118 {
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
                .split(area);
            self.draw_fields(frame, chunks[0]);
            self.draw_preview(frame, chunks[1]);
        } else {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(8)])
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
                let line = format!("{marker} {:<20} {}", field.label, self.field_value(field));
                let style = if selected {
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
                    .title(" settings ")
                    .borders(Borders::ALL)
                    .border_style(Color::Blue),
            ),
            area,
        );
    }

    fn draw_preview(&self, frame: &mut Frame, area: Rect) {
        let commands = build_action_commands(&self.config);
        let lines = commands
            .iter()
            .flat_map(|command| {
                let style = if command.is_dangerous() {
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Green)
                };
                wrap_command(&command.display(), area.width.saturating_sub(4) as usize)
                    .into_iter()
                    .map(move |line| Line::from(Span::styled(line, style)))
            })
            .collect::<Vec<_>>();
        let lines = if lines.is_empty() {
            vec![Line::from("no post-timer actions enabled")]
        } else {
            lines
        };
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .title(" command preview ")
                        .borders(Borders::ALL)
                        .border_style(Color::Blue),
                )
                .wrap(Wrap { trim: false }),
            area,
        );
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
                if let Some(edit) = self.edit.take() {
                    if let Err(err) = self.apply_edit(edit.field, edit.value) {
                        self.status = format!("edit failed: {err}");
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(edit) = &mut self.edit {
                    edit.value.pop();
                }
            }
            KeyCode::Char(value) => {
                if let Some(edit) = &mut self.edit {
                    edit.value.push(value);
                }
            }
            _ => {}
        }
    }

    fn apply_edit(&mut self, field: Field, value: String) -> Result<()> {
        match field.kind {
            FieldKind::Edit => match field.key {
                FieldKey::Duration => self.config.duration = value,
                FieldKey::LockCommand => self.config.actions.lock.command = value,
                _ => {}
            },
            FieldKind::Int => {
                let number: u32 = value.trim().parse().context("expected integer")?;
                match field.key {
                    FieldKey::WorkspaceNumber => self.config.actions.workspace.number = number,
                    FieldKey::BrightnessValue => {
                        self.config.actions.brightness.value = number.min(100) as u8
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
        let duration = match parse_duration(&self.config.duration) {
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
        let config = self.config.clone();
        let handle = thread::spawn(move || {
            run_timer(
                &config,
                &thread_stop,
                &thread_pause,
                &thread_remaining_ms,
                RunMode::Tui,
                TimerOptions::default(),
                None,
            )
        });
        self.timer = Some(TimerState {
            duration,
            stop,
            pause,
            remaining_ms,
            handle,
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
        if let Some(timer) = self.timer.take() {
            timer.stop.store(true, Ordering::Relaxed);
            self.apply_timer_result(timer.handle.join());
        } else {
            self.status = "timer stopped".to_string();
        }
    }

    fn reap_timer(&mut self) {
        if self
            .timer
            .as_ref()
            .is_some_and(|timer| timer.handle.is_finished())
        {
            let timer = self.timer.take().expect("timer exists");
            self.apply_timer_result(timer.handle.join());
        }
    }

    fn apply_timer_result(&mut self, result: thread::Result<Result<TimerReport>>) {
        match result {
            Ok(Ok(report)) => {
                if let Err(err) = append_history(
                    &history_path(&self.path),
                    report.to_history("finished", None),
                ) {
                    self.status = format!("history failed: {err}");
                } else {
                    self.status = "timer finished".to_string();
                }
            }
            Ok(Err(err)) if err.to_string() == "timer stopped" => {
                self.status = "timer stopped".to_string()
            }
            Ok(Err(err)) => self.status = format!("timer failed: {err}"),
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
            FieldKey::WorkspaceEnabled => on_off(self.config.actions.workspace.enabled),
            FieldKey::WorkspaceBackend => self.config.actions.workspace.backend.clone(),
            FieldKey::WorkspaceNumber => self.config.actions.workspace.number.to_string(),
            FieldKey::MediaEnabled => on_off(self.config.actions.media.enabled),
            FieldKey::MediaAction => self.config.actions.media.action.clone(),
            FieldKey::BrightnessEnabled => on_off(self.config.actions.brightness.enabled),
            FieldKey::BrightnessValue => self.config.actions.brightness.value.to_string(),
            FieldKey::MuteEnabled => on_off(self.config.actions.mute.enabled),
            FieldKey::LockEnabled => on_off(self.config.actions.lock.enabled),
            FieldKey::LockCommand => self.config.actions.lock.command.clone(),
            FieldKey::KillEnabled => on_off(self.config.actions.kill.enabled),
            FieldKey::KillProcesses => self.config.actions.kill.processes.join(","),
            FieldKey::PowerMode => self.config.actions.power.mode.clone(),
            FieldKey::CustomEnabled => on_off(self.config.actions.custom.enabled),
            FieldKey::CustomCommands => self.config.actions.custom.commands.join(";"),
        }
    }

    fn toggle_bool(&mut self, key: FieldKey) {
        match key {
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
            FieldKey::LockEnabled => {
                self.config.actions.lock.enabled = !self.config.actions.lock.enabled
            }
            FieldKey::KillEnabled => {
                self.config.actions.kill.enabled = !self.config.actions.kill.enabled
            }
            FieldKey::CustomEnabled => {
                self.config.actions.custom.enabled = !self.config.actions.custom.enabled
            }
            _ => {}
        }
    }

    fn cycle(&mut self, key: FieldKey, options: &'static [&'static str]) {
        let current = match key {
            FieldKey::WorkspaceBackend => &mut self.config.actions.workspace.backend,
            FieldKey::MediaAction => &mut self.config.actions.media.action,
            FieldKey::PowerMode => &mut self.config.actions.power.mode,
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
    WorkspaceEnabled,
    WorkspaceBackend,
    WorkspaceNumber,
    MediaEnabled,
    MediaAction,
    BrightnessEnabled,
    BrightnessValue,
    MuteEnabled,
    LockEnabled,
    LockCommand,
    KillEnabled,
    KillProcesses,
    PowerMode,
    CustomEnabled,
    CustomCommands,
}

const FIELDS: &[Field] = &[
    Field {
        label: "duration",
        key: FieldKey::Duration,
        kind: FieldKind::Edit,
    },
    Field {
        label: "workspace enabled",
        key: FieldKey::WorkspaceEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "workspace backend",
        key: FieldKey::WorkspaceBackend,
        kind: FieldKind::Cycle(&["auto", "niri", "hyprland", "kde"]),
    },
    Field {
        label: "workspace number",
        key: FieldKey::WorkspaceNumber,
        kind: FieldKind::Int,
    },
    Field {
        label: "media enabled",
        key: FieldKey::MediaEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "media action",
        key: FieldKey::MediaAction,
        kind: FieldKind::Cycle(&["stop", "pause", "play-pause", "next", "previous", "none"]),
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
        label: "mute enabled",
        key: FieldKey::MuteEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "lock enabled",
        key: FieldKey::LockEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "lock command",
        key: FieldKey::LockCommand,
        kind: FieldKind::Edit,
    },
    Field {
        label: "kill enabled",
        key: FieldKey::KillEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "kill processes",
        key: FieldKey::KillProcesses,
        kind: FieldKind::Csv,
    },
    Field {
        label: "power mode",
        key: FieldKey::PowerMode,
        kind: FieldKind::Cycle(&["none", "poweroff", "reboot"]),
    },
    Field {
        label: "custom enabled",
        key: FieldKey::CustomEnabled,
        kind: FieldKind::Bool,
    },
    Field {
        label: "custom commands",
        key: FieldKey::CustomCommands,
        kind: FieldKind::Semi,
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
        FieldKey::MediaAction => config.actions.media.enabled,
        FieldKey::BrightnessValue => config.actions.brightness.enabled,
        FieldKey::LockCommand => config.actions.lock.enabled,
        FieldKey::KillProcesses => config.actions.kill.enabled,
        FieldKey::CustomCommands => config.actions.custom.enabled,
        FieldKey::Duration
        | FieldKey::WorkspaceEnabled
        | FieldKey::MediaEnabled
        | FieldKey::BrightnessEnabled
        | FieldKey::MuteEnabled
        | FieldKey::LockEnabled
        | FieldKey::KillEnabled
        | FieldKey::PowerMode
        | FieldKey::CustomEnabled => true,
    }
}

fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h{minutes}m{seconds}s")
    } else if minutes > 0 {
        format!("{minutes}m{seconds}s")
    } else {
        format!("{seconds}s")
    }
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
        return "time left: --".to_string();
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

fn wrap_command(command: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in command.split_whitespace() {
        if current.is_empty() {
            current.push_str("$ ");
            current.push_str(word);
        } else if current.len() + word.len() < width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = format!("  {word}");
        }
    }
    if !current.is_empty() {
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

    fn quiet_config(duration: &str) -> Config {
        let mut config = Config {
            duration: duration.to_string(),
            ..Config::default()
        };
        config.actions.workspace.enabled = false;
        config.actions.media.enabled = false;
        config.actions.brightness.enabled = false;
        config.actions.mute.enabled = false;
        config.actions.lock.enabled = false;
        config.actions.kill.enabled = false;
        config.actions.power.mode = "none".to_string();
        config.actions.custom.enabled = false;
        config
    }

    #[test]
    fn parse_duration_accepts_go_style_values() {
        assert!(parse_duration("25m").is_ok());
        assert!(parse_duration("1h30m").is_ok());
        assert!(parse_duration("1ns").is_ok());
        assert!(parse_duration("bad").is_err());
        assert!(parse_duration("0s").is_err());
    }

    #[test]
    fn preview_matches_expected_commands() {
        let mut config = Config::default();
        config.actions.lock.enabled = true;
        config.actions.kill.enabled = true;
        config.actions.kill.processes = vec!["firefox".to_string(), "telegram-desktop".to_string()];
        config.actions.custom.enabled = true;
        config.actions.custom.commands = vec!["echo done".to_string()];

        let preview = preview_commands(&config);
        assert_eq!(preview.len(), 6);
        assert_eq!(preview[1], "playerctl stop");
        assert_eq!(preview[2], "loginctl lock-session");
        assert_eq!(preview[3], "pkill firefox");
        assert_eq!(preview[4], "pkill telegram-desktop");
        assert_eq!(preview[5], "echo done");
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
    fn run_timer_quiet_mode_writes_nothing() {
        let stop = Arc::new(AtomicBool::new(false));
        let config = quiet_config("1ns");

        run_timer(
            &config,
            &stop,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Tui,
            TimerOptions::default(),
            None,
        )
        .unwrap();
    }

    #[test]
    fn run_timer_cli_mode_writes_status_lines() {
        let stop = Arc::new(AtomicBool::new(false));
        let config = quiet_config("10ms");
        let mut output = Vec::new();

        run_timer(
            &config,
            &stop,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions::default(),
            Some(&mut output),
        )
        .unwrap();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("timer started: 10ms"));
        assert!(text.contains("remaining:"));
        assert!(text.contains("finished"));
    }

    #[test]
    fn run_timer_attempts_all_actions_before_returning_failures() {
        let stop = Arc::new(AtomicBool::new(false));
        let mut config = quiet_config("1ns");
        config.actions.custom.enabled = true;
        config.actions.custom.commands = vec!["false".to_string(), "true".to_string()];
        let mut output = Vec::new();

        let error = run_timer(
            &config,
            &stop,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Cli,
            TimerOptions::default(),
            Some(&mut output),
        )
        .unwrap_err();

        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("run: false"));
        assert!(text.contains("run: true"));
        assert!(error.to_string().contains("1 action(s) failed"));
    }

    #[test]
    fn ensure_config_validates_loaded_config() {
        let path = std::env::temp_dir().join(format!(
            "gosleep-timer-invalid-{}-{}.yaml",
            std::process::id(),
            "power"
        ));
        fs::write(
            &path,
            "duration: 25m\nactions:\n  power:\n    mode: suspend\n",
        )
        .unwrap();

        let error = ensure_config(&path).unwrap_err();
        fs::remove_file(path).ok();

        assert!(error.to_string().contains("invalid power mode"));
    }

    #[test]
    fn timer_header_text_compacts_for_narrow_widths() {
        let remaining = Duration::from_secs(65);

        assert_eq!(format_time_left_text(None, remaining, 20), "time left: --");
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
        assert!(keys.contains(&FieldKey::MediaEnabled));
        assert!(!keys.contains(&FieldKey::MediaAction));
        assert!(!keys.contains(&FieldKey::WorkspaceBackend));
        assert!(!keys.contains(&FieldKey::WorkspaceNumber));
        assert!(!keys.contains(&FieldKey::BrightnessValue));
        assert!(!keys.contains(&FieldKey::LockCommand));
        assert!(!keys.contains(&FieldKey::KillProcesses));
        assert!(!keys.contains(&FieldKey::CustomCommands));

        config.actions.media.enabled = true;
        config.actions.workspace.enabled = true;
        config.actions.brightness.enabled = true;
        config.actions.lock.enabled = true;
        config.actions.kill.enabled = true;
        config.actions.custom.enabled = true;

        let keys = visible_field_keys(&config);
        assert!(keys.contains(&FieldKey::MediaAction));
        assert!(keys.contains(&FieldKey::WorkspaceBackend));
        assert!(keys.contains(&FieldKey::WorkspaceNumber));
        assert!(keys.contains(&FieldKey::BrightnessValue));
        assert!(keys.contains(&FieldKey::LockCommand));
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

    fn visible_field_keys(config: &Config) -> Vec<FieldKey> {
        visible_fields(config)
            .into_iter()
            .map(|field| field.key)
            .collect()
    }

    fn dummy_timer_state() -> TimerState {
        let stop = Arc::new(AtomicBool::new(false));
        let handle = thread::spawn(|| {
            Ok(TimerReport {
                started_at: 0,
                duration: Duration::from_secs(1),
                actions: Vec::new(),
                failures: Vec::new(),
                dry_run: false,
                no_actions: false,
            })
        });
        TimerState {
            duration: Duration::from_secs(1),
            stop,
            pause: Arc::new(AtomicBool::new(false)),
            remaining_ms: Arc::new(AtomicU64::new(1000)),
            handle,
        }
    }
}
