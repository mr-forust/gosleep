use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::actions::{ActionCommand, build_action_commands, escape_terminal_text};
use crate::config::Config;

use super::fields::EditState;

pub(crate) fn action_description(command: &ActionCommand, config: &Config) -> String {
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

pub(crate) fn edit_view(edit: &EditState, width: usize) -> String {
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

pub(crate) fn center_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) fn action_plan_lines(config: &Config, width: usize) -> Vec<Line<'static>> {
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

pub(crate) fn wrap_command(text: &str, width: usize) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::text::Line;

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
    fn wrap_command_keeps_lines_under_width() {
        let lines = wrap_command(
            "if command -v niri >/dev/null 2>&1; then niri msg action focus-workspace 3",
            24,
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| line.len() <= 24));
    }
}
