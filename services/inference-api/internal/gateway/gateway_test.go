package gateway

import (
	"bytes"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/config"
)

func TestHealthAndReadiness(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/health" {
			http.NotFound(w, r)
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"status":"ok"}`))
	}))
	defer upstream.Close()

	server := httptest.NewServer(newTestGateway(t, upstream.URL, 2, 100*time.Millisecond).Handler())
	defer server.Close()

	for _, path := range []string{"/healthz", "/readyz"} {
		response, err := http.Get(server.URL + path)
		if err != nil {
			t.Fatalf("GET %s: %v", path, err)
		}
		io.Copy(io.Discard, response.Body)
		response.Body.Close()
		if response.StatusCode != http.StatusOK {
			t.Fatalf("GET %s status = %d, want 200", path, response.StatusCode)
		}
	}
}

func TestChatStreamingPassThrough(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != chatPath {
			http.NotFound(w, r)
			return
		}
		if got := r.Header.Get("X-OpenMindAI-Gateway"); got != "go" {
			t.Errorf("gateway header = %q, want go", got)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		flusher, _ := w.(http.Flusher)
		_, _ = w.Write([]byte("data: {\"chunk\":\"one\"}\n\n"))
		if flusher != nil {
			flusher.Flush()
		}
		_, _ = w.Write([]byte("data: {\"chunk\":\"two\"}\n\n"))
	}))
	defer upstream.Close()

	server := httptest.NewServer(newTestGateway(t, upstream.URL, 2, 100*time.Millisecond).Handler())
	defer server.Close()

	response, err := http.Post(
		server.URL+chatPath,
		"application/json",
		strings.NewReader(`{"model":"qwen","messages":[{"role":"user","content":"hi"}],"stream":true}`),
	)
	if err != nil {
		t.Fatalf("POST chat: %v", err)
	}
	defer response.Body.Close()

	body, err := io.ReadAll(response.Body)
	if err != nil {
		t.Fatalf("read streaming response: %v", err)
	}
	if response.StatusCode != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", response.StatusCode, body)
	}
	if contentType := response.Header.Get("Content-Type"); !strings.Contains(contentType, "text/event-stream") {
		t.Fatalf("content type = %q, want text/event-stream", contentType)
	}
	if !bytes.Contains(body, []byte(`"one"`)) || !bytes.Contains(body, []byte(`"two"`)) {
		t.Fatalf("streaming body missing chunks: %s", body)
	}
}

func TestSaturatedQueueReturns429(t *testing.T) {
	entered := make(chan struct{}, 1)
	release := make(chan struct{})
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		entered <- struct{}{}
		<-release
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"ok":true}`))
	}))
	defer upstream.Close()

	server := httptest.NewServer(newTestGateway(t, upstream.URL, 1, 25*time.Millisecond).Handler())
	defer server.Close()

	firstDone := make(chan error, 1)
	go func() {
		response, err := http.Post(server.URL+chatPath, "application/json", strings.NewReader(`{"stream":false}`))
		if err == nil {
			io.Copy(io.Discard, response.Body)
			response.Body.Close()
		}
		firstDone <- err
	}()

	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("first request did not reach upstream")
	}

	response, err := http.Post(server.URL+chatPath, "application/json", strings.NewReader(`{"stream":false}`))
	if err != nil {
		t.Fatalf("second POST: %v", err)
	}
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	if response.StatusCode != http.StatusTooManyRequests {
		t.Fatalf("second status = %d, want 429", response.StatusCode)
	}

	close(release)
	select {
	case err := <-firstDone:
		if err != nil {
			t.Fatalf("first request failed: %v", err)
		}
	case <-time.After(time.Second):
		t.Fatal("first request did not complete")
	}
}

func TestRejectsOversizedBody(t *testing.T) {
	upstream := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		t.Fatal("oversized request should not reach upstream")
	}))
	defer upstream.Close()

	gateway := newTestGateway(t, upstream.URL, 1, 100*time.Millisecond)
	gateway.maxBodyBytes = 8
	server := httptest.NewServer(gateway.Handler())
	defer server.Close()

	response, err := http.Post(server.URL+chatPath, "application/json", strings.NewReader(`{"message":"too large"}`))
	if err != nil {
		t.Fatalf("POST oversized body: %v", err)
	}
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	if response.StatusCode != http.StatusRequestEntityTooLarge {
		t.Fatalf("status = %d, want 413", response.StatusCode)
	}
}

func newTestGateway(t *testing.T, upstream string, maxInflight int, queueTimeout time.Duration) *Gateway {
	t.Helper()
	parsed, err := url.Parse(upstream)
	if err != nil {
		t.Fatalf("parse upstream: %v", err)
	}
	return New(config.Config{
		UpstreamURL:          parsed,
		MaxInflight:          maxInflight,
		QueueTimeout:         queueTimeout,
		UpstreamReadyTimeout: 250 * time.Millisecond,
		MaxBodyBytes:         1 << 20,
	})
}
