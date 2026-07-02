package config

func DefaultConfig() *Config {
	return &Config{
		Profiles: map[string]Profile{
			"default": {
				Modules: DefaultModules(),
			},
		},
	}
}

func DefaultModules() *Modules {
	return &Modules{
		Workspace: &WorkspaceConfig{
			Enabled:   true,
			Backend:   "",
			Workspace: 12,
		},
		Media: &MediaConfig{
			Enabled: true,
			Action:  "stop",
		},
		Lock: &LockConfig{
			Enabled: true,
			Command: "",
		},
		Kill: &KillConfig{
			Enabled:   false,
			Processes: nil,
		},
		Notify: &NotifyConfig{
			Enabled: false,
			Sound:   "",
		},
		Sound: &SoundConfig{
			Enabled: true,
			File:    "/usr/share/sounds/freedesktop/stereo/complete.oga",
		},
		Brightness: &BrightnessConfig{
			Enabled: false,
			Value:   0,
		},
		Mute: &MuteConfig{
			Enabled: false,
		},
		Custom: &CustomConfig{
			Enabled: false,
			Pre:     nil,
			Post:    nil,
		},
		Script: &ScriptConfig{
			Enabled: false,
			Path:    "",
		},
	}
}
