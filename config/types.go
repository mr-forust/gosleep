package config

type Config struct {
	Profiles map[string]Profile `yaml:"profiles"`
}

type Profile struct {
	Extends string   `yaml:"extends,omitempty"`
	Modules *Modules `yaml:"modules,omitempty"`
}

type Modules struct {
	Workspace  *WorkspaceConfig  `yaml:"workspace,omitempty"`
	Media      *MediaConfig      `yaml:"media,omitempty"`
	Lock       *LockConfig       `yaml:"lock,omitempty"`
	Kill       *KillConfig       `yaml:"kill,omitempty"`
	Notify     *NotifyConfig     `yaml:"notify,omitempty"`
	Sound      *SoundConfig      `yaml:"sound,omitempty"`
	Brightness *BrightnessConfig `yaml:"brightness,omitempty"`
	Mute       *MuteConfig       `yaml:"mute,omitempty"`
	Custom     *CustomConfig     `yaml:"custom,omitempty"`
	Script     *ScriptConfig     `yaml:"script,omitempty"`
}

type WorkspaceConfig struct {
	Enabled   bool   `yaml:"enabled" default:"true"`
	Backend   string `yaml:"backend"` // auto, niri, hyprland, kde
	Workspace int    `yaml:"workspace"`
}

type MediaConfig struct {
	Enabled bool   `yaml:"enabled" default:"true"`
	Action  string `yaml:"action"` // stop, pause, play-pause, next, previous, none
}

type LockConfig struct {
	Enabled bool   `yaml:"enabled" default:"true"`
	Command string `yaml:"command"` // custom lock cmd, empty = auto
}

type KillConfig struct {
	Enabled   bool     `yaml:"enabled" default:"false"`
	Processes []string `yaml:"processes"`
}

type NotifyConfig struct {
	Enabled bool   `yaml:"enabled" default:"true"`
	Sound   string `yaml:"sound"` // path or empty
}

type SoundConfig struct {
	Enabled bool   `yaml:"enabled" default:"true"`
	File    string `yaml:"file"`
}

type BrightnessConfig struct {
	Enabled bool `yaml:"enabled" default:"false"`
	Value   int  `yaml:"value"` // 0-100
}

type MuteConfig struct {
	Enabled bool `yaml:"enabled" default:"false"`
}

type CustomConfig struct {
	Enabled bool     `yaml:"enabled" default:"false"`
	Pre     []string `yaml:"pre"`
	Post    []string `yaml:"post"`
}

type ScriptConfig struct {
	Enabled bool   `yaml:"enabled" default:"false"`
	Path    string `yaml:"path"`
}
