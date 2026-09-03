package gateway

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/config"
	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/nativeworker"
)

type fakeNative struct {
	calls int
	fail  bool
}

func (f *fakeNative) Ready(context.Context) error { return nil }
func (f *fakeNative) Close()                      {}
func (f *fakeNative) Generate(ctx context.Context, req map[string]any, token func(string) error) error {
	f.calls++
	if req["model"] == "missing" {
		return &nativeworker.RemoteError{Code: "model_not_found", Message: "model is not registered"}
	}
	if req["model"] == "timeout" {
		<-ctx.Done()
		return ctx.Err()
	}
	if err := token("বাংলা 🌼"); err != nil {
		return err
	}
	if f.fail {
		return fmt.Errorf("private /models/path.gguf error")
	}
	return nil
}
func nativeGateway(f nativeBackend) *Gateway {
	g := New(config.Config{MaxInflight: 1, MaxBodyBytes: 1 << 20, QueueTimeout: time.Millisecond, GenerationTimeout: 20 * time.Millisecond})
	g.native = f
	return g
}
func TestNativeChatContract(t *testing.T) {
	f := &fakeNative{}
	g := nativeGateway(f)
	defer g.Close()
	for _, tc := range []struct {
		body     string
		status   int
		contains string
	}{
		{`{"model":"nano","messages":[{"role":"user","content":"hi"}]}`, 200, "বাংলা 🌼"},
		{`{"model":"nano","stream":true,"messages":[{"role":"user","content":"hi"}]}`, 200, "[DONE]"},
		{`{"model":"missing","messages":[{"role":"user","content":"hi"}]}`, 404, "not registered"},
		{`{"model":"timeout","messages":[{"role":"user","content":"hi"}]}`, 504, "deadline exceeded"},
		{`{"model":"nano","model_path":"/tmp/arbitrary","messages":[{"role":"user","content":"hi"}]}`, 400, "unsupported"},
		{`{"model":"nano","max_tokens":9000,"messages":[{"role":"user","content":"hi"}]}`, 400, "limits"},
		{`{"model":"nano","messages":[{"role":"tool","content":"hi"}]}`, 400, "roles"},
	} {
		w := httptest.NewRecorder()
		g.Handler().ServeHTTP(w, nativeHTTPRequest(tc.body))
		if w.Code != tc.status || !strings.Contains(w.Body.String(), tc.contains) {
			t.Fatalf("%d %s", w.Code, w.Body.String())
		}
	}
	if f.calls != 4 {
		t.Fatalf("invalid requests reached worker: %d", f.calls)
	}
	f.fail = true
	w := httptest.NewRecorder()
	g.Handler().ServeHTTP(w, nativeHTTPRequest(`{"model":"nano","stream":true,"messages":[{"role":"user","content":"hi"}]}`))
	if !strings.Contains(w.Body.String(), "native_error") || strings.Contains(w.Body.String(), "[DONE]") || strings.Contains(w.Body.String(), "/models/") {
		t.Fatal(w.Body.String())
	}
}

// CI supplies the compiled production Rust/CXX worker and pinned GGUF fixture.
func TestRealNativeWorker(t *testing.T) {
	executable, model := os.Getenv("OPENMINDAI_TEST_NATIVE_WORKER"), os.Getenv("OPENMINDAI_TEST_MODEL")
	if executable == "" || model == "" {
		t.Skip("native integration fixture not configured")
	}
	executable, _ = filepath.Abs(executable)
	model, _ = filepath.Abs(model)
	registry := filepath.Join(t.TempDir(), "models.json")
	data, _ := json.Marshal(map[string]any{"nano": map[string]any{"path": model, "context_size": 512}})
	if err := os.WriteFile(registry, data, 0600); err != nil {
		t.Fatal(err)
	}
	client := nativeworker.New(executable, "--models", registry)
	g := nativeGateway(client)
	g.generationTimeout = 20 * time.Second
	defer g.Close()
	for _, stream := range []bool{false, true, false} {
		body := fmt.Sprintf(`{"model":"nano","stream":%t,"max_tokens":16,"temperature":0,"messages":[{"role":"user","content":"Once upon a time, the child"}]}`, stream)
		w := httptest.NewRecorder()
		g.Handler().ServeHTTP(w, nativeHTTPRequest(body))
		if w.Code != 200 || strings.Contains(w.Body.String(), `"error"`) {
			t.Fatalf("native response: %d %s", w.Code, w.Body.String())
		}
		if stream && !strings.Contains(w.Body.String(), "[DONE]") {
			t.Fatal(w.Body.String())
		}
		if !stream {
			var v struct {
				Choices []struct{ Message struct{ Content string } }
			}
			if json.Unmarshal(w.Body.Bytes(), &v) != nil || len(v.Choices) != 1 || v.Choices[0].Message.Content == "" {
				t.Fatal(w.Body.String())
			}
		}
	}
	// Kill mid-stream, then require another real generation from a fresh worker.
	ctx, cancel := context.WithCancel(context.Background())
	err := client.Generate(ctx, map[string]any{"model": "nano", "max_tokens": 64, "messages": []map[string]string{{"role": "user", "content": "Once upon a time"}}}, func(string) error { cancel(); return ctx.Err() })
	if err == nil {
		t.Fatal("cancellation unexpectedly succeeded")
	}
	ctx, cancel = context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	var output strings.Builder
	if err = client.Generate(ctx, map[string]any{"model": "nano", "max_tokens": 16, "messages": []map[string]string{{"role": "user", "content": "Once upon a time"}}}, func(s string) error { output.WriteString(s); return nil }); err != nil || output.Len() == 0 {
		t.Fatalf("recovery: %v %s", err, output.String())
	}
}

func nativeHTTPRequest(body string) *http.Request {
	r := httptest.NewRequest(http.MethodPost, chatPath, strings.NewReader(body))
	r.Header.Set("Content-Type", "application/json")
	return r
}
