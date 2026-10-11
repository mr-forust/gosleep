use std::{
    fs,
    io::{self, Read as _, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

const TIMER_STOPPED_MSG: &str = "timer stopped";
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
    inhibit_sleep: bool,
    actions: Actions,
    discord: DiscordConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct Actions {
    workspace: WorkspaceAction,
    media: MediaAction,
    brightness: BrightnessAction,
    mute: ToggleAction,
    mute_input: ToggleAction,
    lock: LockAction,
    monitor: MonitorAction,
    notify: NotifyAction,
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
    command: String,
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
    backend: String,
    command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct MonitorAction {
    enabled: bool,
    backend: String,
    command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct NotifyAction {
    enabled: bool,
    message: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct DiscordConfig {
    enabled: bool,
    application_id: String,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            application_id: String::new(),
        }
    }
}

impl DiscordConfig {
    fn should_connect(&self) -> bool {
        self.enabled && !self.application_id.trim().is_empty()
    }
}

/// Timer phase published through Discord local IPC (US-003/US-004 own wording).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PresenceActivity {
    details: String,
    state: String,
    start_timestamp: Option<u64>,
    end_timestamp: Option<u64>,
}

fn presence_activity_payload(
    activity: &PresenceActivity,
    pid: u32,
    nonce: &str,
) -> serde_json::Value {
    let mut timestamps = serde_json::Map::new();
    if let Some(start) = activity.start_timestamp {
        timestamps.insert(
            "start".to_string(),
            serde_json::Value::Number(serde_json::Number::from(start)),
        );
    }
    if let Some(end) = activity.end_timestamp {
        timestamps.insert(
            "end".to_string(),
            serde_json::Value::Number(serde_json::Number::from(end)),
        );
    }
    let mut fields = serde_json::Map::new();
    fields.insert(
        "details".to_string(),
        serde_json::Value::String(activity.details.clone()),
    );
    fields.insert(
        "state".to_string(),
        serde_json::Value::String(activity.state.clone()),
    );
    if !timestamps.is_empty() {
        fields.insert(
            "timestamps".to_string(),
            serde_json::Value::Object(timestamps),
        );
    }
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": fields,
        },
        "nonce": nonce,
    })
}

fn presence_clear_payload(pid: u32, nonce: &str) -> serde_json::Value {
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": pid,
            "activity": null,
        },
        "nonce": nonce,
    })
}

fn encode_discord_ipc_frame(opcode: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(8 + payload.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn discord_ipc_candidates_from(bases: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for base in bases {
        for index in 0..10 {
            candidates.push(base.join(format!("discord-ipc-{index}")));
        }
    }
    candidates
}

fn discord_ipc_candidates() -> Vec<PathBuf> {
    let mut bases = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        let base = PathBuf::from(runtime);
        if !base.as_os_str().is_empty() {
            bases.push(base);
        }
    }
    if let Some(tmpdir) = std::env::var_os("TMPDIR") {
        let base = PathBuf::from(tmpdir);
        if !base.as_os_str().is_empty() {
            bases.push(base);
        }
    }
    bases.push(PathBuf::from("/tmp"));
    let mut seen = std::collections::HashSet::new();
    bases.retain(|base| seen.insert(base.clone()));
    discord_ipc_candidates_from(&bases)
}

#[derive(Debug)]
enum PresenceRequest {
    Update(PresenceActivity),
    Clear,
}

/// Pure countdown projection for US-003.
///
/// Running presence shows `Sleep timer` with `{remaining} until sleep` and a
/// countdown end timestamp. `Power off after timer` is appended only when the
/// power mode is exactly `poweroff`.
fn should_show_poweroff(power_mode: &str) -> bool {
    power_mode == "poweroff"
}

fn countdown_presence(remaining: Duration, power_mode: &str, now_unix: u64) -> PresenceActivity {
    let mut state = format!("{} until sleep", format_duration(remaining));
    if should_show_poweroff(power_mode) {
        state.push_str(" \u{2022} Power off after timer");
    }
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state,
        start_timestamp: None,
        end_timestamp: Some(now_unix.saturating_add(remaining.as_secs())),
    }
}

/// Paused projection: same details line, `paused` state, and no timestamps so
/// Discord never shows a stale live countdown while paused.
fn paused_presence(power_mode: &str) -> PresenceActivity {
    let mut state = "paused".to_string();
    if should_show_poweroff(power_mode) {
        state.push_str(" \u{2022} Power off after timer");
    }
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state,
        start_timestamp: None,
        end_timestamp: None,
    }
}

/// Post-countdown transitional state published on successful completion
/// instead of clearing. US-004 replaces this with the elapsed `Sleeping for`
/// status and extends the presence lifetime; US-003 only guarantees the
/// countdown is not cleared on success.
fn post_countdown_presence() -> PresenceActivity {
    PresenceActivity {
        details: "Sleep timer".to_string(),
        state: "finished".to_string(),
        start_timestamp: None,
        end_timestamp: None,
    }
}

/// Non-blocking Discord Rich Presence client over local IPC.
///
/// `start` never blocks the caller: the socket handshake runs on a background
/// thread with short timeouts, and `update`/`clear` only enqueue on an
/// unbounded channel. Missing Discord, a missing socket, or any I/O failure
/// is swallowed so timer runs never fail or stall. Dropping the client closes
/// the channel; the worker attempts one final clear in the background.
struct DiscordPresence {
    sender: Option<mpsc::Sender<PresenceRequest>>,
    clear_on_drop: bool,
}

impl DiscordPresence {
    fn start(config: &DiscordConfig) -> Self {
        if !config.should_connect() {
            return Self {
                sender: None,
                clear_on_drop: true,
            };
        }
        let application_id = config.application_id.trim().to_string();
        if application_id.is_empty() {
            return Self {
                sender: None,
                clear_on_drop: true,
            };
        }
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || discord_presence_worker(&application_id, requests));
        Self {
            sender: Some(sender),
            clear_on_drop: true,
        }
    }

    // Used by tests to assert no-op vs active; US-003/004 will branch on it.
    #[allow(dead_code)]
    fn is_active(&self) -> bool {
        self.sender.is_some()
    }

    // Countdown (US-003) and post-countdown (US-004) updates; non-blocking.
    fn update(&self, activity: PresenceActivity) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(PresenceRequest::Update(activity));
        }
    }

    fn clear(&self) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(PresenceRequest::Clear);
        }
    }

    /// Stop the automatic clear on drop. Used on successful completion so the
    /// countdown transitions to the post-countdown state instead of clearing.
    fn disarm(&mut self) {
        self.clear_on_drop = false;
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        // Best-effort clear in the background, then close the channel so the
        // worker exits. No join here so countdown/action paths never block.
        // Disarmed on success so the post-countdown state is retained.
        if self.clear_on_drop {
            self.clear();
        }
        self.sender.take();
    }
}

fn discord_presence_worker(application_id: &str, requests: mpsc::Receiver<PresenceRequest>) {
    let mut stream = discord_connect(application_id);
    let pid = std::process::id();
    for request in requests {
        let Some(socket) = stream.as_mut() else {
            continue;
        };
        let payload = match &request {
            PresenceRequest::Update(activity) => {
                presence_activity_payload(activity, pid, &next_presence_nonce())
            }
            PresenceRequest::Clear => presence_clear_payload(pid, &next_presence_nonce()),
        };
        if discord_send_activity(socket, &payload).is_err() {
            stream = None;
        }
    }
    // No final clear here: `Drop` already sends an explicit `Clear` when the
    // run is stopped, and a disarmed guard (successful completion) must retain
    // the post-countdown state instead of wiping it. Discord clears stale
    // presence itself once the process exits and the socket closes.
}

fn discord_connect(application_id: &str) -> Option<std::os::unix::net::UnixStream> {
    use std::os::unix::net::UnixStream;
    for path in discord_ipc_candidates() {
        let Ok(stream) = UnixStream::connect(&path) else {
            continue;
        };
        if discord_handshake(&stream, application_id).is_ok() {
            return Some(stream);
        }
    }
    None
}

fn discord_handshake(
    stream: &std::os::unix::net::UnixStream,
    application_id: &str,
) -> io::Result<()> {
    let mut stream = stream.try_clone()?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let payload = serde_json::json!({
        "v": 1,
        "client_id": application_id,
    });
    let bytes = serde_json::to_vec(&payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let frame = encode_discord_ipc_frame(0, &bytes);
    stream.write_all(&frame)?;
    // Best effort: consume the READY reply when Discord answers, but a missing
    // or slow reply must not fail the worker; updates will fail later if the
    // handshake did not actually succeed.
    let mut header = [0_u8; 8];
    match stream.read_exact(&mut header) {
        Ok(()) => {
            let len = u32::from_le_bytes(header[4..8].try_into().unwrap_or([0; 4])) as usize;
            if len <= 64 * 1024 {
                let mut body = vec![0_u8; len];
                let _ = stream.read_exact(&mut body);
            }
            Ok(())
        }
        Err(err)
            if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut =>
        {
            Ok(())
        }
        Err(_) => Ok(()),
    }
}

fn discord_send_activity(
    stream: &mut std::os::unix::net::UnixStream,
    payload: &serde_json::Value,
) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let bytes = serde_json::to_vec(payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let frame = encode_discord_ipc_frame(1, &bytes);
    stream.write_all(&frame)?;
    Ok(())
}

fn next_presence_nonce() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("gosleep-{}-{count}", unix_timestamp())
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

struct InhibitGuard(Option<std::process::Child>);

impl Drop for InhibitGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            duration: "25m".to_string(),
            inhibit_sleep: false,
            actions: Actions::default(),
            discord: DiscordConfig::default(),
        }
    }
}

impl Default for WorkspaceAction {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: "auto".to_string(),
            number: 3,
            command: String::new(),
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
            backend: "auto".to_string(),
            command: String::new(),
        }
    }
}

impl Default for MonitorAction {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: "auto".to_string(),
            command: String::new(),
        }
    }
}

impl Default for NotifyAction {
    fn default() -> Self {
        Self {
            enabled: false,
            message: "Timer finished".to_string(),
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
        self.shell
            .as_deref()
            .map(escape_terminal_text)
            .unwrap_or_else(|| {
                self.args
                    .iter()
                    .map(|argument| display_argument(argument))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
    }

    fn is_dangerous(&self) -> bool {
        if self.label == "notify" {
            return false;
        }
        if matches!(self.label, "kill" | "power") {
            return true;
        }
        let Some(shell) = &self.shell else {
            return false;
        };
        if matches!(self.label, "custom" | "workspace" | "monitor" | "lock") {
            return true;
        }
        let command = shell.to_lowercase();
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

fn is_terminal_control(ch: char) -> bool {
    ch.is_control()
        || matches!(
            ch,
            '\u{061c}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
        )
}

fn append_terminal_safe_char(output: &mut String, ch: char) {
    match ch {
        '\n' => output.push_str("\\n"),
        '\r' => output.push_str("\\r"),
        '\t' => output.push_str("\\t"),
        '\0' => output.push_str("\\0"),
        ch if is_terminal_control(ch) => output.push_str(&format!("\\u{{{:x}}}", ch as u32)),
        ch => output.push(ch),
    }
}

fn escape_terminal_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        append_terminal_safe_char(&mut escaped, ch);
    }
    escaped
}

fn display_argument(argument: &str) -> String {
    let quoted = argument.is_empty()
        || argument.chars().any(|ch| {
            !ch.is_ascii_alphanumeric()
                && !matches!(
                    ch,
                    '-' | '_' | '.' | '/' | ':' | '@' | '+' | '=' | ',' | '%'
                )
        });
    let mut display = String::with_capacity(argument.len() + if quoted { 2 } else { 0 });
    if quoted {
        display.push('\'');
    }
    for ch in argument.chars() {
        match ch {
            '\'' if quoted => display.push_str("'\\''"),
            ch => append_terminal_safe_char(&mut display, ch),
        }
    }
    if quoted {
        display.push('\'');
    }
    display
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
    let mut config: Config = noyalib::from_str(&data).context("parse config")?;
    normalize_config(&mut config);
    validate_config(&config)?;
    Ok(config)
}

fn save_config(path: &Path, config: &Config) -> Result<()> {
    let mut config = config.clone();
    normalize_config(&mut config);
    validate_config(&config)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = noyalib::to_string(&config).context("serialize config")?;
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
    if config.actions.lock.backend.is_empty() {
        config.actions.lock.backend = "auto".to_string();
    }
    if config.actions.monitor.backend.is_empty() {
        config.actions.monitor.backend = "auto".to_string();
    }
    if config.actions.notify.message.is_empty() {
        config.actions.notify.message = "Timer finished".to_string();
    }
    if config.actions.power.mode.is_empty() {
        config.actions.power.mode = "none".to_string();
    }
    config.discord.application_id = config.discord.application_id.trim().to_string();
}

fn validate_config(config: &Config) -> Result<()> {
    parse_duration(&config.duration)?;
    match config.actions.power.mode.as_str() {
        "none" | "suspend" | "hibernate" | "hybrid-sleep" | "poweroff" | "reboot" => {}
        mode => bail!("invalid power mode {mode:?}"),
    }
    match config.actions.workspace.backend.as_str() {
        "auto" | "niri" | "hyprland" | "sway" | "i3" | "kde" => {}
        backend => bail!("invalid workspace backend {backend:?}"),
    }
    match config.actions.monitor.backend.as_str() {
        "auto" | "hyprland" | "niri" | "sway" | "kde" | "x11" => {}
        backend => bail!("invalid monitor backend {backend:?}"),
    }
    match config.actions.lock.backend.as_str() {
        "auto" | "loginctl" | "hyprlock" | "swaylock" | "i3lock" | "gnome" | "kde"
        | "xscreensaver" => {}
        backend => bail!("invalid lock backend {backend:?}"),
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
        if !number.is_finite() {
            bail!("duration component must be finite");
        }

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
        if !total.is_finite() {
            bail!("duration is too large");
        }
    }

    if total <= 0.0 {
        bail!("duration must be greater than zero");
    }
    let duration = Duration::try_from_secs_f64(total).context("duration is too large")?;
    if duration.is_zero() {
        bail!("duration must be greater than zero");
    }
    Ok(duration)
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
    if actions.mute_input.enabled {
        commands.push(ActionCommand {
            label: "mute_input",
            args: vec![],
            shell: Some(
                "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ 1 || pactl set-source-mute @DEFAULT_SOURCE@ 1".to_string(),
            ),
        });
    }
    if actions.lock.enabled {
        commands.push(lock_command(&actions.lock));
    }
    if actions.monitor.enabled {
        commands.push(monitor_command(&actions.monitor));
    }
    if actions.notify.enabled {
        commands.push(ActionCommand {
            label: "notify",
            args: vec![
                "notify-send".to_string(),
                "gosleep-timer".to_string(),
                "--".to_string(),
                actions.notify.message.clone(),
            ],
            shell: None,
        });
    }
    if actions.kill.enabled {
        for process in &actions.kill.processes {
            let process = process.trim();
            if !process.is_empty() {
                commands.push(ActionCommand {
                    label: "kill",
                    args: vec!["pkill".to_string(), "--".to_string(), process.to_string()],
                    shell: None,
                });
            }
        }
    }
    match actions.power.mode.as_str() {
        "suspend" => commands.push(ActionCommand {
            label: "power",
            args: vec!["systemctl".to_string(), "suspend".to_string()],
            shell: None,
        }),
        "hibernate" => commands.push(ActionCommand {
            label: "power",
            args: vec!["systemctl".to_string(), "hibernate".to_string()],
            shell: None,
        }),
        "hybrid-sleep" => commands.push(ActionCommand {
            label: "power",
            args: vec!["systemctl".to_string(), "hybrid-sleep".to_string()],
            shell: None,
        }),
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
    if !action.command.is_empty() {
        return ActionCommand {
            label: "workspace",
            args: vec![],
            shell: Some(action.command.clone()),
        };
    }
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
        "sway" => ActionCommand {
            label: "workspace",
            args: vec!["swaymsg", "workspace", "number", &number]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "i3" => ActionCommand {
            label: "workspace",
            args: vec!["i3-msg", "workspace", "number", &number]
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
                "if command -v niri >/dev/null 2>&1; then niri msg action focus-workspace {0}; elif command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch workspace {0}; elif command -v swaymsg >/dev/null 2>&1; then swaymsg workspace number {0}; elif command -v i3-msg >/dev/null 2>&1; then i3-msg workspace number {0}; elif command -v qdbus >/dev/null 2>&1; then qdbus org.kde.KWin /KWin org.kde.KWin.setCurrentDesktop {0}; else echo 'no supported workspace backend found' >&2; exit 127; fi",
                number
            )),
        },
    }
}

fn monitor_command(action: &MonitorAction) -> ActionCommand {
    if !action.command.is_empty() {
        return ActionCommand {
            label: "monitor",
            args: vec![],
            shell: Some(action.command.clone()),
        };
    }
    match action.backend.as_str() {
        "hyprland" => ActionCommand {
            label: "monitor",
            args: vec!["hyprctl", "dispatch", "dpms", "off"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "niri" => ActionCommand {
            label: "monitor",
            args: vec!["niri", "msg", "action", "power-off-monitors"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "sway" => ActionCommand {
            label: "monitor",
            args: vec!["swaymsg", "output", "*", "dpms", "off"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "kde" => ActionCommand {
            label: "monitor",
            args: vec!["kscreen-doctor", "--dpms", "off"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            shell: None,
        },
        "x11" => ActionCommand {
            label: "monitor",
            args: vec![],
            shell: Some("xset dpms force off".to_string()),
        },
        _ => ActionCommand {
            label: "monitor",
            args: vec![],
            shell: Some(
                "if command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch dpms off; elif command -v niri >/dev/null 2>&1; then niri msg action power-off-monitors; elif command -v swaymsg >/dev/null 2>&1; then swaymsg output '*' dpms off; elif command -v kscreen-doctor >/dev/null 2>&1; then kscreen-doctor --dpms off; elif command -v xset >/dev/null 2>&1; then xset dpms force off; else echo 'no supported monitor power-off backend found' >&2; exit 127; fi".to_string()
            ),
        },
    }
}

fn lock_command(action: &LockAction) -> ActionCommand {
    if !action.command.is_empty() {
        return ActionCommand {
            label: "lock",
            args: vec![],
            shell: Some(action.command.clone()),
        };
    }
    match action.backend.as_str() {
        "loginctl" => ActionCommand {
            label: "lock",
            args: vec!["loginctl".to_string(), "lock-session".to_string()],
            shell: None,
        },
        "hyprlock" => ActionCommand {
            label: "lock",
            args: vec![],
            shell: Some("hyprctl dispatch exec hyprlock".to_string()),
        },
        "swaylock" => ActionCommand {
            label: "lock",
            args: vec!["swaylock".to_string()],
            shell: None,
        },
        "i3lock" => ActionCommand {
            label: "lock",
            args: vec!["i3lock".to_string()],
            shell: None,
        },
        "gnome" => ActionCommand {
            label: "lock",
            args: vec!["gnome-screensaver-command".to_string(), "-l".to_string()],
            shell: None,
        },
        "kde" => ActionCommand {
            label: "lock",
            args: vec![
                "qdbus".to_string(),
                "org.kde.ScreenSaver".to_string(),
                "/ScreenSaver".to_string(),
                "org.freedesktop.ScreenSaver.SetActive".to_string(),
                "true".to_string(),
            ],
            shell: None,
        },
        "xscreensaver" => ActionCommand {
            label: "lock",
            args: vec!["xscreensaver-command".to_string(), "-lock".to_string()],
            shell: None,
        },
        _ => ActionCommand {
            label: "lock",
            args: vec![],
            shell: Some(
                "if command -v loginctl >/dev/null 2>&1; then loginctl lock-session; elif command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch exec hyprlock; elif command -v swaylock >/dev/null 2>&1; then swaylock; elif command -v i3lock >/dev/null 2>&1; then i3lock; elif command -v gnome-screensaver-command >/dev/null 2>&1; then gnome-screensaver-command -l; elif command -v qdbus >/dev/null 2>&1; then qdbus org.kde.ScreenSaver /ScreenSaver org.freedesktop.ScreenSaver.SetActive true; elif command -v xscreensaver-command >/dev/null 2>&1; then xscreensaver-command -lock; else echo 'no supported lock backend found' >&2; exit 127; fi".to_string()
            ),
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

fn write_timer_line(output: &mut dyn Write, message: &str) -> io::Result<()> {
    writeln!(output, "{message}")
}

fn write_timer_progress(output: &mut dyn Write, message: &str) -> io::Result<()> {
    write!(output, "\r{message}")?;
    output.flush()
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
    // Local IPC only: no bot token, network service, or credentials. The
    // worker connects in the background; missing Discord never fails/stalls.
    // Countdown updates (US-003) are published from this thread for both CLI
    // and TUI runs, including --no-actions and dry-run. Stopped runs clear
    // explicitly; successful runs transition to the post-countdown state and
    // disarm the drop-clear (US-004 extends the lifetime past this guard).
    let mut discord = DiscordPresence::start(&config.discord);
    discord.update(countdown_presence(
        duration,
        &config.actions.power.mode,
        unix_timestamp(),
    ));
    let mut last_presence_update = Instant::now();
    let mut presence_paused = false;
    let started_at = unix_timestamp();
    let started = Instant::now();
    let mut remaining = duration;
    let mut last_tick = started;
    remaining_ms.store(
        duration.as_millis().min(u128::from(u64::MAX)) as u64,
        Ordering::Relaxed,
    );
    let _inhibit = if config.inhibit_sleep {
        let child = Command::new("systemd-inhibit")
            .args([
                "--what=sleep",
                "--who=gosleep-timer",
                "--why=Timer running",
                "sleep",
                "infinity",
            ])
            .spawn()
            .context("spawn systemd-inhibit")?;
        Some(InhibitGuard(Some(child)))
    } else {
        None
    };

    if let Some(output) = output.as_deref_mut() {
        write_timer_line(output, &format!("timer started: {}", config.duration))?;
    }

    loop {
        if stop.load(Ordering::Relaxed) {
            if let Some(output) = output.as_deref_mut() {
                write_timer_line(output, "stopped")?;
            }
            discord.clear();
            bail!(TIMER_STOPPED_MSG);
        }

        if pause.load(Ordering::Relaxed) {
            if !presence_paused {
                discord.update(paused_presence(&config.actions.power.mode));
                presence_paused = true;
            }
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
        if presence_paused {
            discord.update(countdown_presence(
                remaining,
                &config.actions.power.mode,
                unix_timestamp(),
            ));
            last_presence_update = Instant::now();
            presence_paused = false;
        } else if last_presence_update.elapsed() >= Duration::from_secs(5) {
            discord.update(countdown_presence(
                remaining,
                &config.actions.power.mode,
                unix_timestamp(),
            ));
            last_presence_update = Instant::now();
        }
        if mode == RunMode::Cli
            && let Some(output) = output.as_deref_mut()
        {
            write_timer_progress(
                output,
                &format!("remaining: {}", format_duration(remaining)),
            )?;
        }
        thread::sleep(Duration::from_millis(250));
    }
    remaining_ms.store(0, Ordering::Relaxed);

    if stop.load(Ordering::Relaxed) {
        if let Some(output) = output.as_deref_mut() {
            write_timer_line(output, "stopped")?;
        }
        discord.clear();
        bail!("timer stopped");
    }

    if let Some(output) = output.as_deref_mut() {
        write_timer_line(output, "countdown finished")?;
    }
    let mut failures = Vec::new();
    let actions = if options.no_actions {
        Vec::new()
    } else {
        build_action_commands(config)
    };
    for command in &actions {
        if stop.load(Ordering::Relaxed) {
            if let Some(output) = output.as_deref_mut() {
                write_timer_line(output, "stopped")?;
            }
            discord.clear();
            break;
        }
        if let Some(output) = output.as_deref_mut() {
            let prefix = if options.dry_run { "would run" } else { "run" };
            write_timer_line(output, &format!("{prefix}: {}", command.display()))?;
        }
        if options.dry_run {
            continue;
        }
        if let Err(err) = run_command(command, mode) {
            let failure = format!("{} failed: {err}", command.label);
            if let Some(output) = output.as_deref_mut() {
                write_timer_line(output, &failure)?;
            }
            failures.push(failure);
        } else if let Some(output) = output.as_deref_mut() {
            write_timer_line(output, &format!("{}: done", command.label))?;
        }
    }
    if stop.load(Ordering::Relaxed) {
        discord.clear();
        bail!(TIMER_STOPPED_MSG);
    }
    // Successful completion transitions to the post-countdown state instead
    // of clearing, so Discord keeps showing the finished status until the
    // process exits (CLI) or US-004 publishes the elapsed sleep status.
    discord.update(post_countdown_presence());
    discord.disarm();
    let report = TimerReport {
        started_at,
        duration,
        actions: actions.iter().map(ActionCommand::display).collect(),
        failures: failures.clone(),
        dry_run: options.dry_run,
        no_actions: options.no_actions,
    };
    if !failures.is_empty()
        && let Some(output) = &mut output
    {
        write_timer_line(output, &format!("{} action(s) failed", failures.len()))?;
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
}

struct TimerState {
    duration: Duration,
    stop: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    remaining_ms: Arc<AtomicU64>,
    handle: thread::JoinHandle<Result<TimerReport>>,
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
                "Tab group | {action} | r run | s save | q quit\n1 30m | 2 1h | 3 2h | l preview/results | PgUp/PgDn scroll\n{}",
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
                "Tab group | Enter edit | r run\n1/2/3 presets | l log | PgUp/PgDn\ns save | q quit"
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
        frame.render_widget(
            Paragraph::new(self.status.clone())
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
        let tabs = GROUPS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                Span::styled(
                    format!("{name} "),
                    if index == self.group {
                        Style::default().fg(Color::Black).bg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                )
            })
            .collect::<Vec<_>>();
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
                let prefix = format!("{marker} {}: ", field.label);
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
        let height = usize::from(area.height.saturating_sub(2));
        self.preview_offset = self.preview_offset.min(lines.len().saturating_sub(height));
        let title = if self.show_log {
            "results"
        } else if area.width < 60 {
            "plan / commands"
        } else {
            "action plan / commands"
        };
        let range = if lines.len() > height {
            format!(
                " {}-{}/{}",
                self.preview_offset + 1,
                (self.preview_offset + height).min(lines.len()),
                lines.len()
            )
        } else {
            String::new()
        };
        frame.render_widget(
            Paragraph::new(
                lines
                    .into_iter()
                    .skip(self.preview_offset)
                    .take(height)
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::default()
                    .title(format!(" {title}{range} "))
                    .borders(Borders::ALL)
                    .border_style(Color::Blue),
            ),
            area,
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
        if let Some(timer) = self.timer.take() {
            timer.stop.store(true, Ordering::Relaxed);
            let result = timer.handle.join();
            self.action_log.extend(timer.updates.try_iter());
            self.apply_timer_result(result);
        } else {
            self.status = "timer stopped".to_string();
        }
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

    fn apply_timer_result(&mut self, result: thread::Result<Result<TimerReport>>) {
        match result {
            Ok(Ok(report)) => {
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
        FieldKey::WorkspaceBackend | FieldKey::WorkspaceNumber | FieldKey::WorkspaceCommand => {
            config.actions.workspace.enabled
        }
        FieldKey::MediaAction => config.actions.media.enabled,
        FieldKey::BrightnessValue => config.actions.brightness.enabled,
        FieldKey::LockBackend | FieldKey::LockCommand => config.actions.lock.enabled,
        FieldKey::MonitorBackend | FieldKey::MonitorCommand => config.actions.monitor.enabled,
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
    fn action_display_escapes_terminal_controls_and_quotes_arguments() {
        let message = "line one\nline two\r\t\u{1b}[31m\u{9b}2J\u{7}\u{202e}猫🙂";
        let invisible_joiner = "syst\u{200d}emctl";
        let command = ActionCommand {
            label: "notify",
            args: vec![
                "notify-send".to_string(),
                "--".to_string(),
                message.to_string(),
                "$(id)".to_string(),
                "`id`".to_string(),
                invisible_joiner.to_string(),
            ],
            shell: None,
        };

        let display = command.display();

        assert_eq!(command.args[2], message);
        assert!(display.contains("\\n"));
        assert!(display.contains("\\r\\t"));
        assert!(display.contains("\\u{1b}[31m"));
        assert!(display.contains("\\u{9b}2J"));
        assert!(display.contains("\\u{7}"));
        assert!(display.contains("\\u{202e}"));
        assert!(display.contains("猫🙂"));
        assert!(display.contains("'$(id)'"));
        assert!(display.contains("'`id`'"));
        assert!(display.contains("'syst\\u{200d}emctl'"));
        assert!(!display.chars().any(char::is_control));
    }

    #[test]
    fn shell_display_escapes_controls_without_changing_executed_command() {
        let shell = "printf 'first\nsecond' \u{1b}]0;title\u{7}";
        let command = ActionCommand {
            label: "custom",
            args: Vec::new(),
            shell: Some(shell.to_string()),
        };

        assert_eq!(command.shell.as_deref(), Some(shell));
        let display = command.display();
        assert!(display.contains("first\\nsecond"));
        assert!(display.contains("\\u{1b}]0;title\\u{7}"));
        assert!(!display.chars().any(char::is_control));
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
            assert!(rendered.contains("\\u{1b}]0;title\\u{7}"));
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

    #[test]
    fn tui_worker_reports_each_controlled_action_result() {
        let mut config = quiet_config("1ns");
        config.actions.custom.enabled = true;
        config.actions.custom.commands = vec!["exit 0".to_string(), "exit 7".to_string()];
        let (sender, receiver) = mpsc::channel();
        let mut writer = UpdateWriter {
            sender,
            pending: String::new(),
        };
        let report = run_timer(
            &config,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Tui,
            TimerOptions::default(),
            Some(&mut writer),
        )
        .unwrap();
        let lines = receiver.try_iter().collect::<Vec<_>>();
        assert!(lines.iter().any(|line| line == "custom: done"));
        assert!(lines.iter().any(|line| line.starts_with("custom failed:")));
        assert_eq!(report.failures.len(), 1);
        assert!(!lines.iter().any(|line| line.contains("remaining:")));
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
    fn parse_duration_accepts_go_style_values() {
        assert!(parse_duration("25m").is_ok());
        assert!(parse_duration("1h30m").is_ok());
        assert!(parse_duration("1ns").is_ok());
        assert!(parse_duration("bad").is_err());
        assert!(parse_duration("0s").is_err());
    }

    #[test]
    fn parse_duration_rejects_overflow_and_rounds_that_become_zero() {
        assert!(parse_duration(
            "999999999999999999999999999999999999999999999999999999999999999999999999999999999999999h"
        )
        .is_err());
        assert!(parse_duration("0.0000000001ns").is_err());
    }

    #[test]
    fn auto_backends_fail_when_no_supported_command_exists() {
        let workspace = workspace_command(&WorkspaceAction {
            backend: "auto".to_string(),
            ..WorkspaceAction::default()
        });
        let monitor = monitor_command(&MonitorAction {
            backend: "auto".to_string(),
            ..MonitorAction::default()
        });
        let lock = lock_command(&LockAction {
            backend: "auto".to_string(),
            ..LockAction::default()
        });

        for command in [workspace, monitor, lock] {
            assert!(command.display().contains("exit 127"));
        }
    }

    #[test]
    fn preview_matches_expected_commands() {
        let mut config = Config::default();
        config.actions.lock.enabled = true;
        config.actions.lock.backend = "loginctl".to_string();
        config.actions.kill.enabled = true;
        config.actions.kill.processes = vec!["firefox".to_string(), "telegram-desktop".to_string()];
        config.actions.custom.enabled = true;
        config.actions.custom.commands = vec!["echo done".to_string()];

        let preview = preview_commands(&config);
        assert_eq!(preview.len(), 6);
        assert_eq!(preview[1], "playerctl stop");
        assert_eq!(preview[2], "loginctl lock-session");
        assert_eq!(preview[3], "pkill -- firefox");
        assert_eq!(preview[4], "pkill -- telegram-desktop");
        assert_eq!(preview[5], "echo done");
    }

    #[test]
    fn notify_message_with_quote_stays_single_argument() {
        let mut config = quiet_config("25m");
        config.actions.notify.enabled = true;
        config.actions.notify.message = "it's done".to_string();

        let commands = build_action_commands(&config);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].label, "notify");
        assert!(commands[0].shell.is_none());
        assert_eq!(
            commands[0].args,
            vec![
                "notify-send".to_string(),
                "gosleep-timer".to_string(),
                "--".to_string(),
                "it's done".to_string(),
            ]
        );
        assert_eq!(
            commands[0].display(),
            "notify-send gosleep-timer -- 'it'\\''s done'"
        );
        assert_eq!(preview_commands(&config), vec![commands[0].display()]);
    }

    #[test]
    fn kill_process_with_leading_dash_uses_separator() {
        let mut config = quiet_config("25m");
        config.actions.kill.enabled = true;
        config.actions.kill.processes = vec!["-evil".to_string()];

        let commands = build_action_commands(&config);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            vec!["pkill".to_string(), "--".to_string(), "-evil".to_string(),]
        );
        assert!(commands[0].shell.is_none());
    }

    #[test]
    fn dangerous_flags_cover_shell_overrides_without_notify_false_positives() {
        let notify = ActionCommand {
            label: "notify",
            args: vec![
                "notify-send".to_string(),
                "gosleep-timer".to_string(),
                "--".to_string(),
                "please rm -rf /".to_string(),
            ],
            shell: None,
        };
        assert!(!notify.is_dangerous());

        let custom = ActionCommand {
            label: "custom",
            args: vec![],
            shell: Some("echo done".to_string()),
        };
        assert!(custom.is_dangerous());

        for label in ["workspace", "monitor", "lock"] {
            let command = ActionCommand {
                label,
                args: vec![],
                shell: Some("my-custom-command".to_string()),
            };
            assert!(command.is_dangerous(), "{label} override must be flagged");
        }

        let workspace_args = workspace_command(&WorkspaceAction {
            backend: "sway".to_string(),
            number: 3,
            ..WorkspaceAction::default()
        });
        assert!(workspace_args.shell.is_none());
        assert!(!workspace_args.is_dangerous());

        let media = ActionCommand {
            label: "media",
            args: vec!["playerctl".to_string(), "stop".to_string()],
            shell: None,
        };
        assert!(!media.is_dangerous());
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

        let report = run_timer(
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
        assert!(text.contains("run: false"));
        assert!(text.contains("run: true"));
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].contains("custom failed"));
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
            "duration: 25m\nactions:\n  power:\n    mode: bogus\n",
        )
        .unwrap();

        let error = ensure_config(&path).unwrap_err();
        fs::remove_file(path).ok();

        assert!(error.to_string().contains("invalid power mode"));
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
    fn kde_monitor_uses_dpms_off_and_differs_from_lock() {
        let monitor = MonitorAction {
            backend: "kde".to_string(),
            ..MonitorAction::default()
        };
        let lock = LockAction {
            backend: "kde".to_string(),
            ..LockAction::default()
        };

        assert_eq!(
            monitor_command(&monitor).display(),
            "kscreen-doctor --dpms off"
        );
        assert_ne!(
            monitor_command(&monitor).display(),
            lock_command(&lock).display()
        );
    }

    #[test]
    fn sway_and_i3_workspace_use_workspace_number() {
        let sway = WorkspaceAction {
            backend: "sway".to_string(),
            number: 3,
            ..WorkspaceAction::default()
        };
        let i3 = WorkspaceAction {
            backend: "i3".to_string(),
            number: 3,
            ..WorkspaceAction::default()
        };

        assert_eq!(
            workspace_command(&sway).display(),
            "swaymsg workspace number 3"
        );
        assert_eq!(
            workspace_command(&i3).display(),
            "i3-msg workspace number 3"
        );
    }

    #[test]
    fn discord_defaults_to_enabled_without_application_id() {
        let config = Config::default();
        assert!(config.discord.enabled);
        assert!(config.discord.application_id.is_empty());
        assert!(!config.discord.should_connect());
    }

    #[test]
    fn old_config_without_discord_section_loads_defaults() {
        let config: Config =
            noyalib::from_str("duration: 25m\ninhibit_sleep: false\nactions: {}\n").unwrap();
        assert!(config.discord.enabled);
        assert!(config.discord.application_id.is_empty());
        assert!(!config.discord.should_connect());

        let configured: Config = noyalib::from_str(
            "duration: 25m\ndiscord:\n  enabled: false\n  application_id: \"123\"\n",
        )
        .unwrap();
        assert!(!configured.discord.enabled);
        assert_eq!(configured.discord.application_id, "123");
        assert!(!configured.discord.should_connect());

        let mut enabled = configured.clone();
        enabled.discord.enabled = true;
        assert!(enabled.discord.should_connect());
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
    fn discord_activity_payload_formats_details_state_and_timestamps() {
        let activity = PresenceActivity {
            details: "Sleep timer".to_string(),
            state: "45m until sleep".to_string(),
            start_timestamp: Some(1_700_000_000),
            end_timestamp: Some(1_700_002_700),
        };
        let payload = presence_activity_payload(&activity, 1234, "nonce-1");
        assert_eq!(payload["cmd"], "SET_ACTIVITY");
        assert_eq!(payload["args"]["pid"], 1234);
        assert_eq!(payload["args"]["activity"]["details"], "Sleep timer");
        assert_eq!(payload["args"]["activity"]["state"], "45m until sleep");
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["start"],
            1_700_000_000
        );
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["end"],
            1_700_002_700
        );
        assert_eq!(payload["nonce"], "nonce-1");

        let minimal = PresenceActivity {
            details: "Sleep timer".to_string(),
            state: "paused".to_string(),
            start_timestamp: None,
            end_timestamp: None,
        };
        let payload = presence_activity_payload(&minimal, 1234, "nonce-2");
        assert_eq!(payload["args"]["activity"]["details"], "Sleep timer");
        assert_eq!(payload["args"]["activity"]["state"], "paused");
        assert!(
            payload["args"]["activity"].get("timestamps").is_none(),
            "paused state must not leave a stale countdown timestamp"
        );

        let clear = presence_clear_payload(1234, "nonce-3");
        assert_eq!(clear["cmd"], "SET_ACTIVITY");
        assert!(clear["args"]["activity"].is_null());
        assert_eq!(clear["nonce"], "nonce-3");
    }

    #[test]
    fn discord_ipc_frame_encodes_opcode_and_length_little_endian() {
        let frame = encode_discord_ipc_frame(1, b"{}");
        assert_eq!(frame, vec![1, 0, 0, 0, 2, 0, 0, 0, b'{', b'}']);

        let handshake = encode_discord_ipc_frame(0, b"hi");
        assert_eq!(&handshake[0..4], &0_u32.to_le_bytes());
        assert_eq!(&handshake[4..8], &2_u32.to_le_bytes());
        assert_eq!(&handshake[8..], b"hi");
    }

    #[test]
    fn discord_ipc_candidates_cover_runtime_and_tmp_sockets() {
        let bases = vec![PathBuf::from("/run/user/1000"), PathBuf::from("/tmp")];
        let candidates = discord_ipc_candidates_from(&bases);
        assert_eq!(candidates.len(), 20);
        assert_eq!(candidates[0], PathBuf::from("/run/user/1000/discord-ipc-0"));
        assert_eq!(candidates[9], PathBuf::from("/run/user/1000/discord-ipc-9"));
        assert_eq!(candidates[10], PathBuf::from("/tmp/discord-ipc-0"));
        assert_eq!(candidates[19], PathBuf::from("/tmp/discord-ipc-9"));
    }

    #[test]
    fn discord_disabled_or_unconfigured_is_noop_without_blocking() {
        for config in [
            DiscordConfig {
                enabled: false,
                application_id: "123".to_string(),
            },
            DiscordConfig {
                enabled: true,
                application_id: String::new(),
            },
            DiscordConfig {
                enabled: true,
                application_id: "   ".to_string(),
            },
        ] {
            assert!(!config.should_connect());
            let started = Instant::now();
            let presence = DiscordPresence::start(&config);
            assert!(!presence.is_active());
            presence.update(PresenceActivity {
                details: "Sleep timer".to_string(),
                state: "45m until sleep".to_string(),
                start_timestamp: None,
                end_timestamp: None,
            });
            presence.clear();
            drop(presence);
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "disabled/unconfigured presence must be an instant no-op"
            );
        }
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
            state: "10ms until sleep".to_string(),
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
        let report = run_timer(
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
    fn countdown_presence_shows_remaining_with_end_timestamp() {
        let remaining = Duration::from_secs(45 * 60);
        let activity = countdown_presence(remaining, "none", 1_700_000_000);
        assert_eq!(activity.details, "Sleep timer");
        assert_eq!(
            activity.state,
            format!("{} until sleep", format_duration(remaining))
        );
        assert_eq!(activity.start_timestamp, None);
        assert_eq!(
            activity.end_timestamp,
            Some(1_700_000_000 + remaining.as_secs())
        );

        // Payload keeps the live countdown end timestamp.
        let payload = presence_activity_payload(&activity, 1234, "nonce-c1");
        assert_eq!(
            payload["args"]["activity"]["timestamps"]["end"],
            1_700_000_000 + remaining.as_secs()
        );
    }

    #[test]
    fn paused_presence_has_no_stale_countdown_timestamp() {
        let activity = paused_presence("none");
        assert_eq!(activity.details, "Sleep timer");
        assert_eq!(activity.state, "paused");
        assert_eq!(activity.start_timestamp, None);
        assert_eq!(activity.end_timestamp, None);

        let payload = presence_activity_payload(&activity, 1234, "nonce-p1");
        assert!(
            payload["args"]["activity"].get("timestamps").is_none(),
            "paused state must not leave a stale countdown timestamp"
        );
    }

    #[test]
    fn poweroff_indicator_only_for_exact_poweroff_mode() {
        let running_off = countdown_presence(Duration::from_secs(60), "poweroff", 1000);
        assert!(
            running_off.state.contains("Power off after timer"),
            "poweroff mode must show the indicator, got {:?}",
            running_off.state
        );
        let paused_off = paused_presence("poweroff");
        assert!(paused_off.state.contains("Power off after timer"));

        for mode in [
            "none",
            "suspend",
            "hibernate",
            "hybrid-sleep",
            "reboot",
            "",
            "PowerOff",
            "POWEROFF",
        ] {
            let running = countdown_presence(Duration::from_secs(60), mode, 1000);
            assert!(
                !running.state.contains("Power off after timer"),
                "mode {mode:?} must omit the indicator, got {:?}",
                running.state
            );
            let paused = paused_presence(mode);
            assert!(
                !paused.state.contains("Power off after timer"),
                "mode {mode:?} must omit the indicator when paused, got {:?}",
                paused.state
            );
        }
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
        // --no-actions and dry-run behavior on both paths.
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
            let report = run_timer(
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
        }
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
