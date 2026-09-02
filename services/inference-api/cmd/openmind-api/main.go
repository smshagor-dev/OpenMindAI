package main

import (
	"context"
	"errors"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"

	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/config"
	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/gateway"
)

func main() {
	cfg, err := config.Load()
	if err != nil {
		log.Fatalf("configuration error: %v", err)
	}

	api := gateway.New(cfg)
	server := &http.Server{
		Addr:              cfg.ListenAddress,
		Handler:           api.Handler(),
		ReadHeaderTimeout: cfg.ReadHeaderTimeout,
		IdleTimeout:       cfg.IdleTimeout,
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	errCh := make(chan error, 1)
	go func() {
		log.Printf("OpenMindAI inference API listening on http://%s (upstream=%s)", cfg.ListenAddress, cfg.UpstreamURL.String())
		errCh <- server.ListenAndServe()
	}()

	select {
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), cfg.ShutdownTimeout)
		defer cancel()
		if err := server.Shutdown(shutdownCtx); err != nil {
			log.Printf("graceful shutdown failed: %v", err)
			_ = server.Close()
		}
	case err := <-errCh:
		if err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Fatalf("inference API stopped unexpectedly: %v", err)
		}
	}
}
