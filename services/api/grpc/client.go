package grpc

import (
	"context"
	"fmt"
	"log/slog"

	"github.com/Depo-dev/trident/services/api/gen"
	"github.com/Depo-dev/trident/services/api/internal/httputil"
	"go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/metadata"
)

// requestIDMetadataKey is the gRPC metadata key used to propagate the API
// request id to downstream services. Lower-case per gRPC metadata convention.
const requestIDMetadataKey = "x-request-id"

// requestIDUnaryInterceptor copies the request id attached to the call context
// by the RequestID middleware into outgoing gRPC metadata, so a request can be
// correlated across the API gateway and backend services.
func requestIDUnaryInterceptor(
	ctx context.Context,
	method string,
	req, reply any,
	cc *grpc.ClientConn,
	invoker grpc.UnaryInvoker,
	opts ...grpc.CallOption,
) error {
	if id := httputil.RequestIDFromContext(ctx); id != "" {
		ctx = metadata.AppendToOutgoingContext(ctx, requestIDMetadataKey, id)
	}
	return invoker(ctx, method, req, reply, cc, opts...)
}

// Client wraps the gRPC connection and client
type Client struct {
	conn *grpc.ClientConn
	gen.EventsClient
}

// NewClient creates a new gRPC client connection
func NewClient(_ context.Context, addr string) (*Client, error) {
	conn, err := grpc.NewClient(
		addr,
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(10*1024*1024)),
		grpc.WithStatsHandler(otelgrpc.NewClientHandler()),
		grpc.WithChainUnaryInterceptor(requestIDUnaryInterceptor),
	)
	if err != nil {
		return nil, fmt.Errorf("failed to dial gRPC server: %w", err)
	}

	slog.Info("connected to gRPC server", "addr", addr)
	return &Client{
		conn:         conn,
		EventsClient: gen.NewEventsClient(conn),
	}, nil
}

// Close closes the gRPC connection
func (c *Client) Close() error {
	return c.conn.Close()
}
