package httputil

import (
	"context"
	"strings"
)

// requestIDContextKey is the context key under which the per-request id is
// stored. It lives in httputil (a leaf package) so both the middleware layer
// and the error writers can read it without an import cycle.
type requestIDContextKey struct{}

// maxRequestIDLen bounds an accepted client-supplied X-Request-Id so a caller
// cannot smuggle an unbounded value into logs and downstream metadata.
const maxRequestIDLen = 128

// ContextWithRequestID returns a copy of ctx carrying the request id.
func ContextWithRequestID(ctx context.Context, id string) context.Context {
	return context.WithValue(ctx, requestIDContextKey{}, id)
}

// RequestIDFromContext returns the request id attached by the RequestID
// middleware, or "" if none is present.
func RequestIDFromContext(ctx context.Context) string {
	id, _ := ctx.Value(requestIDContextKey{}).(string)
	return id
}

// ValidRequestID reports whether a client-supplied X-Request-Id is safe to
// echo and propagate: non-empty, within the length bound, and containing only
// visible ASCII (no control characters or whitespace that could corrupt logs
// or gRPC metadata).
func ValidRequestID(id string) bool {
	if id == "" || len(id) > maxRequestIDLen {
		return false
	}
	for _, r := range id {
		if r < 0x21 || r > 0x7e {
			return false
		}
	}
	return !strings.ContainsAny(id, " \t")
}
