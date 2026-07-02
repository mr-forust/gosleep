package kill

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type KillModule struct{}

func (m *KillModule) Name() string {
	return "kill"
}

func (m *KillModule) Enabled(cfg *config.Modules) bool {
	return cfg.Kill != nil && cfg.Kill.Enabled && len(cfg.Kill.Processes) > 0
}

func (m *KillModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePre {
		return nil
	}

	var cmds []string
	for _, proc := range ctx.Config.Kill.Processes {
		if proc == "" {
			continue
		}
		cmds = append(cmds, fmt.Sprintf("pkill %s", proc))
	}
	return cmds
}

func (m *KillModule) Validate() error {
	_, err := exec.LookPath("pkill")
	return err
}

func init() {
	modules.Register(&KillModule{})
}
