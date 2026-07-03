package config

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

func DefaultConfigPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("home dir: %w", err)
	}
	return filepath.Join(home, ".config", "gosleep-timer", "config.yaml"), nil
}

func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read config: %w", err)
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse config: %w", err)
	}

	if cfg.Profiles == nil {
		cfg.Profiles = make(map[string]Profile)
	}

	resolved := make(map[string]Profile, len(cfg.Profiles))
	for name := range cfg.Profiles {
		seen := make(map[string]bool)
		r := resolveExtends(cfg.Profiles, name, seen)
		if r == nil {
			return nil, fmt.Errorf("circular extends detected for profile %q", name)
		}
		resolved[name] = *r
	}
	cfg.Profiles = resolved

	return &cfg, nil
}

func Save(path string, cfg *Config) error {
	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("mkdir config dir: %w", err)
	}

	data, err := yaml.Marshal(cfg)
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}

	if err := os.WriteFile(path, data, 0644); err != nil {
		return fmt.Errorf("write config: %w", err)
	}
	return nil
}

func LoadOrCreate(path string) (*Config, error) {
	cfg, err := Load(path)
	if err == nil {
		return cfg, nil
	}

	if !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}

	cfg = DefaultConfig()
	if err := Save(path, cfg); err != nil {
		return nil, fmt.Errorf("create default config: %w", err)
	}
	return cfg, nil
}

func SaveToString(cfg *Config) (string, error) {
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return "", fmt.Errorf("marshal config: %w", err)
	}
	return string(data), nil
}

func DetectDesktop() string {
	if de := os.Getenv("XDG_CURRENT_DESKTOP"); de != "" {
		lower := strings.ToLower(de)
		if strings.Contains(lower, "hyprland") {
			return "hyprland"
		}
		if strings.Contains(lower, "kde") || strings.Contains(lower, "plasma") {
			return "kde"
		}
		if strings.Contains(lower, "niri") {
			return "niri"
		}
	}

	if err := exec.Command("hyprctl", "instances").Run(); err == nil {
		return "hyprland"
	}

	if err := exec.Command("niri", "msg", "-h").Run(); err == nil {
		return "niri"
	}

	return "generic"
}

func resolveExtends(profiles map[string]Profile, name string, seen map[string]bool) *Profile {
	if seen[name] {
		return nil
	}
	seen[name] = true

	p, ok := profiles[name]
	if !ok {
		return nil
	}

	base := &Profile{}
	if p.Extends != "" {
		base = resolveExtends(profiles, p.Extends, seen)
		if base == nil {
			return nil
		}
	}

	merged := &Profile{
		Extends: p.Extends,
		Modules: copyModules(base.Modules),
	}

	if p.Modules != nil {
		if merged.Modules == nil {
			merged.Modules = &Modules{}
		}
		overlayModules(merged.Modules, p.Modules)
	}

	return merged
}

func copyModules(m *Modules) *Modules {
	if m == nil {
		return nil
	}
	c := &Modules{}
	if m.Workspace != nil {
		v := *m.Workspace
		c.Workspace = &v
	}
	if m.Media != nil {
		v := *m.Media
		c.Media = &v
	}
	if m.Lock != nil {
		v := *m.Lock
		c.Lock = &v
	}
	if m.Kill != nil {
		v := *m.Kill
		v.Processes = append([]string(nil), m.Kill.Processes...)
		c.Kill = &v
	}
	if m.Notify != nil {
		v := *m.Notify
		c.Notify = &v
	}
	if m.Sound != nil {
		v := *m.Sound
		c.Sound = &v
	}
	if m.Brightness != nil {
		v := *m.Brightness
		c.Brightness = &v
	}
	if m.Mute != nil {
		v := *m.Mute
		c.Mute = &v
	}
	if m.Custom != nil {
		v := *m.Custom
		v.Pre = append([]string(nil), m.Custom.Pre...)
		v.Post = append([]string(nil), m.Custom.Post...)
		c.Custom = &v
	}
	if m.Script != nil {
		v := *m.Script
		c.Script = &v
	}
	return c
}

func overlayModules(base, overlay *Modules) {
	if overlay.Workspace != nil {
		if base.Workspace == nil {
			base.Workspace = &WorkspaceConfig{}
		}
		base.Workspace.Enabled = overlay.Workspace.Enabled
		if overlay.Workspace.Backend != "" {
			base.Workspace.Backend = overlay.Workspace.Backend
		}
		if overlay.Workspace.Workspace != 0 {
			base.Workspace.Workspace = overlay.Workspace.Workspace
		}
	}
	if overlay.Media != nil {
		if base.Media == nil {
			base.Media = &MediaConfig{}
		}
		base.Media.Enabled = overlay.Media.Enabled
		if overlay.Media.Action != "" {
			base.Media.Action = overlay.Media.Action
		}
	}
	if overlay.Lock != nil {
		if base.Lock == nil {
			base.Lock = &LockConfig{}
		}
		base.Lock.Enabled = overlay.Lock.Enabled
		if overlay.Lock.Command != "" {
			base.Lock.Command = overlay.Lock.Command
		}
	}
	if overlay.Kill != nil {
		base.Kill = &KillConfig{
			Enabled:   overlay.Kill.Enabled,
			Processes: append([]string(nil), overlay.Kill.Processes...),
		}
	}
	if overlay.Notify != nil {
		if base.Notify == nil {
			base.Notify = &NotifyConfig{}
		}
		base.Notify.Enabled = overlay.Notify.Enabled
		if overlay.Notify.Sound != "" {
			base.Notify.Sound = overlay.Notify.Sound
		}
	}
	if overlay.Sound != nil {
		if base.Sound == nil {
			base.Sound = &SoundConfig{}
		}
		base.Sound.Enabled = overlay.Sound.Enabled
		if overlay.Sound.File != "" {
			base.Sound.File = overlay.Sound.File
		}
	}
	if overlay.Brightness != nil {
		base.Brightness = &BrightnessConfig{
			Enabled: overlay.Brightness.Enabled,
			Value:   overlay.Brightness.Value,
		}
	}
	if overlay.Mute != nil {
		base.Mute = &MuteConfig{
			Enabled: overlay.Mute.Enabled,
		}
	}
	if overlay.Custom != nil {
		base.Custom = &CustomConfig{
			Enabled: overlay.Custom.Enabled,
			Pre:     append([]string(nil), overlay.Custom.Pre...),
			Post:    append([]string(nil), overlay.Custom.Post...),
		}
	}
	if overlay.Script != nil {
		base.Script = &ScriptConfig{
			Enabled: overlay.Script.Enabled,
			Path:    overlay.Script.Path,
		}
	}
}
