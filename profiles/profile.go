package profiles

type Info struct {
	Name        string
	Description string
	Modules     []string // list of enabled module names
	Duration    string   // optional default duration
}
