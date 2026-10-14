use std::{
    io::{self, Write},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::actions::{ActionCommand, build_action_commands};
use crate::config::{Config, parse_duration, validate_config};
use crate::discord::{DiscordPresence, countdown_presence, paused_presence, post_run_presence};

pub(crate) const TIMER_STOPPED_MSG: &str = "timer stopped";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HistoryEntry {
    pub(crate) started_at: u64,
    pub(crate) duration_seconds: u64,
    pub(crate) slept_seconds: Option<u64>,
    pub(crate) status: String,
    pub(crate) actions: Vec<String>,
    pub(crate) failures: Vec<String>,
    pub(crate) dry_run: bool,
    pub(crate) no_actions: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct TimerReport {
    pub(crate) started_at: u64,
    pub(crate) duration: Duration,
    pub(crate) sleep_started_at: Option<u64>,
    pub(crate) actions: Vec<String>,
    pub(crate) failures: Vec<String>,
    pub(crate) dry_run: bool,
    pub(crate) no_actions: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TimerOptions {
    pub(crate) dry_run: bool,
    pub(crate) no_actions: bool,
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

impl TimerReport {
    pub(crate) fn to_history(&self, status: &str, slept_seconds: Option<u64>) -> HistoryEntry {
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

pub(crate) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn format_duration(duration: Duration) -> String {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunMode {
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

pub(crate) fn run_timer(
    config: &Config,
    stop: &Arc<AtomicBool>,
    pause: &Arc<AtomicBool>,
    remaining_ms: &Arc<AtomicU64>,
    mode: RunMode,
    options: TimerOptions,
    mut output: Option<&mut dyn Write>,
) -> Result<(TimerReport, DiscordPresence)> {
    validate_config(config)?;
    let duration = parse_duration(&config.duration)?;
    // Local IPC only: no bot token, network service, or credentials. The
    // worker connects in the background; missing Discord never fails/stalls.
    // Countdown updates (US-003) are published from this thread for both CLI
    // and TUI runs, including --no-actions and dry-run. Stopped runs clear
    // explicitly. Successful runs transition to the post-countdown state and
    // disarm the drop-clear; the returned guard keeps the IPC connection open
    // so US-004 retains the green elapsed timer while the TUI stays open and during
    // the CLI wake-confirmation wait. Discord clears it on process exit/poweroff.
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
    // Elapsed sleep time is measured from the start of post-countdown actions.
    let sleep_start_unix = unix_timestamp();
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
    // Successful completion transitions instead of clearing. Only a real,
    // fully successful action run represents a sleep interval, so only then
    // publish the elapsed start timestamp without state text. Dry-run,
    // --no-actions, failures, and empty action lists keep the accurate
    // timestamp-free `finished` state. The disarmed guard is returned so the
    // caller retains the status; Discord clears it on process exit/poweroff.
    let completed = post_run_presence(
        options.dry_run,
        options.no_actions,
        !failures.is_empty(),
        !actions.is_empty(),
        sleep_start_unix,
    );
    let sleep_started_at = completed.start_timestamp;
    discord.update(completed);
    discord.disarm();
    let report = TimerReport {
        started_at,
        duration,
        sleep_started_at,
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
    Ok((report, discord))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

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
    fn report_converts_to_history_entry() {
        let report = TimerReport {
            started_at: 1_700_000_000,
            duration: Duration::from_secs(90),
            sleep_started_at: None,
            actions: vec!["true".to_string()],
            failures: Vec::new(),
            dry_run: false,
            no_actions: false,
        };
        let entry = report.to_history("finished", Some(60));
        assert_eq!(entry.started_at, 1_700_000_000);
        assert_eq!(entry.duration_seconds, 90);
        assert_eq!(entry.slept_seconds, Some(60));
        assert_eq!(entry.status, "finished");
        assert_eq!(entry.actions, vec!["true".to_string()]);
        assert!(entry.failures.is_empty());
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

        let (report, _presence) = run_timer(
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
    fn tui_worker_reports_each_controlled_action_result() {
        let mut config = quiet_config("1ns");
        config.actions.custom.enabled = true;
        config.actions.custom.commands = vec!["exit 0".to_string(), "exit 7".to_string()];
        let mut output = Vec::new();
        let (report, _presence) = run_timer(
            &config,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicU64::new(0)),
            RunMode::Tui,
            TimerOptions::default(),
            Some(&mut output),
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("custom: done"));
        assert!(text.contains("custom failed:"));
        assert_eq!(report.failures.len(), 1);
        assert!(!text.contains("remaining:"));
    }
}
