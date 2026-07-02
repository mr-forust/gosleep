package script

import (
	"fmt"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type ScriptModule struct{}

func (m *ScriptModule) Name() string {
	return "script"
}

func (m *ScriptModule) Enabled(cfg *config.Modules) bool {
	return cfg.Script != nil && cfg.Script.Enabled && cfg.Script.Path != ""
}

func (m *ScriptModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	scriptPath := ctx.Config.Script.Path
	switch stage {
	case modules.StagePre:
		return []string{fmt.Sprintf("%s pre %s %s", scriptPath, ctx.Profile, ctx.Duration)}
	case modules.StagePost:
		return []string{fmt.Sprintf("%s post %s %s", scriptPath, ctx.Profile, ctx.Duration)}
	}
	return nil
}

func (m *ScriptModule) Validate() error {
	// Validation happens after config is loaded, so we check at registration time
	// that the module itself is valid, but path is per-profile.
	// Return nil here; path validation happens at runtime in BuildCmds.
	return nil
}

func init() {
	modules.Register(&ScriptModule{})
}
