package lock

import (
	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type LockModule struct{}

func (m *LockModule) Name() string {
	return "lock"
}

func (m *LockModule) Enabled(cfg *config.Modules) bool {
	return cfg.Lock != nil && cfg.Lock.Enabled
}

func (m *LockModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePost {
		return nil
	}
	cmd := ctx.Config.Lock.Command
	if cmd == "" {
		cmd = "loginctl lock-session"
	}
	return []string{cmd}
}

func (m *LockModule) Validate() error {
	// No strict validation — custom commands may be set
	return nil
}

func init() {
	modules.Register(&LockModule{})
}
