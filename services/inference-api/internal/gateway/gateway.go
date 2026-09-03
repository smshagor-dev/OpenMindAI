package gateway

import (
	"bytes"
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/config"
	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/nativeworker"
)

const (
	chatPath   = "/v1/chat/completions"
	healthPath = "/healthz"
	readyPath  = "/readyz"
)

type Gateway struct {
	native            nativeBackend
	generationTimeout time.Duration
	upstream          *url.URL
	client            *http.Client
	slots             chan struct{}
	queueTimeout      time.Duration
	readyTimeout      time.Duration
	maxBodyBytes      int64
}

func New(cfg config.Config) *Gateway {
	maxIdlePerHost := cfg.MaxInflight * 2
	if maxIdlePerHost < 8 {
		maxIdlePerHost = 8
	}

	transport := &http.Transport{
		Proxy:                 http.ProxyFromEnvironment,
		ForceAttemptHTTP2:     true,
		MaxIdleConns:          128,
		MaxIdleConnsPerHost:   maxIdlePerHost,
		MaxConnsPerHost:       cfg.MaxInflight + maxIdlePerHost,
		IdleConnTimeout:       90 * time.Second,
		TLSHandshakeTimeout:   10 * time.Second,
		ResponseHeaderTimeout: 2 * time.Minute,
		ExpectContinueTimeout: time.Second,
		DisableCompression:    true,
	}

	g := &Gateway{
		upstream: cfg.UpstreamURL,
		client: &http.Client{
			Transport: transport,
		},
		slots:        make(chan struct{}, cfg.MaxInflight),
		queueTimeout: cfg.QueueTimeout,
		readyTimeout: cfg.UpstreamReadyTimeout,
		maxBodyBytes: cfg.MaxBodyBytes,
	}
	if cfg.Backend == "native" {
		g.native = nativeworker.New(cfg.NativeWorker, "--models", cfg.NativeModels)
		g.slots = make(chan struct{}, 1)
		if g.maxBodyBytes > MaxNativeBody {
			g.maxBodyBytes = MaxNativeBody
		}
	}
	g.generationTimeout = cfg.GenerationTimeout
	if g.generationTimeout == 0 {
		g.generationTimeout = 120 * time.Second
	}
	return g
}
func (g *Gateway) Close() {
	if g.native != nil {
		g.native.Close()
	}
	g.client.CloseIdleConnections()
}

func (g *Gateway) Handler() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc(healthPath, g.handleHealth)
	mux.HandleFunc(readyPath, g.handleReady)
	mux.HandleFunc(chatPath, g.handleChat)
	return requestIDMiddleware(recoveryMiddleware(mux))
}

func (g *Gateway) handleHealth(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		methodNotAllowed(w, http.MethodGet)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"status":  "ok",
		"service": "openmind-inference-api",
	})
}

func (g *Gateway) handleReady(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		methodNotAllowed(w, http.MethodGet)
		return
	}

	ctx, cancel := context.WithTimeout(r.Context(), g.readyTimeout)
	defer cancel()

	if g.native != nil {
		if err := g.native.Ready(ctx); err != nil {
			writeJSON(w, http.StatusServiceUnavailable, map[string]any{"status": "not_ready", "backend": "native"})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"status": "ready", "backend": "native"})
		return
	}
	ready, detail := g.upstreamReady(ctx)
	if !ready {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{
			"status":   "not_ready",
			"upstream": g.upstream.Host,
			"detail":   detail,
		})
		return
	}

	writeJSON(w, http.StatusOK, map[string]any{
		"status":   "ready",
		"upstream": g.upstream.Host,
	})
}

func (g *Gateway) handleChat(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		methodNotAllowed(w, http.MethodPost)
		return
	}

	if g.native != nil {
		media, _, err := mime.ParseMediaType(r.Header.Get("Content-Type"))
		if err != nil || media != "application/json" {
			writeError(w, http.StatusUnsupportedMediaType, "native chat requires application/json")
			return
		}
		controller := http.NewResponseController(w)
		_ = controller.SetReadDeadline(time.Now().Add(15 * time.Second))
		defer func() { _ = controller.SetReadDeadline(time.Time{}) }()
	}
	body, err := readBoundedBody(r.Body, g.maxBodyBytes)
	if err != nil {
		if errors.Is(err, errBodyTooLarge) {
			writeError(w, http.StatusRequestEntityTooLarge, "request body exceeds configured limit")
			return
		}
		writeError(w, http.StatusBadRequest, "unable to read request body")
		return
	}
	if len(bytes.TrimSpace(body)) == 0 || !json.Valid(body) {
		writeError(w, http.StatusBadRequest, "request body must be valid JSON")
		return
	}

	if !g.acquire(r.Context()) {
		if r.Context().Err() != nil {
			return
		}
		w.Header().Set("Retry-After", "1")
		writeError(w, http.StatusTooManyRequests, "inference queue is saturated")
		return
	}
	defer g.release()

	if g.native != nil {
		g.handleNative(w, r, body)
		return
	}
	target := g.resolve(chatPath)
	upstreamRequest, err := http.NewRequestWithContext(r.Context(), http.MethodPost, target.String(), bytes.NewReader(body))
	if err != nil {
		writeError(w, http.StatusInternalServerError, "failed to create upstream request")
		return
	}
	copyRequestHeaders(upstreamRequest.Header, r.Header)
	if upstreamRequest.Header.Get("Content-Type") == "" {
		upstreamRequest.Header.Set("Content-Type", "application/json")
	}
	upstreamRequest.Header.Set("X-OpenMindAI-Gateway", "go")

	response, err := g.client.Do(upstreamRequest)
	if err != nil {
		if r.Context().Err() != nil {
			return
		}
		writeError(w, http.StatusBadGateway, "inference upstream is unavailable")
		return
	}
	defer response.Body.Close()

	copyResponseHeaders(w.Header(), response.Header)
	w.Header().Set("X-OpenMindAI-Gateway", "go")
	if strings.Contains(strings.ToLower(response.Header.Get("Content-Type")), "text/event-stream") {
		w.Header().Set("Cache-Control", "no-cache, no-transform")
		w.Header().Set("X-Accel-Buffering", "no")
	}
	w.WriteHeader(response.StatusCode)

	if err := streamResponse(w, response.Body); err != nil && r.Context().Err() == nil {
		return
	}
}

func (g *Gateway) acquire(ctx context.Context) bool {
	select {
	case g.slots <- struct{}{}:
		return true
	default:
	}

	timer := time.NewTimer(g.queueTimeout)
	defer timer.Stop()
	select {
	case g.slots <- struct{}{}:
		return true
	case <-ctx.Done():
		return false
	case <-timer.C:
		return false
	}
}

func (g *Gateway) release() {
	<-g.slots
}

func (g *Gateway) upstreamReady(ctx context.Context) (bool, string) {
	paths := []string{"/health", "/v1/models"}
	var lastDetail string
	for _, path := range paths {
		target := g.resolve(path)
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, target.String(), nil)
		if err != nil {
			return false, err.Error()
		}
		response, err := g.client.Do(request)
		if err != nil {
			if ctx.Err() != nil {
				return false, ctx.Err().Error()
			}
			lastDetail = err.Error()
			continue
		}
		io.Copy(io.Discard, io.LimitReader(response.Body, 4<<10))
		response.Body.Close()
		if response.StatusCode >= 200 && response.StatusCode < 300 {
			return true, ""
		}
		lastDetail = fmt.Sprintf("%s returned HTTP %d", path, response.StatusCode)
	}
	if lastDetail == "" {
		lastDetail = "no readiness endpoint succeeded"
	}
	return false, lastDetail
}

func (g *Gateway) resolve(path string) *url.URL {
	resolved := *g.upstream
	basePath := strings.TrimRight(resolved.Path, "/")
	resolved.Path = basePath + path
	resolved.RawPath = ""
	resolved.RawQuery = ""
	resolved.Fragment = ""
	return &resolved
}

var errBodyTooLarge = errors.New("body too large")

func readBoundedBody(body io.ReadCloser, maxBytes int64) ([]byte, error) {
	defer body.Close()
	reader := io.LimitReader(body, maxBytes+1)
	content, err := io.ReadAll(reader)
	if err != nil {
		return nil, err
	}
	if int64(len(content)) > maxBytes {
		return nil, errBodyTooLarge
	}
	return content, nil
}

func streamResponse(w http.ResponseWriter, source io.Reader) error {
	flusher, canFlush := w.(http.Flusher)
	buffer := make([]byte, 32<<10)
	for {
		n, err := source.Read(buffer)
		if n > 0 {
			if _, writeErr := w.Write(buffer[:n]); writeErr != nil {
				return writeErr
			}
			if canFlush {
				flusher.Flush()
			}
		}
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil
			}
			return err
		}
	}
}

func requestIDMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestID := strings.TrimSpace(r.Header.Get("X-Request-ID"))
		if requestID == "" || len(requestID) > 128 {
			requestID = newRequestID()
		}
		r.Header.Set("X-Request-ID", requestID)
		w.Header().Set("X-Request-ID", requestID)
		next.ServeHTTP(w, r)
	})
}

func recoveryMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer func() {
			if recover() != nil {
				writeError(w, http.StatusInternalServerError, "internal gateway error")
			}
		}()
		next.ServeHTTP(w, r)
	})
}

func newRequestID() string {
	var raw [12]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return fmt.Sprintf("om-%d", time.Now().UnixNano())
	}
	return "om-" + hex.EncodeToString(raw[:])
}

func copyRequestHeaders(dst, src http.Header) {
	for key, values := range src {
		if isHopByHopHeader(key) {
			continue
		}
		for _, value := range values {
			dst.Add(key, value)
		}
	}
}

func copyResponseHeaders(dst, src http.Header) {
	for key, values := range src {
		if isHopByHopHeader(key) {
			continue
		}
		for _, value := range values {
			dst.Add(key, value)
		}
	}
}

func isHopByHopHeader(key string) bool {
	switch http.CanonicalHeaderKey(key) {
	case "Connection", "Keep-Alive", "Proxy-Authenticate", "Proxy-Authorization", "Te", "Trailer", "Transfer-Encoding", "Upgrade":
		return true
	default:
		return false
	}
}

func methodNotAllowed(w http.ResponseWriter, allowed string) {
	w.Header().Set("Allow", allowed)
	writeError(w, http.StatusMethodNotAllowed, "method not allowed")
}

func writeError(w http.ResponseWriter, status int, message string) {
	writeJSON(w, status, map[string]any{
		"error": map[string]any{
			"message": message,
			"type":    "gateway_error",
		},
	})
}

func writeJSON(w http.ResponseWriter, status int, value any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(value)
}
