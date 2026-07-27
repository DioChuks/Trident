package trident

import (
	"encoding/json"
	"fmt"
)

// TridentApiError is the typed error returned for non-2xx API responses.
type TridentApiError struct {
	Status  int
	Code    string
	Message string
	Field   string
}

func (e *TridentApiError) Error() string {
	if e.Field != "" {
		return fmt.Sprintf("trident API error %d (%s): %s (field: %s)", e.Status, e.Code, e.Message, e.Field)
	}
	return fmt.Sprintf("trident API error %d (%s): %s", e.Status, e.Code, e.Message)
}

func parseApiError(status int, body string) *TridentApiError {
	var env struct {
		Error struct {
			Code    string `json:"code"`
			Message string `json:"message"`
			Field   string `json:"field,omitempty"`
		} `json:"error"`
	}
	if err := json.Unmarshal([]byte(body), &env); err == nil && env.Error.Code != "" {
		return &TridentApiError{Status: status, Code: env.Error.Code, Message: env.Error.Message, Field: env.Error.Field}
	}
	msg := body
	if msg == "" {
		msg = fmt.Sprintf("HTTP %d", status)
	}
	return &TridentApiError{Status: status, Code: "INTERNAL", Message: msg}
}
