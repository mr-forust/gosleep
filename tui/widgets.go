package tui

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
)

// ProgressBar renders a progress bar with given width, percent (0.0-1.0)
func ProgressBar(width int, percent float64, label string) string {
	if width < 10 {
		width = 10
	}
	filled := int(float64(width) * percent)
	if filled > width {
		filled = width
	}
	empty := width - filled

	bar := strings.Repeat("█", filled) + strings.Repeat("░", empty)
	bar = lipgloss.NewStyle().Foreground(lipgloss.Color("#33ccff")).Render(bar)

	pct := fmt.Sprintf("%3d%%", int(percent*100))
	return fmt.Sprintf("%s %s  %s", pct, bar, label)
}

// TimerBlock renders the main timer display (HH:MM:SS)
func TimerBlock(remaining time.Duration) string {
	totalSec := int(remaining.Seconds())
	h := totalSec / 3600
	m := (totalSec % 3600) / 60
	s := totalSec % 60

	timeStr := fmt.Sprintf("%02d:%02d:%02d", h, m, s)
	return TimerStyle.Render(timeStr)
}

// ModuleCheckbox renders a module toggle line
func ModuleCheckbox(name string, enabled bool, checked bool) string {
	checkmark := " "
	if checked {
		checkmark = "●"
	}
	style := InactiveModuleStyle
	if enabled {
		style = ActiveModuleStyle
	}
	return style.Render(fmt.Sprintf("[ %s ] %s", checkmark, name))
}

// StatusLine renders a status message
func StatusLine(msg string, isError bool) string {
	if isError {
		return ErrorStyle.Render(msg)
	}
	return StatusStyle.Render(msg)
}

// HelpBar renders the bottom help bar with keybindings
func HelpBar(items ...string) string {
	return HelpStyle.Render(strings.Join(items, "  ·  "))
}

// InputField renders a labeled input field
func InputField(label, value, placeholder string, focused bool) string {
	style := LabelStyle
	borderColor := "#444"
	if focused {
		borderColor = "#33ccff"
	}
	val := value
	if val == "" {
		val = placeholder
		style = DimmedStyle
	}
	inputBox := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(lipgloss.Color(borderColor)).
		Width(30).
		Render(val)

	return fmt.Sprintf("%s\n%s", style.Render(label), inputBox)
}

// SelectWidget renders a simple select list
func SelectWidget(label string, options []string, selected int) string {
	var b strings.Builder
	b.WriteString(LabelStyle.Render(label))
	b.WriteString("\n")
	for i, opt := range options {
		if i == selected {
			b.WriteString(lipgloss.NewStyle().
				Foreground(lipgloss.Color("#33ccff")).
				Bold(true).
				Render("▸ " + opt))
		} else {
			b.WriteString("  " + opt)
		}
		b.WriteString("\n")
	}
	return b.String()
}
