package middleware

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"time"

	"github.com/Depo-dev/trident/services/api/internal/httputil"
	"github.com/redis/go-redis/v9"
)

// DefaultIdempotencyTTL bounds how long a replayed response stays available
// (issue #225). 24h comfortably covers a client retrying after a network
// partition, a redeploy, or an overnight retry queue, without keeping every
// create call's response in Redis forever.
const DefaultIdempotencyTTL = 24 * time.Hour

// maxIdempotencyBodyBytes bounds the request body read into memory to
// compute a fingerprint. The routes this wraps (create api-key, create
// webhook) accept small JSON bodies — BodySizeLimit already caps every body
// well under this — so this is a defensive ceiling, not an expected limit.
const maxIdempotencyBodyBytes = 1 << 20 // 1 MiB

// maxIdempotencyKeyLen bounds the client-supplied Idempotency-Key itself, so
// a pathologically large header cannot be used as an oversized Redis key.
// 255 matches the common convention (Stripe, among others) for this header.
const maxIdempotencyKeyLen = 255

// idempotencyRecord is what gets persisted to Redis under the client's key:
// enough to both detect a conflicting reuse (Fingerprint) and replay the
// original response byte-for-byte (Status/Header/Body).
type idempotencyRecord struct {
	Fingerprint string      `json:"fingerprint"`
	Status      int         `json:"status"`
	Header      http.Header `json:"header"`
	Body        []byte      `json:"body"`
}

// Idempotency returns middleware that honours an `Idempotency-Key` header on
// an unsafe POST endpoint (issue #225): a retry carrying the same key and an
// identical request body replays the first attempt's response instead of
// re-executing the handler — so a client retrying after a network blip (but
// whose first request actually succeeded) cannot double-create a resource.
// The same key reused with a *different* body is rejected as a conflict
// rather than either silently executing twice or replaying the wrong
// response.
//
// A request with no Idempotency-Key header passes straight through —
// support is opt-in, matching the issue's "Accept an Idempotency-Key
// header", not a requirement that every POST supply one. It is also a
// no-op when rdb is nil (Redis unavailable), the same fail-open posture
// AuditMiddleware and the auth Redis cache take elsewhere in this service:
// idempotency protection is best-effort, and refusing every create request
// because Redis is down would be a worse outage than the problem it guards
// against.
//
// ttl <= 0 uses DefaultIdempotencyTTL.
func Idempotency(rdb *redis.Client, ttl time.Duration) func(http.Handler) http.Handler {
	if ttl <= 0 {
		ttl = DefaultIdempotencyTTL
	}
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			key := r.Header.Get("Idempotency-Key")
			if key == "" || rdb == nil {
				next.ServeHTTP(w, r)
				return
			}
			if len(key) > maxIdempotencyKeyLen {
				httputil.WriteErrorCtx(r.Context(), w, http.StatusBadRequest, httputil.INVALID_ARGUMENT,
					"Idempotency-Key exceeds maximum length")
				return
			}

			// The body is needed twice — once here to fingerprint it, once
			// by the real handler — so it is read fully up front and
			// replaced with a fresh reader over the same bytes.
			body, err := io.ReadAll(io.LimitReader(r.Body, maxIdempotencyBodyBytes+1))
			_ = r.Body.Close()
			if err != nil {
				httputil.WriteErrorCtx(r.Context(), w, http.StatusBadRequest, httputil.INVALID_ARGUMENT,
					"failed to read request body")
				return
			}
			if len(body) > maxIdempotencyBodyBytes {
				httputil.WriteErrorCtx(r.Context(), w, http.StatusRequestEntityTooLarge, httputil.PAYLOAD_TOO_LARGE,
					"request body too large")
				return
			}
			r.Body = io.NopCloser(bytes.NewReader(body))

			fingerprint := fingerprintIdempotentRequest(r.Method, r.URL.Path, body)
			redisKey := "idempotency:" + key

			if cached, err := rdb.Get(r.Context(), redisKey).Result(); err == nil {
				var rec idempotencyRecord
				if jsonErr := json.Unmarshal([]byte(cached), &rec); jsonErr == nil {
					if rec.Fingerprint != fingerprint {
						httputil.WriteErrorCtx(r.Context(), w, http.StatusConflict, httputil.CONFLICT,
							"Idempotency-Key was already used with a different request")
						return
					}
					replayIdempotentResponse(w, rec)
					return
				}
				// A malformed cache entry should not be possible — this
				// middleware is the only writer — but treating it as a miss
				// re-executes the handler rather than failing the request.
				slog.WarnContext(r.Context(), "idempotency: malformed cache entry; re-executing", "key", key)
			} else if err != redis.Nil {
				slog.WarnContext(r.Context(), "idempotency: redis get failed; proceeding without replay", "err", err)
			}

			capture := newCaptureWriter()
			next.ServeHTTP(capture, r)
			capture.flushTo(w)

			rec := idempotencyRecord{
				Fingerprint: fingerprint,
				Status:      capture.statusCode,
				Header:      capture.header,
				Body:        capture.body.Bytes(),
			}
			if data, err := json.Marshal(rec); err == nil {
				if err := rdb.Set(r.Context(), redisKey, data, ttl).Err(); err != nil {
					slog.WarnContext(r.Context(), "idempotency: redis set failed; a retry will re-execute", "err", err)
				}
			}
		})
	}
}

// fingerprintIdempotentRequest identifies "the same request" for conflict
// detection: method, path, and body must all match a stored record for a
// key reuse to be treated as a legitimate retry rather than a conflicting
// second use of the same key.
func fingerprintIdempotentRequest(method, path string, body []byte) string {
	h := sha256.New()
	h.Write([]byte(method))
	h.Write([]byte{0})
	h.Write([]byte(path))
	h.Write([]byte{0})
	h.Write(body)
	return hex.EncodeToString(h.Sum(nil))
}

// replayIdempotentResponse writes a previously-captured response byte-for-
// byte, so a retried request cannot tell it didn't re-execute the handler.
func replayIdempotentResponse(w http.ResponseWriter, rec idempotencyRecord) {
	for k, vv := range rec.Header {
		for _, v := range vv {
			w.Header().Add(k, v)
		}
	}
	w.Header().Set("Idempotent-Replayed", "true")
	w.WriteHeader(rec.Status)
	_, _ = w.Write(rec.Body)
}

// captureWriter buffers a handler's full response (headers, status, body) in
// memory instead of streaming it, so nothing is sent to the real client
// until the response is known — and can be persisted for replay — in full.
// Safe here because both routes Idempotency wraps return a single small
// JSON object, never a stream.
type captureWriter struct {
	header      http.Header
	statusCode  int
	body        bytes.Buffer
	wroteHeader bool
}

func newCaptureWriter() *captureWriter {
	return &captureWriter{header: make(http.Header), statusCode: http.StatusOK}
}

func (c *captureWriter) Header() http.Header { return c.header }

func (c *captureWriter) WriteHeader(code int) {
	if !c.wroteHeader {
		c.statusCode = code
		c.wroteHeader = true
	}
}

func (c *captureWriter) Write(b []byte) (int, error) {
	if !c.wroteHeader {
		c.WriteHeader(http.StatusOK)
	}
	return c.body.Write(b)
}

// flushTo writes the captured response to the real ResponseWriter, once the
// underlying handler has finished.
func (c *captureWriter) flushTo(w http.ResponseWriter) {
	for k, vv := range c.header {
		for _, v := range vv {
			w.Header().Add(k, v)
		}
	}
	w.WriteHeader(c.statusCode)
	_, _ = w.Write(c.body.Bytes())
}
