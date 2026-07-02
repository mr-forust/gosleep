package modules

import "github.com/forust/gosleep-timer/config"

var registry []Module

func Register(m Module) {
	registry = append(registry, m)
}

func GetAll() []Module {
	return registry
}

func GetEnabled(cfg *config.Modules) []Module {
	var enabled []Module
	for _, m := range registry {
		if m.Enabled(cfg) {
			enabled = append(enabled, m)
		}
	}
	return enabled
}

func CollectPreCmds(ctx *ModuleContext) []string {
	var cmds []string
	for _, m := range GetEnabled(ctx.Config) {
		cmds = append(cmds, m.BuildCmds(ctx, StagePre)...)
	}
	return cmds
}

func CollectPostCmds(ctx *ModuleContext) []string {
	var cmds []string
	for _, m := range GetEnabled(ctx.Config) {
		cmds = append(cmds, m.BuildCmds(ctx, StagePost)...)
	}
	return cmds
}

func ValidateAll() []error {
	var errs []error
	for _, m := range registry {
		if err := m.Validate(); err != nil {
			errs = append(errs, err)
		}
	}
	return errs
}

func Init() {
	// registry starts empty; modules register themselves via init() or Register()
}
