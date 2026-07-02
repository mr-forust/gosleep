package sound

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type SoundModule struct {
	player string
}

func (m *SoundModule) Name() string {
	return "sound"
}

func (m *SoundModule) Enabled(cfg *config.Modules) bool {
	return cfg.Sound != nil && cfg.Sound.Enabled && cfg.Sound.File != ""
}

func (m *SoundModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePost {
		return nil
	}

	file := ctx.Config.Sound.File
	if file == "" {
		return nil
	}

	player := m.player
	if player == "" {
		player = detectPlayer()
	}

	switch player {
	case "paplay":
		return []string{fmt.Sprintf("paplay %s", file)}
	case "aplay":
		return []string{fmt.Sprintf("aplay %s", file)}
	case "ffplay":
		return []string{fmt.Sprintf("ffplay -nodisp -autoexit %s", file)}
	default:
		return nil
	}
}

func (m *SoundModule) Validate() error {
	player := detectPlayer()
	if player == "" {
		return fmt.Errorf("no audio player found (paplay, aplay, or ffplay)")
	}
	m.player = player
	return nil
}

func detectPlayer() string {
	if _, err := exec.LookPath("paplay"); err == nil {
		return "paplay"
	}
	if _, err := exec.LookPath("aplay"); err == nil {
		return "aplay"
	}
	if _, err := exec.LookPath("ffplay"); err == nil {
		return "ffplay"
	}
	return ""
}

func init() {
	modules.Register(&SoundModule{})
}
