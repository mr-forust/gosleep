package modules

import (
	"context"

	"github.com/forust/gosleep-timer/config"
)

type Stage string

const (
	StagePre  Stage = "pre"
	StagePost Stage = "post"
)

type ModuleContext struct {
	Context  context.Context
	Profile  string
	Duration string
	Config   *config.Modules
}

type ModuleResult struct {
	Module  string
	Cmd     string
	Output  string
	Error   error
	Stage   Stage
}

type Module interface {
	Name() string
	// Enabled returns true if this module should run for the given config
	Enabled(cfg *config.Modules) bool
	// BuildCmds returns shell commands for the given stage (pre/post)
	BuildCmds(ctx *ModuleContext, stage Stage) []string
	// Validate checks if required binaries exist
	Validate() error
}
