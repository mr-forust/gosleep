package tui

import (
	"context"
	"fmt"
	"strings"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/engine"
	"github.com/forust/gosleep-timer/history"
	"github.com/forust/gosleep-timer/modules"
	"github.com/forust/gosleep-timer/profiles"
)

type screen int

const (
	screenMain screen = iota
	screenRunning
	screenHistory
	screenStats
	screenQR
	screenHelp
)

type focusField int

const (
	focusTimeInput focusField = iota
	focusProfile
	focusStartButton
	focusStopButton
)

type model struct {
	cfg   *config.Config
	pMgr  *profiles.Manager

	// UI state
	screen    screen
	focus     focusField
	err       error

	// Timer input
	timeInput string

	// Profile
	profiles    []string
	selProfile  int

	// Running timer
	timer        *engine.Timer
	stageRunner  *engine.StageRunner
	remaining    time.Duration
	total        time.Duration
	currentStage engine.StageType
	timerStatus  engine.Status
	statusMsg    string

	// History
	records    []history.Record
	histLog    *history.Log

	// Module toggles (map profileName -> map moduleName -> enabled)
	moduleToggles map[string]map[string]bool
}

func NewModel(cfg *config.Config) *model {
	pMgr := profiles.NewManager(cfg)
	profilesList := pMgr.List()
	if len(profilesList) == 0 {
		profilesList = []string{"default"}
	}

	// Init module toggles
	toggles := make(map[string]map[string]bool)
	for _, name := range profilesList {
		toggles[name] = initModuleToggles(cfg, name)
	}

	// Open history log
	histPath, err := history.DefaultPath()
	histLog := &history.Log{}
	if err == nil {
		if l, err := history.Open(histPath); err == nil {
			histLog = l
		}
	}

	return &model{
		cfg:           cfg,
		pMgr:          pMgr,
		screen:        screenMain,
		profiles:      profilesList,
		selProfile:    0,
		timeInput:     "25m",
		timerStatus:   engine.StatusIdle,
		moduleToggles: toggles,
		histLog:       histLog,
	}
}

func initModuleToggles(cfg *config.Config, profileName string) map[string]bool {
	t := map[string]bool{
		"workspace":  true,
		"media":      true,
		"lock":       true,
		"kill":       false,
		"notify":     true,
		"sound":      true,
		"brightness": false,
		"mute":       false,
		"custom":     false,
		"script":     false,
	}
	p, ok := cfg.Profiles[profileName]
	if !ok || p.Modules == nil {
		return t
	}
	m := p.Modules
	if m.Workspace != nil { t["workspace"] = m.Workspace.Enabled }
	if m.Media != nil { t["media"] = m.Media.Enabled }
	if m.Lock != nil { t["lock"] = m.Lock.Enabled }
	if m.Kill != nil { t["kill"] = m.Kill.Enabled }
	if m.Notify != nil { t["notify"] = m.Notify.Enabled }
	if m.Sound != nil { t["sound"] = m.Sound.Enabled }
	if m.Brightness != nil { t["brightness"] = m.Brightness.Enabled }
	if m.Mute != nil { t["mute"] = m.Mute.Enabled }
	if m.Custom != nil { t["custom"] = m.Custom.Enabled }
	if m.Script != nil { t["script"] = m.Script.Enabled }
	return t
}

func (m *model) Init() tea.Cmd {
	return nil
}

// Messages
type timerTickMsg struct {
	remaining time.Duration
	total     time.Duration
	status    engine.Status
	stage     engine.StageType
	done      bool
}

type timerDoneMsg struct {
	status engine.Status
}

type errorMsg struct {
	err error
}

func (m *model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		return m.handleKey(msg)
	case timerTickMsg:
		m.remaining = msg.remaining
		m.total = msg.total
		m.timerStatus = msg.status
		m.currentStage = msg.stage
		if msg.done {
			m.screen = screenMain
			m.timerStatus = engine.StatusIdle
			m.statusMsg = "✅ Timer completed"
			m.recordRun("completed", "")
			return m, nil
		}
		return m, nil
	case timerDoneMsg:
		m.screen = screenMain
		m.timerStatus = engine.StatusIdle
		if msg.status == engine.StatusKilled {
			m.statusMsg = "⛔ Timer killed"
		}
		return m, nil
	case errorMsg:
		m.err = msg.err
		m.statusMsg = fmt.Sprintf("❌ %v", msg.err)
		m.screen = screenMain
		m.timerStatus = engine.StatusIdle
		return m, nil
	default:
		return m, nil
	}
}

func (m *model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch m.screen {
	case screenMain:
		return m.handleMainKey(msg)
	case screenRunning:
		return m.handleRunningKey(msg)
	case screenHistory:
		return m.handleHistoryKey(msg)
	case screenStats:
		return m.handleStatsKey(msg)
	case screenQR:
		return m.handleQRKey(msg)
	case screenHelp:
		return m.handleHelpKey(msg)
	}
	return m, nil
}

func (m *model) handleMainKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "ctrl+c", "esc":
		m.histLog.Close()
		return m, tea.Quit
	case "tab":
		m.focus++
		if m.focus > focusStopButton {
			m.focus = focusTimeInput
		}
	case "shift+tab":
		m.focus--
		if m.focus < focusTimeInput {
			m.focus = focusStopButton
		}
	case "enter", " ":
		switch m.focus {
		case focusTimeInput:
			m.focus = focusProfile
		case focusProfile:
			m.focus = focusStartButton
		case focusStartButton:
			return m, m.startTimer()
		case focusStopButton:
			m.stopTimer()
		}
	case "s":
		return m, m.startTimer()
	case "S":
		m.stopTimer()
	case "h":
		m.loadHistory()
		m.screen = screenHistory
	case "?":
		m.screen = screenHelp
	case "up", "k":
		m.selProfile--
		if m.selProfile < 0 {
			m.selProfile = len(m.profiles) - 1
		}
		m.loadModuleToggles()
	case "down", "j":
		m.selProfile++
		if m.selProfile >= len(m.profiles) {
			m.selProfile = 0
		}
		m.loadModuleToggles()
	case "1", "2", "3", "4", "5", "6", "7", "8", "9", "0":
		// Toggle modules by number (1-9,0=10)
		idx := int(msg.String()[0] - '1')
		if msg.String() == "0" {
			idx = 9
		}
		m.toggleModule(idx)
	default:
		// Handle time input when focused
		if m.focus == focusTimeInput {
			if msg.Type == tea.KeyBackspace || msg.Type == tea.KeyDelete {
				if len(m.timeInput) > 0 {
					m.timeInput = m.timeInput[:len(m.timeInput)-1]
				}
			} else if msg.Type == tea.KeyRunes {
				m.timeInput += msg.String()
			}
		}
	}
	return m, nil
}

func (m *model) loadModuleToggles() {
	name := m.profiles[m.selProfile]
	if _, ok := m.moduleToggles[name]; !ok {
		m.moduleToggles[name] = initModuleToggles(m.cfg, name)
	}
}

func (m *model) toggleModule(idx int) {
	modules := []string{"workspace", "media", "lock", "kill", "notify", "sound", "brightness", "mute", "custom", "script"}
	if idx < 0 || idx >= len(modules) {
		return
	}
	name := m.profiles[m.selProfile]
	key := modules[idx]
	if toggles, ok := m.moduleToggles[name]; ok {
		toggles[key] = !toggles[key]
	}
}

func (m *model) handleRunningKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "S", "s":
		m.stopTimer()
		m.screen = screenMain
		m.statusMsg = "⛔ Timer stopped"
		return m, nil
	case "q", "esc", "ctrl+c":
		m.stopTimer()
		m.screen = screenMain
		return m, nil
	}
	return m, nil
}

func (m *model) handleHistoryKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "esc", "h":
		m.screen = screenMain
	case "s":
		m.screen = screenStats
	}
	return m, nil
}

func (m *model) handleStatsKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "esc", "h":
		m.screen = screenMain
	}
	return m, nil
}

func (m *model) handleQRKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "esc":
		m.screen = screenMain
	}
	return m, nil
}

func (m *model) handleHelpKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "esc", "?":
		m.screen = screenMain
	}
	return m, nil
}

func (m *model) startTimer() tea.Cmd {
	d, err := engine.ParseDuration(m.timeInput)
	if err != nil {
		m.statusMsg = fmt.Sprintf("❌ Invalid time: %s", m.timeInput)
		return nil
	}
	if d <= 0 {
		m.statusMsg = "❌ Duration must be positive"
		return nil
	}

	profileName := m.profiles[m.selProfile]
	profile, err := m.pMgr.Get(profileName)
	if err != nil {
		m.statusMsg = fmt.Sprintf("❌ Profile error: %v", err)
		return nil
	}

	// Build module config based on toggles
	modCfg := m.buildModuleConfig(profile)

	// Collect commands from modules
	modCtx := &modules.ModuleContext{
		Context:  context.Background(),
		Profile:  profileName,
		Duration: m.timeInput,
		Config:   modCfg,
	}

	preCmds := modules.CollectPreCmds(modCtx)
	postCmds := modules.CollectPostCmds(modCtx)

	m.timer = engine.NewTimer(d)
	m.stageRunner = engine.NewStageRunner(m.timer, preCmds, postCmds)
	m.screen = screenRunning

	// Start runner in background
	go m.stageRunner.Run(context.Background())

	// Start reading events
	go func() {
		for evt := range m.stageRunner.Events {
			m.handleTimerEvent(evt)
		}
	}()

	m.statusMsg = ""
	return nil
}

func (m *model) buildModuleConfig(profile *config.Profile) *config.Modules {
	if profile.Modules == nil {
		return config.DefaultModules()
	}
	modCfg := *profile.Modules // shallow copy
	toggles := m.moduleToggles[m.profiles[m.selProfile]]
	setEnabled(&modCfg, toggles)
	return &modCfg
}

func setEnabled(mod *config.Modules, toggles map[string]bool) {
	if mod.Workspace != nil { mod.Workspace.Enabled = toggles["workspace"] }
	if mod.Media != nil { mod.Media.Enabled = toggles["media"] }
	if mod.Lock != nil { mod.Lock.Enabled = toggles["lock"] }
	if mod.Kill != nil { mod.Kill.Enabled = toggles["kill"] }
	if mod.Notify != nil { mod.Notify.Enabled = toggles["notify"] }
	if mod.Sound != nil { mod.Sound.Enabled = toggles["sound"] }
	if mod.Brightness != nil { mod.Brightness.Enabled = toggles["brightness"] }
	if mod.Mute != nil { mod.Mute.Enabled = toggles["mute"] }
	if mod.Custom != nil { mod.Custom.Enabled = toggles["custom"] }
	if mod.Script != nil { mod.Script.Enabled = toggles["script"] }
}

func (m *model) handleTimerEvent(evt engine.TimerEvent) {
	m.remaining = evt.Remaining
	m.total = evt.Total
	m.currentStage = evt.Stage
	m.timerStatus = evt.Status
}

func (m *model) stopTimer() {
	if m.stageRunner != nil {
		m.stageRunner.Stop()
	}
	m.timerStatus = engine.StatusIdle
	m.screen = screenMain
	m.recordRun("killed", "")
}

func (m *model) recordRun(status, errStr string) {
	if m.histLog == nil {
		return
	}
	m.histLog.Append(history.Record{
		Profile:  m.profiles[m.selProfile],
		Duration: m.timeInput,
		Status:   status,
		Error:    errStr,
	})
}

func (m *model) loadHistory() {
	path, err := history.DefaultPath()
	if err != nil {
		m.statusMsg = fmt.Sprintf("❌ History path: %v", err)
		return
	}
	records, err := history.ReadAll(path)
	if err != nil {
		m.statusMsg = fmt.Sprintf("❌ History read: %v", err)
		return
	}
	m.records = records
}

func (m *model) View() string {
	switch m.screen {
	case screenMain:
		return m.mainView()
	case screenRunning:
		return m.runningView()
	case screenHistory:
		return m.historyView()
	case screenStats:
		return m.statsView()
	case screenQR:
		return m.qrView()
	case screenHelp:
		return m.helpView()
	}
	return ""
}

func (m *model) mainView() string {
	var b strings.Builder

	b.WriteString(TitleStyle.Render("⏱ gosleep-timer"))
	b.WriteString("\n\n")

	// Profile selector
	b.WriteString(SelectWidget("Profile:", m.profiles, m.selProfile))
	b.WriteString("\n")

	// Time input
	timeFocused := m.focus == focusTimeInput
	b.WriteString(InputField("Duration:", m.timeInput, "e.g. 25m, 1h30m, 90s", timeFocused))
	b.WriteString("\n")

	// Module toggles
	b.WriteString(LabelStyle.Render("Modules:"))
	b.WriteString("\n")
	moduleNames := []string{"workspace", "media", "lock", "kill", "notify", "sound", "brightness", "mute", "custom", "script"}
	toggles := m.moduleToggles[m.profiles[m.selProfile]]
	for i, name := range moduleNames {
		enabled := toggles[name]
		line := fmt.Sprintf("  %d. ", i+1)
		if i == 9 {
			line = "  0. "
		}
		line += ModuleCheckbox(name, enabled, enabled)
		b.WriteString(line)
		b.WriteString("\n")
	}
	b.WriteString("\n")

	// Buttons
	startStyle := SuccessButtonStyle
	stopStyle := DangerButtonStyle
	if m.focus == focusStartButton {
		startStyle = startStyle.Bold(true).Background(lipgloss.Color("#00cc55"))
	}
	if m.focus == focusStopButton {
		stopStyle = stopStyle.Bold(true).Background(lipgloss.Color("#dd4444"))
	}
	b.WriteString(lipgloss.JoinHorizontal(lipgloss.Center,
		startStyle.Render("[ s ] Start"),
		"  ",
		stopStyle.Render("[ S ] Stop"),
	))
	b.WriteString("\n\n")

	// Status
	if m.statusMsg != "" {
		isErr := strings.HasPrefix(m.statusMsg, "❌")
		b.WriteString(StatusLine(m.statusMsg, isErr))
		b.WriteString("\n")
	}

	// Help bar
	b.WriteString("\n")
	b.WriteString(HelpBar("s:start", "S:stop", "h:history", "?:help", "q:quit"))

	return BorderStyle.Render(b.String())
}

func (m *model) runningView() string {
	var b strings.Builder

	b.WriteString(TitleStyle.Render("⏱ Timer Running"))
	b.WriteString("\n\n")

	// Big timer display
	b.WriteString(TimerBlock(m.remaining))
	b.WriteString("\n\n")

	// Progress bar
	totalSec := m.total.Seconds()
	remainingSec := m.remaining.Seconds()
	var pct float64
	if totalSec > 0 {
		pct = 1.0 - (remainingSec / totalSec)
	}
	b.WriteString(ProgressBar(40, pct, ""))
	b.WriteString("\n\n")

	// Stage info
	stageStr := string(m.currentStage)
	if stageStr == "" {
		stageStr = "timer"
	}
	b.WriteString(StatusStyle.Render(fmt.Sprintf("Stage: %s", stageStr)))
	b.WriteString("\n")

	// Profile info
	profileName := m.profiles[m.selProfile]
	b.WriteString(StatusStyle.Render(fmt.Sprintf("Profile: %s", profileName)))
	b.WriteString("\n\n")

	// Stop hint
	b.WriteString(HelpStyle.Render("Press S to stop · q to quit"))
	b.WriteString("\n")

	return BorderStyle.Render(b.String())
}

func (m *model) historyView() string {
	var b strings.Builder
	b.WriteString(TitleStyle.Render("📋 History"))
	b.WriteString("\n\n")

	if len(m.records) == 0 {
		b.WriteString(StatusStyle.Render("No history yet."))
	} else {
		start := 0
		if len(m.records) > 20 {
			start = len(m.records) - 20
		}
		for i := len(m.records) - 1; i >= start; i-- {
			r := m.records[i]
			line := fmt.Sprintf("%s | %-11s | %-6s | %s",
				r.Timestamp.Format("15:04 02-Jan"),
				r.Duration,
				r.Status,
				r.Profile,
			)
			style := ActiveModuleStyle
			if r.Status == "killed" {
				style = ErrorStyle
			}
			b.WriteString(style.Render(line))
			b.WriteString("\n")
		}
	}

	b.WriteString("\n")
	b.WriteString(HelpBar("s:stats", "h/esc:back", "q:quit"))
	return BorderStyle.Render(b.String())
}

func (m *model) statsView() string {
	var b strings.Builder
	b.WriteString(TitleStyle.Render("📊 Statistics"))
	b.WriteString("\n\n")

	stats := history.Calculate(m.records)
	b.WriteString(fmt.Sprintf("Total runs:     %d\n", stats.TotalRuns))
	b.WriteString(fmt.Sprintf("Completed:      %d\n", stats.CompletedRuns))
	b.WriteString(fmt.Sprintf("Killed:         %d\n", stats.KilledRuns))
	b.WriteString(fmt.Sprintf("Errors:         %d\n", stats.ErrorRuns))
	b.WriteString(fmt.Sprintf("Total time:     %s\n", stats.TotalDuration.Round(time.Second)))
	if stats.TotalRuns > 0 {
		b.WriteString(fmt.Sprintf("Avg duration:   %s\n", stats.AvgDuration.Round(time.Second)))
	}
	b.WriteString(fmt.Sprintf("Most used:      %s\n", stats.MostUsedProfile))
	if stats.LastRun != nil {
		b.WriteString(fmt.Sprintf("Last run:       %s (%s — %s)\n",
			stats.LastRun.Timestamp.Format("02 Jan 15:04"),
			stats.LastRun.Duration,
			stats.LastRun.Status,
		))
	}
	b.WriteString("\n")
	b.WriteString(HelpBar("h:back", "q:quit"))
	return BorderStyle.Render(b.String())
}

func (m *model) qrView() string {
	var b strings.Builder
	b.WriteString(TitleStyle.Render("📱 QR Code"))
	b.WriteString("\n\n")

	// Generate config YAML
	data, err := config.SaveToString(m.cfg)
	if err != nil {
		b.WriteString(ErrorStyle.Render(fmt.Sprintf("Config error: %v", err)))
	} else {
		qr, err := GenerateQR(data)
		if err != nil {
			b.WriteString(ErrorStyle.Render(fmt.Sprintf("QR error: %v", err)))
		} else {
			b.WriteString(qr)
		}
	}

	b.WriteString("\n")
	b.WriteString(HelpBar("q/esc:back"))
	return BorderStyle.Render(b.String())
}

func (m *model) helpView() string {
	var b strings.Builder
	b.WriteString(TitleStyle.Render("⌨ Help"))
	b.WriteString("\n")
	b.WriteString(HelpView())
	b.WriteString("\n")
	b.WriteString(HelpBar("q/esc:back"))
	return BorderStyle.Render(b.String())
}
