package history

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"time"
)

type Record struct {
	Timestamp time.Time `json:"ts"`
	Profile   string    `json:"profile"`
	Duration  string    `json:"duration"`
	Status    string    `json:"status"` // completed, killed, error
	Error     string    `json:"error,omitempty"`
}

type Log struct {
	path string
	file *os.File
}

func DefaultPath() (string, error) {
	// $XDG_DATA_HOME/gosleep-timer/history.jsonl
	// fallback ~/.local/share/gosleep-timer/history.jsonl
	dataHome := os.Getenv("XDG_DATA_HOME")
	if dataHome == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", err
		}
		dataHome = filepath.Join(home, ".local", "share")
	}
	dir := filepath.Join(dataHome, "gosleep-timer")
	if err := os.MkdirAll(dir, 0755); err != nil {
		return "", err
	}
	return filepath.Join(dir, "history.jsonl"), nil
}

func Open(path string) (*Log, error) {
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return nil, err
	}
	return &Log{path: path, file: f}, nil
}

func (l *Log) Append(r Record) error {
	r.Timestamp = time.Now().UTC()
	data, err := json.Marshal(r)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintln(l.file, string(data))
	return err
}

func (l *Log) Close() error {
	return l.file.Close()
}

func ReadAll(path string) ([]Record, error) {
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return []Record{}, nil
		}
		return nil, err
	}
	defer f.Close()

	var records []Record
	scanner := bufio.NewScanner(f)
	for scanner.Scan() {
		var r Record
		if err := json.Unmarshal(scanner.Bytes(), &r); err != nil {
			continue // skip corrupt lines
		}
		records = append(records, r)
	}
	return records, scanner.Err()
}
