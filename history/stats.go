package history

import "time"

type Stats struct {
	TotalRuns       int
	CompletedRuns   int
	KilledRuns      int
	ErrorRuns       int
	TotalDuration   time.Duration
	AvgDuration     time.Duration
	MostUsedProfile string
	LastRun         *Record
}

func Calculate(records []Record) Stats {
	var s Stats
	profileCount := make(map[string]int)
	s.TotalRuns = len(records)

	for _, r := range records {
		switch r.Status {
		case "completed":
			s.CompletedRuns++
		case "killed":
			s.KilledRuns++
		case "error":
			s.ErrorRuns++
		}

		profileCount[r.Profile]++

		d, err := time.ParseDuration(r.Duration)
		if err == nil {
			s.TotalDuration += d
		}

		s.LastRun = &r
	}

	if s.TotalRuns > 0 {
		s.AvgDuration = time.Duration(int64(s.TotalDuration) / int64(s.TotalRuns))
	}

	maxCount := 0
	for name, count := range profileCount {
		if count > maxCount {
			maxCount = count
			s.MostUsedProfile = name
		}
	}

	return s
}
