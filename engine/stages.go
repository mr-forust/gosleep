package engine

import (
	"context"
	"time"
)

type StageRunner struct {
	Timer       *Timer
	PreCmds     []string
	PostCmds    []string
	Events      chan TimerEvent
	ExecTimeout time.Duration
}

func NewStageRunner(timer *Timer, pre, post []string) *StageRunner {
	return &StageRunner{
		Timer:       timer,
		PreCmds:     pre,
		PostCmds:    post,
		Events:      make(chan TimerEvent, 100),
		ExecTimeout: 30 * time.Second,
	}
}

func (sr *StageRunner) Run(ctx context.Context) {
	defer close(sr.Events)

	// PreCmds
	for _, cmd := range sr.PreCmds {
		result := Execute(ctx, cmd, sr.ExecTimeout)
		if result.Err != nil {
			sr.Events <- TimerEvent{Stage: StagePre, Status: StatusError, Done: true}
			return
		}
		sr.Events <- TimerEvent{Stage: StagePre, Status: StatusRunning}
	}

	// Timer
	timerEvents := make(chan TimerEvent, 100)
	go sr.Timer.Start(ctx, timerEvents)

	timerDone := false
timerLoop:
	for {
		select {
		case ev := <-timerEvents:
			sr.Events <- ev
			if ev.Done {
				timerDone = true
				break timerLoop
			}
		case <-ctx.Done():
			break timerLoop
		}
	}

	// PostCmds (only on normal timer completion)
	if timerDone && sr.Timer.Status == StatusDone {
		for _, cmd := range sr.PostCmds {
			result := Execute(ctx, cmd, sr.ExecTimeout)
			if result.Err != nil {
				sr.Events <- TimerEvent{Stage: StagePost, Status: StatusError, Done: true}
				return
			}
			sr.Events <- TimerEvent{Stage: StagePost, Status: StatusRunning}
		}
		sr.Events <- TimerEvent{Status: StatusDone, Done: true}
	}
}

func (sr *StageRunner) Stop() {
	sr.Timer.Stop()
}
