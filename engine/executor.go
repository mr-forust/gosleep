package engine

import (
	"context"
	"os/exec"
	"time"
)

type ExecResult struct {
	Cmd     string
	Output  string
	Err     error
	Elapsed time.Duration
}

func Execute(ctx context.Context, cmd string, timeout time.Duration) ExecResult {
	start := time.Now()

	var cancel context.CancelFunc
	if timeout > 0 {
		ctx, cancel = context.WithTimeout(ctx, timeout)
		defer cancel()
	}

	out, err := exec.CommandContext(ctx, "bash", "-c", cmd).CombinedOutput()
	elapsed := time.Since(start)

	return ExecResult{
		Cmd:     cmd,
		Output:  string(out),
		Err:     err,
		Elapsed: elapsed,
	}
}

func ExecuteAll(ctx context.Context, cmds []string, timeout time.Duration) []ExecResult {
	results := make([]ExecResult, len(cmds))
	for i, cmd := range cmds {
		results[i] = Execute(ctx, cmd, timeout)
	}
	return results
}
