use crate::config::{Config, LockAction, MonitorAction, WorkspaceAction, normalize_config};

#[derive(Debug, Clone)]
pub(crate) struct ActionCommand {
    pub(crate) label: &'static str,
    pub(crate) args: Vec<String>,
    pub(crate) shell: Option<String>,
}

impl ActionCommand {
    pub(crate) fn display(&self) -> String {
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

    pub(crate) fn is_dangerous(&self) -> bool {
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

pub(crate) fn is_terminal_control(ch: char) -> bool {
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

pub(crate) fn escape_terminal_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        append_terminal_safe_char(&mut escaped, ch);
    }
    escaped
}

pub(crate) fn display_argument(argument: &str) -> String {
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

pub(crate) fn build_action_commands(config: &Config) -> Vec<ActionCommand> {
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

pub(crate) fn workspace_command(action: &WorkspaceAction) -> ActionCommand {
    if action.backend == "auto" && !action.command.is_empty() {
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

pub(crate) fn monitor_command(action: &MonitorAction) -> ActionCommand {
    if action.backend == "auto" && !action.command.is_empty() {
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
            args: vec!["swaymsg", "output", "*", "power", "off"]
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
                "if command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch dpms off; elif command -v niri >/dev/null 2>&1; then niri msg action power-off-monitors; elif command -v swaymsg >/dev/null 2>&1; then swaymsg output '*' power off; elif command -v kscreen-doctor >/dev/null 2>&1; then kscreen-doctor --dpms off; elif command -v xset >/dev/null 2>&1; then xset dpms force off; else echo 'no supported monitor power-off backend found' >&2; exit 127; fi".to_string()
            ),
        },
    }
}

pub(crate) fn lock_command(action: &LockAction) -> ActionCommand {
    if action.backend == "auto" && !action.command.is_empty() {
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
                "org.freedesktop.ScreenSaver".to_string(),
                "/ScreenSaver".to_string(),
                "org.freedesktop.ScreenSaver.Lock".to_string(),
            ],
            shell: None,
        },
        "noctalia-qs" => ActionCommand {
            label: "lock",
            args: vec![
                "qs".to_string(),
                "-c".to_string(),
                "noctalia-shell".to_string(),
                "ipc".to_string(),
                "call".to_string(),
                "lockScreen".to_string(),
                "lock".to_string(),
            ],
            shell: None,
        },
        "noctalia" => ActionCommand {
            label: "lock",
            args: vec![
                "noctalia".to_string(),
                "msg".to_string(),
                "session".to_string(),
                "lock".to_string(),
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
                "if command -v loginctl >/dev/null 2>&1; then loginctl lock-session; elif command -v hyprctl >/dev/null 2>&1; then hyprctl dispatch exec hyprlock; elif command -v swaylock >/dev/null 2>&1; then swaylock; elif command -v i3lock >/dev/null 2>&1; then i3lock; elif command -v gnome-screensaver-command >/dev/null 2>&1; then gnome-screensaver-command -l; elif command -v qdbus >/dev/null 2>&1; then qdbus org.freedesktop.ScreenSaver /ScreenSaver org.freedesktop.ScreenSaver.Lock; elif command -v xscreensaver-command >/dev/null 2>&1; then xscreensaver-command -lock; else echo 'no supported lock backend found' >&2; exit 127; fi".to_string()
            ),
        },
    }
}

pub(crate) fn preview_commands(config: &Config) -> Vec<String> {
    build_action_commands(config)
        .iter()
        .map(ActionCommand::display)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::validate_config;

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
    fn noctalia_lock_backends_use_their_supported_ipc_commands() {
        let cases = [
            (
                "noctalia-qs",
                "qs -c noctalia-shell ipc call lockScreen lock",
            ),
            ("noctalia", "noctalia msg session lock"),
        ];

        for (backend, expected) in cases {
            let action = LockAction {
                backend: backend.to_string(),
                ..LockAction::default()
            };
            let command = lock_command(&action);
            assert_eq!(command.display(), expected);
            assert!(command.shell.is_none());

            let mut config = quiet_config("25m");
            config.actions.lock.backend = backend.to_string();
            assert!(validate_config(&config).is_ok());
        }
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
}
