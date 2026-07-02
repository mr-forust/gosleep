package brightness

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type BrightnessModule struct {
	tool string
}

func (m *BrightnessModule) Name() string {
	return "brightness"
}

func (m *BrightnessModule) Enabled(cfg *config.Modules) bool {
	return cfg.Brightness != nil && cfg.Brightness.Enabled
}

func (m *BrightnessModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	val := ctx.Config.Brightness.Value
	if val < 0 {
		val = 0
	}
	if val > 100 {
		val = 100
	}

	switch stage {
	case modules.StagePre:
		if m.tool == "light" {
			return []string{fmt.Sprintf("light -S %d", val)}
		}
		return []string{fmt.Sprintf("brightnessctl set %d%%", val)}
	case modules.StagePost:
		if m.tool == "light" {
			return []string{"light -S 100"}
		}
		return []string{"brightnessctl set 100%"}
	}
	return nil
}

func (m *BrightnessModule) Validate() error {
	if _, err := exec.LookPath("brightnessctl"); err == nil {
		m.tool = "brightnessctl"
		return nil
	}
	if _, err := exec.LookPath("light"); err == nil {
		m.tool = "light"
		return nil
	}
	return fmt.Errorf("brightness: neither brightnessctl nor light found in PATH")
}

func init() {
	modules.Register(&BrightnessModule{})
}
