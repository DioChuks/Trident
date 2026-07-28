package trident

import "fmt"

// APIError represents a non-2xx HTTP response from the Trident API,
// optionally after exhausting the configured retry policy (Attempts > 1).
type APIError struct {
	StatusCode int
	Body       string
	Attempts   int
}

func (e *APIError) Error() string {
	if e.Attempts > 1 {
		return fmt.Sprintf("request failed with status %d after %d attempts: %s", e.StatusCode, e.Attempts, e.Body)
	}
	return fmt.Sprintf("request failed with status %d: %s", e.StatusCode, e.Body)
}

// RequestError represents a transport-level failure (e.g. a network error)
// that occurred after the configured retry policy was exhausted.
type RequestError struct {
	Attempts int
	Err      error
}

func (e *RequestError) Error() string {
	return fmt.Sprintf("request failed after %d attempt(s): %v", e.Attempts, e.Err)
}

func (e *RequestError) Unwrap() error {
	return e.Err
}
