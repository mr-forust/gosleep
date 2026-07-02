package mute

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type MuteModule struct {
	tool string
}

func (m *MuteModule) Name() string {
	return "mute"
}

func (m *MuteModule) Enabled(cfg *config.Modules) bool {
	return cfg.Mute != nil && cfg.Mute.Enabled
}

func (m *MuteModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	switch stage {
	case modules.StagePre:
		if m.tool == "wpctl" {
			return []string{"wpctl set-mute @DEFAULT_AUDIO_SINK@ 1"}
		}
		return []string{"pactl set-sink-mute @DEFAULT_SINK@ 1"}
	case modules.StagePost:
		if m.tool == "wpctl" {
			return []string{"wpctl set-mute @DEFAULT_AUDIO_SINK@ 0"}
		}
		return []string{"pactl set-sink-mute @DEFAULT_SINK@ 0"}
	}
	return nil
}

func (m *MuteModule) Validate() error {
	if _, err := exec.LookPath("pactl"); err == nil {
		m.tool = "pactl"
		return nil
	}
	if _, err := exec.LookPath("wpctl"); err == nil {
		m.tool = "wpctl"
		return nil
	}
	return fmt.Errorf("mute: neither pactl nor wpctl found in PATH")
}

func init() {
	modules.Register(&MuteModule{})
}
