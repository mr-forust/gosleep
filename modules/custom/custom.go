package custom

import (
	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type CustomModule struct{}

func (m *CustomModule) Name() string {
	return "custom"
}

func (m *CustomModule) Enabled(cfg *config.Modules) bool {
	return cfg.Custom != nil && cfg.Custom.Enabled
}

func (m *CustomModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	switch stage {
	case modules.StagePre:
		return ctx.Config.Custom.Pre
	case modules.StagePost:
		return ctx.Config.Custom.Post
	}
	return nil
}

func (m *CustomModule) Validate() error {
	return nil
}

func init() {
	modules.Register(&CustomModule{})
}
