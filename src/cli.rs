use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::actions::{build_action_commands, preview_commands};
use crate::config::{Config, ensure_config, validate_config};
use crate::timer::{HistoryEntry, RunMode, TimerOptions, format_duration, run_timer};

pub(crate) fn handle_run(
    config: &mut Config,
    config_path: &Path,
    dry_run: bool,
    no_actions: bool,
    duration: Option<String>,
) -> Result<()> {
    if let Some(duration) = duration {
        config.duration = duration;
    }
    let mut stdout = io::stdout();
    let options = TimerOptions {
        dry_run,
        no_actions,
    };
    let (report, _sleep_presence) = run_timer(
        config,
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
    let slept_seconds = prompt_wake_confirmation(config, options)?;
    let status = if report.failures.is_empty() {
        "finished"
    } else {
        "partial"
    };
    append_history(
        &history_path(config_path),
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

pub(crate) fn handle_validate(path: &Path, config: &Config) -> Result<()> {
    validate_config(config)?;
    println!("valid {}", path.display());
    Ok(())
}

pub(crate) fn handle_preview(config: &Config) {
    for command in preview_commands(config) {
        println!("{command}");
    }
}

pub(crate) fn handle_init(path: &Path) {
    println!("{}", path.display());
}

pub(crate) fn handle_history(config_path: &Path) -> Result<()> {
    print_history(&history_path(config_path))
}

pub(crate) fn handle_stats(config_path: &Path) -> Result<()> {
    print_stats(&history_path(config_path))
}

pub(crate) fn history_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("history.yaml")
}

pub(crate) fn read_history(path: &Path) -> Result<Vec<HistoryEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    noyalib::from_str(&data).with_context(|| format!("parse {}", path.display()))
}

pub(crate) fn append_history(path: &Path, entry: HistoryEntry) -> Result<()> {
    let mut entries = read_history(path)?;
    entries.push(entry);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let data = noyalib::to_string(&entries).context("serialize history")?;
    fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub(crate) fn print_status(path: &Path, config: &Config) {
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

pub(crate) fn print_history(path: &Path) -> Result<()> {
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

pub(crate) fn print_stats(path: &Path) -> Result<()> {
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

pub(crate) fn edit_config(path: &Path) -> Result<()> {
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

pub(crate) fn prompt_wake_confirmation(
    config: &Config,
    options: TimerOptions,
) -> Result<Option<u64>> {
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
