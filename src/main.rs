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
use clap::{Parser, Subcommand};

mod actions;
mod config;
mod discord;
mod timer;
mod tui;

use crate::actions::{build_action_commands, preview_commands};
use crate::config::{Config, default_config_path, ensure_config, validate_config};
use crate::timer::{HistoryEntry, RunMode, TimerOptions, format_duration, run_timer};
use crate::tui::run_tui;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord::{
        DiscordConfig, DiscordPresence, PresenceActivity, countdown_presence,
        post_countdown_presence,
    };
    use crate::timer::TIMER_STOPPED_MSG;

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
}
