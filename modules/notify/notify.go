package notify

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type NotifyModule struct{}

func (m *NotifyModule) Name() string {
	return "notify"
}

func (m *NotifyModule) Enabled(cfg *config.Modules) bool {
	return cfg.Notify != nil && cfg.Notify.Enabled
}

func (m *NotifyModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePost {
		return nil
	}

	title := "gosleep-timer"
	body := fmt.Sprintf("Timer %s finished", ctx.Duration)
	cmds := []string{fmt.Sprintf("notify-send %q %q", title, body)}

	if ctx.Config.Notify.Sound != "" {
		if _, err := exec.LookPath("paplay"); err == nil {
			cmds = append(cmds, fmt.Sprintf("paplay %s", ctx.Config.Notify.Sound))
		} else if _, err := exec.LookPath("aplay"); err == nil {
			cmds = append(cmds, fmt.Sprintf("aplay %s", ctx.Config.Notify.Sound))
		}
	}

	return cmds
}

func (m *NotifyModule) Validate() error {
	_, err := exec.LookPath("notify-send")
	return err
}

func init() {
	modules.Register(&NotifyModule{})
}
