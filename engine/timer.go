package engine

import (
	"context"
	"fmt"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type Status int

const (
	StatusIdle Status = iota
	StatusRunning
	StatusPaused
	StatusDone
	StatusKilled
	StatusError
)

type StageType string

const (
	StagePre   StageType = "pre"
	StageTimer StageType = "timer"
	StagePost  StageType = "post"
)

type TimerEvent struct {
	Remaining time.Duration
	Total     time.Duration
	Stage     StageType
	Status    Status
	Done      bool
}

type Timer struct {
	Duration time.Duration
	Status   Status
	StartedAt    time.Time
	elapsed  time.Duration
	cancel   context.CancelFunc
}

func ParseDuration(s string) (time.Duration, error) {
	s = strings.TrimSpace(s)
	if d, err := time.ParseDuration(s); err == nil {
		return d, nil
	}
	re := regexp.MustCompile(`(\d+)([hms])`)
	matches := re.FindAllStringSubmatch(s, -1)
	if len(matches) == 0 {
		return 0, fmt.Errorf("invalid duration: %s", s)
	}
	var d time.Duration
	for _, m := range matches {
		v, _ := strconv.Atoi(m[1])
		switch m[2] {
		case "h":
			d += time.Duration(v) * time.Hour
		case "m":
			d += time.Duration(v) * time.Minute
		case "s":
			d += time.Duration(v) * time.Second
		}
	}
	return d, nil
}

func NewTimer(d time.Duration) *Timer {
	return &Timer{Duration: d, Status: StatusIdle}
}

func (t *Timer) Start(ctx context.Context, events chan<- TimerEvent) {
	ctx, cancel := context.WithCancel(ctx)
	t.cancel = cancel
	t.StartedAt = time.Now()
	t.Status = StatusRunning

	go func() {
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()

		for {
			select {
			case <-ctx.Done():
				t.Status = StatusKilled
				events <- TimerEvent{Status: StatusKilled, Done: true}
				return
			case <-ticker.C:
				elapsed := time.Since(t.StartedAt)
				remaining := t.Duration - elapsed
				if remaining <= 0 {
					t.Status = StatusDone
					events <- TimerEvent{Remaining: 0, Total: t.Duration, Status: StatusDone, Done: true}
					return
				}
				events <- TimerEvent{
					Remaining: remaining,
					Total:     t.Duration,
					Stage:     StageTimer,
					Status:    StatusRunning,
				}
			}
		}
	}()
}

func (t *Timer) Stop() {
	if t.cancel != nil {
		t.cancel()
	}
	t.Status = StatusKilled
}
