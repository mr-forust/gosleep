use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) duration: String,
    pub(crate) inhibit_sleep: bool,
    pub(crate) actions: Actions,
    pub(crate) discord: DiscordConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct Actions {
    pub(crate) workspace: WorkspaceAction,
    pub(crate) media: MediaAction,
    pub(crate) brightness: BrightnessAction,
    pub(crate) mute: ToggleAction,
    pub(crate) mute_input: ToggleAction,
    pub(crate) lock: LockAction,
    pub(crate) monitor: MonitorAction,
    pub(crate) notify: NotifyAction,
    pub(crate) kill: KillAction,
    pub(crate) power: PowerAction,
    pub(crate) custom: CustomAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceAction {
    pub(crate) enabled: bool,
    pub(crate) backend: String,
    pub(crate) number: u32,
    pub(crate) command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MediaAction {
    pub(crate) enabled: bool,
    pub(crate) action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct BrightnessAction {
    pub(crate) enabled: bool,
    pub(crate) value: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct ToggleAction {
    pub(crate) enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct LockAction {
    pub(crate) enabled: bool,
    pub(crate) backend: String,
    pub(crate) command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MonitorAction {
    pub(crate) enabled: bool,
    pub(crate) backend: String,
    pub(crate) command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct NotifyAction {
    pub(crate) enabled: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct KillAction {
    pub(crate) enabled: bool,
    pub(crate) processes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PowerAction {
    pub(crate) mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub(crate) struct CustomAction {
    pub(crate) enabled: bool,
    pub(crate) commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DiscordConfig {
    pub(crate) enabled: bool,
    pub(crate) application_id: String,
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
    pub(crate) fn should_connect(&self) -> bool {
        self.enabled && !self.application_id.trim().is_empty()
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

pub(crate) fn default_config_path() -> PathBuf {
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

pub(crate) fn ensure_config(path: &Path) -> Result<Config> {
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

pub(crate) fn save_config(path: &Path, config: &Config) -> Result<()> {
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

pub(crate) fn normalize_config(config: &mut Config) {
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

pub(crate) fn validate_config(config: &Config) -> Result<()> {
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
        | "xscreensaver" | "noctalia-qs" | "noctalia" => {}
        backend => bail!("invalid lock backend {backend:?}"),
    }
    match config.actions.media.action.as_str() {
        "none" | "stop" | "pause" | "play-pause" | "next" | "previous" => {}
        action => bail!("invalid media action {action:?}"),
    }
    Ok(())
}

pub(crate) fn parse_duration(value: &str) -> Result<Duration> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
