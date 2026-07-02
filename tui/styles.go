package tui

import (
    "github.com/charmbracelet/lipgloss"
)

var (
    AppStyle = lipgloss.NewStyle().
        Padding(1, 2).
        Align(lipgloss.Center)

    TitleStyle = lipgloss.NewStyle().
        Bold(true).
        Foreground(lipgloss.Color("#33ccff")).
        Align(lipgloss.Center).
        Padding(0, 1)

    LabelStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#33ccff")).
        Padding(0, 1)

    StatusStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#00ff88")).
        Align(lipgloss.Center).
        PaddingTop(1)

    ErrorStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#ff4444")).
        Align(lipgloss.Center)

    TimerStyle = lipgloss.NewStyle().
        Bold(true).
        Foreground(lipgloss.Color("#ffffff")).
        Background(lipgloss.Color("#33ccff")).
        Align(lipgloss.Center).
        Padding(0, 2)

    ProgressBarStyle = lipgloss.NewStyle().
        Height(1)

    HelpStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#888888"))

    ActiveModuleStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#00ff88"))

    InactiveModuleStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#666666"))

    BorderStyle = lipgloss.NewStyle().
        Border(lipgloss.RoundedBorder()).
        BorderForeground(lipgloss.Color("#33ccff")).
        Padding(1, 2)

    ButtonStyle = lipgloss.NewStyle().
        Bold(true).
        Padding(0, 2)

    SuccessButtonStyle = ButtonStyle.Copy().
        Foreground(lipgloss.Color("#ffffff")).
        Background(lipgloss.Color("#00aa44"))

    DangerButtonStyle = ButtonStyle.Copy().
        Foreground(lipgloss.Color("#ffffff")).
        Background(lipgloss.Color("#cc3333"))

    DimmedStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("#444444"))
)
