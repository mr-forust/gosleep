package main

import (
	"context"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/spf13/cobra"
	"gopkg.in/yaml.v3"
	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/engine"
	"github.com/forust/gosleep-timer/history"
	"github.com/forust/gosleep-timer/modules"
	"github.com/forust/gosleep-timer/tui"

	// Register all modules
	_ "github.com/forust/gosleep-timer/modules/brightness"
	_ "github.com/forust/gosleep-timer/modules/custom"
	_ "github.com/forust/gosleep-timer/modules/kill"
	_ "github.com/forust/gosleep-timer/modules/lock"
	_ "github.com/forust/gosleep-timer/modules/media"
	_ "github.com/forust/gosleep-timer/modules/mute"
	_ "github.com/forust/gosleep-timer/modules/notify"
	_ "github.com/forust/gosleep-timer/modules/script"
	_ "github.com/forust/gosleep-timer/modules/sound"
	_ "github.com/forust/gosleep-timer/modules/workspace"
)

var (
	cfgFile  string
	profile  string
	cliMode  bool
	quiet    bool
)

var rootCmd = &cobra.Command{
	Use:   "gosleep-timer",
	Short: "TUI sleep timer with modular WM/DE support",
	Long: `gosleep-timer — configurable timer with modules for workspace switching,
media control, screen locking, process killing, notifications, and more.

Supports niri, Hyprland, KDE, and custom commands.`,
	RunE: func(cmd *cobra.Command, args []string) error {
		// Init modules registry
		modules.Init()

		// Load config
		cfgPath := cfgFile
		if cfgPath == "" {
			var err error
			cfgPath, err = config.DefaultConfigPath()
			if err != nil {
				return fmt.Errorf("config path: %w", err)
			}
		}

		cfg, err := config.LoadOrCreate(cfgPath)
		if err != nil {
			return fmt.Errorf("config: %w", err)
		}

		// CLI mode (run without TUI)
		if cliMode {
			return runCLI(cfg, args)
		}

		// Validate modules
		if errs := modules.ValidateAll(); len(errs) > 0 {
			for _, e := range errs {
				fmt.Fprintf(os.Stderr, "⚠ module warning: %v\n", e)
			}
		}

		// Start TUI
		m := tui.NewModel(cfg)
		p := tea.NewProgram(m, tea.WithAltScreen())
		if _, err := p.Run(); err != nil {
			return fmt.Errorf("tui error: %w", err)
		}
		return nil
	},
}

var listProfilesCmd = &cobra.Command{
	Use:   "list-profiles",
	Short: "List available profiles",
	RunE: func(cmd *cobra.Command, args []string) error {
		cfg, err := loadConfig()
		if err != nil {
			return err
		}

		for name := range cfg.Profiles {
			p := cfg.Profiles[name]
			extends := ""
			if p.Extends != "" {
				extends = fmt.Sprintf(" (extends: %s)", p.Extends)
			}
			fmt.Printf("  %s%s\n", name, extends)
		}
		return nil
	},
}

var exportCmd = &cobra.Command{
	Use:   "export",
	Short: "Export config to stdout",
	RunE: func(cmd *cobra.Command, args []string) error {
		cfg, err := loadConfig()
		if err != nil {
			return err
		}
		data, err := config.SaveToString(cfg)
		if err != nil {
			return err
		}
		fmt.Print(data)
		return nil
	},
}

var importCmd = &cobra.Command{
	Use:   "import < file",
	Short: "Import config from stdin",
	RunE: func(cmd *cobra.Command, args []string) error {
		cfgPath, err := getConfigPath()
		if err != nil {
			return err
		}

		data, err := os.ReadFile("/dev/stdin")
		if err != nil {
			return fmt.Errorf("read stdin: %w", err)
		}

		var cfg config.Config
		if err := yaml.Unmarshal(data, &cfg); err != nil {
			return fmt.Errorf("parse config: %w", err)
		}

		if err := config.Save(cfgPath, &cfg); err != nil {
			return fmt.Errorf("save config: %w", err)
		}

		fmt.Printf("Config imported to %s\n", cfgPath)
		return nil
	},
}

var historyCmd = &cobra.Command{
	Use:   "history",
	Short: "Show timer history",
	RunE: func(cmd *cobra.Command, args []string) error {
		path, err := history.DefaultPath()
		if err != nil {
			return err
		}
		records, err := history.ReadAll(path)
		if err != nil {
			return err
		}
		if len(records) == 0 {
			fmt.Println("No history.")
			return nil
		}
		for _, r := range records {
			errStr := ""
			if r.Error != "" {
				errStr = fmt.Sprintf(" error=%s", r.Error)
			}
			fmt.Printf("%s | %-8s | %-10s | %s%s\n",
				r.Timestamp.Format("2006-01-02 15:04:05"),
				r.Duration,
				r.Status,
				r.Profile,
				errStr,
			)
		}
		return nil
	},
}

var statsCmd = &cobra.Command{
	Use:   "stats",
	Short: "Show timer statistics",
	RunE: func(cmd *cobra.Command, args []string) error {
		path, err := history.DefaultPath()
		if err != nil {
			return err
		}
		records, err := history.ReadAll(path)
		if err != nil {
			return err
		}
		s := history.Calculate(records)
		fmt.Printf("Total runs:   %d\n", s.TotalRuns)
		fmt.Printf("Completed:    %d\n", s.CompletedRuns)
		fmt.Printf("Killed:       %d\n", s.KilledRuns)
		fmt.Printf("Errors:       %d\n", s.ErrorRuns)
		return nil
	},
}

func runCLI(cfg *config.Config, args []string) error {
	duration := "25m"
	if len(args) > 0 {
		duration = args[0]
	}

	d, err := engine.ParseDuration(duration)
	if err != nil {
		return fmt.Errorf("invalid duration %q: %w", duration, err)
	}

	profileName := profile
	if profileName == "" {
		profileName = "default"
	}

	p, ok := cfg.Profiles[profileName]
	if !ok {
		return fmt.Errorf("profile %q not found", profileName)
	}

	modCfg := p.Modules
	if modCfg == nil {
		modCfg = config.DefaultModules()
	}

	modCtx := &modules.ModuleContext{
		Context:  context.Background(),
		Profile:  profileName,
		Duration: duration,
		Config:   modCfg,
	}

	preCmds := modules.CollectPreCmds(modCtx)
	postCmds := modules.CollectPostCmds(modCtx)

	if !quiet {
		fmt.Printf("Timer: %s | Profile: %s\n", duration, profileName)
		fmt.Printf("Pre:  %s\n", strings.Join(preCmds, "; "))
	}

	t := engine.NewTimer(d)
	sr := engine.NewStageRunner(t, preCmds, postCmds)

	// Handle signals
	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go func() {
		<-sigCh
		sr.Stop()
		cancel()
		if !quiet {
			fmt.Println("\n⛔ Timer killed")
		}
	}()

	go sr.Run(ctx)

	for evt := range sr.Events {
		if evt.Done {
			break
		}
		if evt.Stage == engine.StageTimer && !quiet {
			rem := evt.Remaining.Round(time.Second)
			fmt.Printf("\r⏱ %s remaining...", rem)
		}
	}

	if !quiet {
		fmt.Println("\n✅ Timer completed!")
	}

	// Record history
	histPath, err := history.DefaultPath()
	if err == nil {
		if l, err := history.Open(histPath); err == nil {
			l.Append(history.Record{
				Profile:  profileName,
				Duration: duration,
				Status:   "completed",
			})
			l.Close()
		}
	}

	return nil
}

func loadConfig() (*config.Config, error) {
	path, err := getConfigPath()
	if err != nil {
		return nil, err
	}
	return config.LoadOrCreate(path)
}

func getConfigPath() (string, error) {
	if cfgFile != "" {
		return cfgFile, nil
	}
	return config.DefaultConfigPath()
}

func init() {
	rootCmd.PersistentFlags().StringVarP(&cfgFile, "config", "c", "", "config file path")
	rootCmd.PersistentFlags().BoolVarP(&quiet, "quiet", "q", false, "suppress output")

	runCmd := &cobra.Command{
		Use:   "run [duration]",
		Short: "Run timer in CLI mode (no TUI)",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			cliMode = true
			cfg, err := loadConfig()
			if err != nil {
				return err
			}
			return runCLI(cfg, args)
		},
	}
	runCmd.Flags().StringVarP(&profile, "profile", "p", "default", "profile name")
	rootCmd.AddCommand(runCmd)

	rootCmd.AddCommand(listProfilesCmd)
	rootCmd.AddCommand(exportCmd)
	rootCmd.AddCommand(importCmd)
	rootCmd.AddCommand(historyCmd)
	rootCmd.AddCommand(statsCmd)
}

func main() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
