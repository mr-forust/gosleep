package profiles

import (
	"fmt"
	"sort"

	"github.com/forust/gosleep-timer/config"
)

type Manager struct {
	cfg *config.Config
}

func NewManager(cfg *config.Config) *Manager {
	return &Manager{cfg: cfg}
}

func (m *Manager) List() []string {
	// Return sorted list of profile names
	names := make([]string, 0, len(m.cfg.Profiles))
	for name := range m.cfg.Profiles {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

func (m *Manager) Get(name string) (*config.Profile, error) {
	p, ok := m.cfg.Profiles[name]
	if !ok {
		return nil, fmt.Errorf("profile %q not found", name)
	}
	return &p, nil
}

func (m *Manager) Info(name string) (*Info, error) {
	p, err := m.Get(name)
	if err != nil {
		return nil, err
	}
	info := &Info{Name: name}
	if p.Modules != nil {
		m := p.Modules
		if m.Workspace != nil && m.Workspace.Enabled {
			info.Modules = append(info.Modules, "workspace")
		}
		if m.Media != nil && m.Media.Enabled {
			info.Modules = append(info.Modules, "media")
		}
		if m.Lock != nil && m.Lock.Enabled {
			info.Modules = append(info.Modules, "lock")
		}
		if m.Kill != nil && m.Kill.Enabled {
			info.Modules = append(info.Modules, "kill")
		}
		if m.Notify != nil && m.Notify.Enabled {
			info.Modules = append(info.Modules, "notify")
		}
		if m.Sound != nil && m.Sound.Enabled {
			info.Modules = append(info.Modules, "sound")
		}
		if m.Brightness != nil && m.Brightness.Enabled {
			info.Modules = append(info.Modules, "brightness")
		}
		if m.Mute != nil && m.Mute.Enabled {
			info.Modules = append(info.Modules, "mute")
		}
		if m.Custom != nil && m.Custom.Enabled {
			info.Modules = append(info.Modules, "custom")
		}
		if m.Script != nil && m.Script.Enabled {
			info.Modules = append(info.Modules, "script")
		}
	}
	return info, nil
}
