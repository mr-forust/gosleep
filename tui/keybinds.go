package tui

type KeyAction int

const (
    KeyQuit KeyAction = iota
    KeyStart
    KeyStop
    KeyToggleModule
    KeyNextProfile
    KeyPrevProfile
    KeyFocusNext
    KeyFocusPrev
    KeyHistory
    KeyStats
    KeyHelp
)

type KeyBinding struct {
    Key    string
    Action KeyAction
    Label  string
}

var DefaultKeyBindings = []KeyBinding{
    {Key: "s", Action: KeyStart, Label: "start"},
    {Key: "S", Action: KeyStop, Label: "stop"},
    {Key: "tab", Action: KeyFocusNext, Label: "next"},
    {Key: "shift+tab", Action: KeyFocusPrev, Label: "prev"},
    {Key: "h", Action: KeyHistory, Label: "history"},
    {Key: "?", Action: KeyHelp, Label: "help"},
    {Key: "q", Action: KeyQuit, Label: "quit"},
    {Key: "esc", Action: KeyQuit, Label: "quit"},
}

func HelpView() string {
    s := "\n  Keys:\n"
    for _, kb := range DefaultKeyBindings {
        s += "    " + kb.Key + " — " + kb.Label + "\n"
    }
    return s
}
