use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, NaiveTime, TimeZone};

use crate::config::Config;

pub(crate) const GROUPS: &[&str] = &["Timer", "Audio", "Screen", "Session", "More"];

pub(crate) fn field_group(key: FieldKey) -> usize {
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

pub(crate) fn field_hint(field: Field) -> &'static str {
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

#[derive(Clone, Copy)]
pub(crate) struct Field {
    pub(crate) label: &'static str,
    pub(crate) key: FieldKey,
    pub(crate) kind: FieldKind,
}

#[derive(Clone, Copy)]
pub(crate) enum FieldKind {
    Edit,
    Bool,
    Cycle(&'static [&'static str]),
    Int,
    Csv,
    Semi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FieldKey {
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

pub(crate) const FIELDS: &[Field] = &[
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

pub(crate) fn visible_fields(config: &Config) -> Vec<&'static Field> {
    FIELDS
        .iter()
        .filter(|field| field_visible(config, field.key))
        .collect()
}

pub(crate) fn field_visible(config: &Config, key: FieldKey) -> bool {
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

pub(crate) fn on_off(value: bool) -> String {
    if value { "on" } else { "off" }.to_string()
}

pub(crate) fn parse_clock_time(value: &str) -> Result<NaiveTime> {
    let value = value.trim();
    if value.len() != 5 || value.as_bytes().get(2).is_none_or(|ch| *ch != b':') {
        bail!("expected local time HH:MM (00:00..23:59)");
    }
    NaiveTime::parse_from_str(value, "%H:%M").context("expected local time HH:MM (00:00..23:59)")
}

pub(crate) fn duration_until<T: TimeZone>(time: NaiveTime, now: DateTime<T>) -> Result<Duration> {
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

pub(crate) struct EditState {
    pub(crate) field: Field,
    pub(crate) value: String,
    pub(crate) cursor: usize,
}

pub(crate) fn split_list(value: &str, separator: char) -> Vec<String> {
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

    fn visible_field_keys(config: &Config) -> Vec<FieldKey> {
        visible_fields(config)
            .into_iter()
            .map(|field| field.key)
            .collect()
    }

    #[test]
    fn split_list_trims_empty_values() {
        assert_eq!(
            split_list("firefox, telegram-desktop, ", ','),
            vec!["firefox", "telegram-desktop"]
        );
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
}
