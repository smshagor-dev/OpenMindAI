package config

import (
	"fmt"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

const (
	defaultListenAddress = "127.0.0.1:11435"
	defaultUpstreamURL   = "http://127.0.0.1:8080"
)

type Config struct {
	ListenAddress        string
	UpstreamURL          *url.URL
	MaxInflight          int
	QueueTimeout         time.Duration
	ShutdownTimeout      time.Duration
	ReadHeaderTimeout    time.Duration
	IdleTimeout          time.Duration
	UpstreamReadyTimeout time.Duration
	MaxBodyBytes         int64
}

func Load() (Config, error) {
	upstream, err := parseHTTPURL(envOr("OPENMINDAI_INFERENCE_UPSTREAM", defaultUpstreamURL))
	if err != nil {
		return Config{}, fmt.Errorf("OPENMINDAI_INFERENCE_UPSTREAM: %w", err)
	}

	maxInflight, err := envPositiveInt("OPENMINDAI_API_MAX_INFLIGHT", 4)
	if err != nil {
		return Config{}, err
	}
	maxBodyBytes, err := envPositiveInt64("OPENMINDAI_API_MAX_BODY_BYTES", 8<<20)
	if err != nil {
		return Config{}, err
	}
	queueTimeout, err := envDuration("OPENMINDAI_API_QUEUE_TIMEOUT", 2*time.Second)
	if err != nil {
		return Config{}, err
	}
	shutdownTimeout, err := envDuration("OPENMINDAI_API_SHUTDOWN_TIMEOUT", 10*time.Second)
	if err != nil {
		return Config{}, err
	}
	readHeaderTimeout, err := envDuration("OPENMINDAI_API_READ_HEADER_TIMEOUT", 10*time.Second)
	if err != nil {
		return Config{}, err
	}
	idleTimeout, err := envDuration("OPENMINDAI_API_IDLE_TIMEOUT", 2*time.Minute)
	if err != nil {
		return Config{}, err
	}
	readyTimeout, err := envDuration("OPENMINDAI_API_READY_TIMEOUT", 2*time.Second)
	if err != nil {
		return Config{}, err
	}

	listenAddress := strings.TrimSpace(envOr("OPENMINDAI_API_ADDR", defaultListenAddress))
	if listenAddress == "" {
		return Config{}, fmt.Errorf("OPENMINDAI_API_ADDR must not be empty")
	}

	return Config{
		ListenAddress:        listenAddress,
		UpstreamURL:          upstream,
		MaxInflight:          maxInflight,
		QueueTimeout:         queueTimeout,
		ShutdownTimeout:      shutdownTimeout,
		ReadHeaderTimeout:    readHeaderTimeout,
		IdleTimeout:          idleTimeout,
		UpstreamReadyTimeout: readyTimeout,
		MaxBodyBytes:         maxBodyBytes,
	}, nil
}

func parseHTTPURL(raw string) (*url.URL, error) {
	parsed, err := url.Parse(strings.TrimSpace(raw))
	if err != nil {
		return nil, err
	}
	if parsed.Scheme != "http" && parsed.Scheme != "https" {
		return nil, fmt.Errorf("scheme must be http or https")
	}
	if parsed.Host == "" {
		return nil, fmt.Errorf("host is required")
	}
	if parsed.User != nil {
		return nil, fmt.Errorf("userinfo is not supported")
	}
	parsed.Path = strings.TrimRight(parsed.Path, "/")
	parsed.RawQuery = ""
	parsed.Fragment = ""
	return parsed, nil
}

func envOr(key, fallback string) string {
	if value, ok := os.LookupEnv(key); ok {
		return value
	}
	return fallback
}

func envPositiveInt(key string, fallback int) (int, error) {
	value := strings.TrimSpace(envOr(key, strconv.Itoa(fallback)))
	parsed, err := strconv.Atoi(value)
	if err != nil || parsed <= 0 {
		return 0, fmt.Errorf("%s must be a positive integer", key)
	}
	return parsed, nil
}

func envPositiveInt64(key string, fallback int64) (int64, error) {
	value := strings.TrimSpace(envOr(key, strconv.FormatInt(fallback, 10)))
	parsed, err := strconv.ParseInt(value, 10, 64)
	if err != nil || parsed <= 0 {
		return 0, fmt.Errorf("%s must be a positive integer", key)
	}
	return parsed, nil
}

func envDuration(key string, fallback time.Duration) (time.Duration, error) {
	value := strings.TrimSpace(envOr(key, fallback.String()))
	parsed, err := time.ParseDuration(value)
	if err != nil || parsed <= 0 {
		return 0, fmt.Errorf("%s must be a positive Go duration", key)
	}
	return parsed, nil
}
