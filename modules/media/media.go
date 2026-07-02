package media

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type MediaModule struct{}

func (m *MediaModule) Name() string {
	return "media"
}

func (m *MediaModule) Enabled(cfg *config.Modules) bool {
	return cfg.Media != nil && cfg.Media.Enabled
}

func (m *MediaModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePre {
		return nil
	}
	if ctx.Config.Media.Action == "none" {
		return nil
	}
	return []string{fmt.Sprintf("playerctl %s", ctx.Config.Media.Action)}
}

func (m *MediaModule) Validate() error {
	if _, err := exec.LookPath("playerctl"); err != nil {
		return fmt.Errorf("media: playerctl not found")
	}
	return nil
}

func init() {
	modules.Register(&MediaModule{})
}
