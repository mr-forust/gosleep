use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod actions;
mod cli;
mod config;
mod discord;
mod timer;
mod tui;

use crate::cli::{
    edit_config, handle_history, handle_init, handle_preview, handle_run, handle_stats,
    handle_validate, print_status,
};
use crate::config::{default_config_path, ensure_config};
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
        }) => handle_run(&mut config, &path, dry_run, no_actions, duration),
        Some(Commands::Status) => {
            print_status(&path, &config);
            Ok(())
        }
        Some(Commands::Validate) => handle_validate(&path, &config),
        Some(Commands::Edit) => edit_config(&path),
        Some(Commands::History) => handle_history(&path),
        Some(Commands::Stats) => handle_stats(&path),
        Some(Commands::Preview) => {
            handle_preview(&config);
            Ok(())
        }
        Some(Commands::Init) => {
            handle_init(&path);
            Ok(())
        }
        None => run_tui(path, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::discord::{
        DiscordConfig, DiscordPresence, PresenceActivity, countdown_presence,
        post_countdown_presence,
    };
    use crate::timer::{RunMode, TIMER_STOPPED_MSG, TimerOptions, run_timer};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64},
        },
        time::{Duration, Instant},
    };

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
