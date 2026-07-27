package grpc

import (
	"context"
	"fmt"
	"log/slog"
	"time"

	"github.com/Depo-dev/trident/services/api/gen"
	"github.com/Depo-dev/trident/services/api/middleware"
	"go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
)

// metricsUnaryInterceptor records trident_api_grpc_client_requests_total /
// trident_api_grpc_client_request_duration_seconds for every unary call to
// the internal events backend (#297 — gRPC call latency/errors).
func metricsUnaryInterceptor(
	ctx context.Context,
	method string,
	req, reply any,
	cc *grpc.ClientConn,
	invoker grpc.UnaryInvoker,
	opts ...grpc.CallOption,
) error {
	start := time.Now()
	err := invoker(ctx, method, req, reply, cc, opts...)
	middleware.RecordGRPCClientCall(method, status.Code(err).String(), time.Since(start).Seconds())
	return err
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
		grpc.WithChainUnaryInterceptor(metricsUnaryInterceptor),
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
