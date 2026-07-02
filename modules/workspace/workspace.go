package workspace

import (
	"fmt"
	"os/exec"

	"github.com/forust/gosleep-timer/config"
	"github.com/forust/gosleep-timer/modules"
)

type WorkspaceModule struct{}

func (m *WorkspaceModule) Name() string {
	return "workspace"
}

func (m *WorkspaceModule) Enabled(cfg *config.Modules) bool {
	return cfg.Workspace != nil && cfg.Workspace.Enabled
}

func (m *WorkspaceModule) BuildCmds(ctx *modules.ModuleContext, stage modules.Stage) []string {
	if stage != modules.StagePre {
		return nil
	}

	backend := ctx.Config.Workspace.Backend
	if backend == "" || backend == "auto" {
		backend = config.DetectDesktop()
	}

	n := ctx.Config.Workspace.Workspace
	switch backend {
	case "niri":
		return []string{fmt.Sprintf("niri msg action focus-workspace %d", n)}
	case "hyprland":
		return []string{fmt.Sprintf("hyprctl dispatch workspace %d", n)}
	case "kde":
		return []string{fmt.Sprintf("qdbus org.kde.KWin /KWin setCurrentDesktop %d", n)}
	default:
		return nil
	}
}

func (m *WorkspaceModule) Validate() error {
	// Check common workspace binaries
	for _, bin := range []string{"niri", "hyprctl", "qdbus"} {
		if _, err := exec.LookPath(bin); err == nil {
			return nil
		}
	}
	return fmt.Errorf("workspace: no supported backend binary found (niri, hyprctl, qdbus)")
}

func init() {
	modules.Register(&WorkspaceModule{})
}
