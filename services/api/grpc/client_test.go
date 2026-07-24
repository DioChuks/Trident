package grpc

import (
	"context"
	"testing"

	"github.com/Depo-dev/trident/services/api/internal/httputil"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// TestRequestIDUnaryInterceptor_PropagatesMetadata asserts the request id on
// the call context is copied into outgoing gRPC metadata.
func TestRequestIDUnaryInterceptor_PropagatesMetadata(t *testing.T) {
	const id = "grpc-prop-99"
	ctx := httputil.ContextWithRequestID(context.Background(), id)

	var got metadata.MD
	invoker := func(ctx context.Context, _ string, _, _ any, _ *grpc.ClientConn, _ ...grpc.CallOption) error {
		got, _ = metadata.FromOutgoingContext(ctx)
		return nil
	}

	err := requestIDUnaryInterceptor(ctx, "/svc/Method", nil, nil, nil, invoker)
	if err != nil {
		t.Fatalf("interceptor returned error: %v", err)
	}
	if vals := got.Get(requestIDMetadataKey); len(vals) != 1 || vals[0] != id {
		t.Fatalf("metadata %s = %v, want [%s]", requestIDMetadataKey, got.Get(requestIDMetadataKey), id)
	}
}

// TestRequestIDUnaryInterceptor_NoIDNoMetadata asserts no request-id metadata
// is attached when the context carries none.
func TestRequestIDUnaryInterceptor_NoIDNoMetadata(t *testing.T) {
	var got metadata.MD
	invoker := func(ctx context.Context, _ string, _, _ any, _ *grpc.ClientConn, _ ...grpc.CallOption) error {
		got, _ = metadata.FromOutgoingContext(ctx)
		return nil
	}

	if err := requestIDUnaryInterceptor(context.Background(), "/svc/Method", nil, nil, nil, invoker); err != nil {
		t.Fatalf("interceptor returned error: %v", err)
	}
	if vals := got.Get(requestIDMetadataKey); len(vals) != 0 {
		t.Fatalf("expected no request-id metadata, got %v", vals)
	}
}
