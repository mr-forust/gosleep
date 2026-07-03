package main

import (
	"bufio"
	"fmt"
	"os"
	"strconv"
	"strings"

	"github.com/spf13/cobra"
	"github.com/forust/gosleep-timer/config"
)

var initCmd = &cobra.Command{
	Use:   "init",
	Short: "Interactive config setup wizard",
	Long: `Creates a new config.yaml with interactive prompts.
Walks through all modules and profiles step by step.`,
	RunE: func(cmd *cobra.Command, args []string) error {
		cfgPath, err := getConfigPath()
		if err != nil {
			return err
		}

		// Check if config already exists
		if _, err := os.Stat(cfgPath); err == nil {
			fmt.Printf("Config already exists at %s\n", cfgPath)
			fmt.Print("Overwrite? [y/N]: ")
			reader := bufio.NewReader(os.Stdin)
			answer, _ := reader.ReadString('\n')
			answer = strings.TrimSpace(strings.ToLower(answer))
			if answer != "y" && answer != "yes" {
				fmt.Println("Aborted.")
				return nil
			}
		}

		cfg := runWizard()
		if cfg == nil {
			fmt.Println("\nSetup aborted.")
			return nil
		}

		if err := config.Save(cfgPath, cfg); err != nil {
			return fmt.Errorf("save config: %w", err)
		}

		fmt.Printf("\n✅ Config saved to %s\n", cfgPath)
		fmt.Println("Run 'gosleep-timer' to start the TUI.")
		return nil
	},
}

func init() {
	rootCmd.AddCommand(initCmd)
}

func runWizard() *config.Config {
	reader := bufio.NewReader(os.Stdin)
	cfg := config.DefaultConfig()
	p := &config.Profile{}

	fmt.Println(strings.Repeat("=", 50))
	fmt.Println("  ⏱  gosleep-timer Setup Wizard")
	fmt.Println(strings.Repeat("=", 50))

	// Profile name
	fmt.Print("\nProfile name [default]: ")
	name := readLine(reader, "default")

	// Duration
	fmt.Print("Default duration (e.g. 25m, 1h, 90s) [25m]: ")
	dur := readLine(reader, "25m")

	// Workspace module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Workspace — switch to a workspace on timer start")
	wsEnabled := askYesNo(reader, "Enable workspace switch?", true)
	ws := &config.WorkspaceConfig{Enabled: wsEnabled}
	if wsEnabled {
		fmt.Print("Workspace number [12]: ")
		wsStr := readLine(reader, "12")
		ws.Workspace, _ = strconv.Atoi(wsStr)
		if ws.Workspace == 0 {
			ws.Workspace = 12
		}

		fmt.Println("Backend (auto-detect / niri / hyprland / kde):")
		fmt.Print("  [auto]: ")
		ws.Backend = readLine(reader, "auto")
		if ws.Backend == "auto" {
			ws.Backend = ""
		}
	}

	// Media module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Media — control music player on timer start")
	mediaEnabled := askYesNo(reader, "Enable media control?", true)
	media := &config.MediaConfig{Enabled: mediaEnabled}
	if mediaEnabled {
		fmt.Println("Player action (stop / pause / play-pause / next / previous / none):")
		fmt.Print("  [stop]: ")
		action := readLine(reader, "stop")
		media.Action = action
	}

	// Lock module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Lock — lock screen when timer finishes")
	lockEnabled := askYesNo(reader, "Enable screen lock?", true)
	lock := &config.LockConfig{Enabled: lockEnabled}
	if lockEnabled {
		fmt.Print("Custom lock command (leave empty for auto): ")
		cmd := readLine(reader, "")
		lock.Command = cmd
	}

	// Kill module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Kill — terminate apps when timer starts")
	killEnabled := askYesNo(reader, "Enable process killer?", false)
	kill := &config.KillConfig{Enabled: killEnabled}
	if killEnabled {
		fmt.Print("Processes to kill (comma separated, e.g. firefox,telegram): ")
		procs := readLine(reader, "")
		if procs != "" {
			for _, p := range strings.Split(procs, ",") {
				p = strings.TrimSpace(p)
				if p != "" {
					kill.Processes = append(kill.Processes, p)
				}
			}
		}
	}

	// Notify module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Notify — desktop notification when timer finishes")
	notifyEnabled := askYesNo(reader, "Enable notifications?", true)
	notify := &config.NotifyConfig{Enabled: notifyEnabled}
	if notifyEnabled {
		fmt.Print("Notification sound path (empty = no sound): ")
		notify.Sound = readLine(reader, "")
	}

	// Sound module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Sound — play audio when timer finishes")
	soundEnabled := askYesNo(reader, "Enable sound?", true)
	sound := &config.SoundConfig{Enabled: soundEnabled}
	if soundEnabled {
		sound.File = "/usr/share/sounds/freedesktop/stereo/complete.oga"
		fmt.Printf("  Default: %s\n", sound.File)
		fmt.Print("Custom sound file (leave empty for default): ")
		custom := readLine(reader, "")
		if custom != "" {
			sound.File = custom
		}
	}

	// Brightness module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Brightness — dim screen during timer")
	brightnessEnabled := askYesNo(reader, "Enable brightness control?", false)
	brightness := &config.BrightnessConfig{Enabled: brightnessEnabled}
	if brightnessEnabled {
		fmt.Print("Brightness level 0-100 [0]: ")
		valStr := readLine(reader, "0")
		val, _ := strconv.Atoi(valStr)
		brightness.Value = val
	}

	// Mute module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Mute — mute audio during timer")
	muteEnabled := askYesNo(reader, "Enable mute?", false)
	mute := &config.MuteConfig{Enabled: muteEnabled}

	// Custom commands
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Custom — run your own commands")
	customEnabled := askYesNo(reader, "Enable custom commands?", false)
	custom := &config.CustomConfig{Enabled: customEnabled}
	if customEnabled {
		fmt.Println("Pre-commands (run BEFORE timer, one per line, empty line to end):")
		custom.Pre = readLines(reader, "pre")
		fmt.Println("Post-commands (run AFTER timer, one per line, empty line to end):")
		custom.Post = readLines(reader, "post")
	}

	// Script module
	fmt.Println(strings.Repeat("-", 50))
	fmt.Println("Script — run a script with timer context (args: pre/post, profile, duration)")
	scriptEnabled := askYesNo(reader, "Enable script module?", false)
	script := &config.ScriptConfig{Enabled: scriptEnabled}
	if scriptEnabled {
		fmt.Print("Script path: ")
		script.Path = readLine(reader, "")
	}

	// Build final config
	p.Modules = &config.Modules{
		Workspace:  ws,
		Media:      media,
		Lock:       lock,
		Kill:       kill,
		Notify:     notify,
		Sound:      sound,
		Brightness: brightness,
		Mute:       mute,
		Custom:     custom,
		Script:     script,
	}

	cfg.Profiles = map[string]config.Profile{
		name: *p,
	}

	fmt.Println(strings.Repeat("=", 50))
	fmt.Println("  ✅ Setup complete!")
	fmt.Printf("  Profile: %s\n", name)
	fmt.Printf("  Duration: %s\n", dur)
	fmt.Println(strings.Repeat("=", 50))

	return cfg
}

func readLine(reader *bufio.Reader, fallback string) string {
	line, _ := reader.ReadString('\n')
	line = strings.TrimSpace(line)
	if line == "" {
		return fallback
	}
	return line
}

func askYesNo(reader *bufio.Reader, prompt string, defaultYes bool) bool {
	suffix := "[Y/n]"
	if !defaultYes {
		suffix = "[y/N]"
	}
	fmt.Printf("%s %s: ", prompt, suffix)
	answer := strings.ToLower(strings.TrimSpace(readLine(reader, "")))
	if answer == "" {
		return defaultYes
	}
	return answer == "y" || answer == "yes"
}

func readLines(reader *bufio.Reader, label string) []string {
	var lines []string
	for {
		fmt.Printf("  %s> ", label)
		line, _ := reader.ReadString('\n')
		line = strings.TrimSpace(line)
		if line == "" {
			break
		}
		lines = append(lines, line)
	}
	return lines
}
