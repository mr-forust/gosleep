use std::time::Duration;

use crate::timer::format_duration;

pub(crate) fn format_compact_duration(duration: Duration) -> String {
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

pub(crate) fn format_time_left_text(is_running: bool, remaining: Duration, width: u16) -> String {
    if !is_running {
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

pub(crate) fn format_gauge_label(progress: f64) -> String {
    format!("{:>3}%", (progress * 100.0).round() as u16)
}

pub(crate) fn ascii_time(value: &str) -> Vec<String> {
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

pub(crate) fn ascii_glyph(ch: char) -> [&'static str; 7] {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_header_text_compacts_for_narrow_widths() {
        let remaining = Duration::from_secs(65);

        assert_eq!(format_time_left_text(false, remaining, 20), "not running");
        assert_eq!(
            format_time_left_text(true, remaining, 40),
            "time left: 1m5s"
        );
        assert_eq!(format_time_left_text(true, remaining, 12), "left: 01:05");
        assert_eq!(format_time_left_text(true, remaining, 5), "01:05");
    }

    #[test]
    fn gauge_label_is_short_percent_only() {
        assert_eq!(format_gauge_label(0.0), "  0%");
        assert_eq!(format_gauge_label(0.58), " 58%");
        assert_eq!(format_gauge_label(1.0), "100%");
    }

    #[test]
    fn ascii_time_rows_have_equal_width() {
        let rows = ascii_time("14:45:27");
        let width = rows.first().map(String::len).unwrap_or(0);

        assert_eq!(rows.len(), 7);
        assert!(width > 0);
        assert!(rows.iter().all(|row| row.len() == width));
    }
}
