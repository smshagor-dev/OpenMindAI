package gateway

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/smshagor-dev/OpenMindAI/services/inference-api/internal/nativeworker"
)

type nativeBackend interface {
	Ready(context.Context) error
	Generate(context.Context, map[string]any, func(string) error) error
	Close()
}

type nativeMessage struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}
type nativeRequest struct {
	Model       string          `json:"model"`
	Messages    []nativeMessage `json:"messages"`
	Stream      bool            `json:"stream"`
	Temperature *float32        `json:"temperature,omitempty"`
	TopP        *float32        `json:"top_p,omitempty"`
	MaxTokens   *uint32         `json:"max_tokens,omitempty"`
}

func decodeNative(body []byte) (nativeRequest, error) {
	var req nativeRequest
	decoder := json.NewDecoder(bytes.NewReader(body))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&req); err != nil {
		return req, errors.New("unsupported or invalid native chat fields")
	}
	if decoder.Decode(new(any)) != io.EOF {
		return req, errors.New("expected one chat request")
	}
	if len(req.Model) == 0 || len(req.Model) > 128 || len(req.Messages) == 0 || len(req.Messages) > 256 {
		return req, errors.New("model and 1..256 messages are required")
	}
	size := 0
	for _, m := range req.Messages {
		if (m.Role != "system" && m.Role != "user" && m.Role != "assistant") || strings.TrimSpace(m.Content) == "" {
			return req, errors.New("native chat requires nonempty text messages with system, user or assistant roles")
		}
		size += len(m.Content)
	}
	if size > MaxNativeBody || (req.MaxTokens != nil && (*req.MaxTokens == 0 || *req.MaxTokens > 8192)) || (req.Temperature != nil && (*req.Temperature < 0 || *req.Temperature > 2)) || (req.TopP != nil && (*req.TopP <= 0 || *req.TopP > 1)) {
		return req, errors.New("native generation resource limits exceeded")
	}
	return req, nil
}

const MaxNativeBody = (1 << 20) - 1024

func (g *Gateway) handleNative(w http.ResponseWriter, r *http.Request, body []byte) {
	req, err := decodeNative(body)
	if err != nil {
		writeError(w, http.StatusBadRequest, err.Error())
		return
	}
	ctx, cancel := context.WithTimeout(r.Context(), g.generationTimeout)
	defer cancel()
	id := "chatcmpl-" + newRequestID()
	created := time.Now().Unix()
	var content strings.Builder
	started := false
	controller := http.NewResponseController(w)
	defer func() { _ = controller.SetWriteDeadline(time.Time{}) }()
	send := func(value any) error {
		data, e := json.Marshal(value)
		if e != nil {
			return e
		}
		if e = controller.SetWriteDeadline(time.Now().Add(5 * time.Second)); e != nil && !errors.Is(e, http.ErrNotSupported) {
			return e
		}
		if !started {
			w.Header().Set("Content-Type", "text/event-stream")
			w.Header().Set("Cache-Control", "no-cache, no-transform")
			w.Header().Set("X-Accel-Buffering", "no")
			started = true
		}
		if _, e = fmt.Fprintf(w, "data: %s\n\n", data); e != nil {
			return e
		}
		return controller.Flush()
	}
	chunk := func(delta map[string]any, finish any) map[string]any {
		return map[string]any{"id": id, "object": "chat.completion.chunk", "created": created, "model": req.Model, "choices": []any{map[string]any{"index": 0, "delta": delta, "finish_reason": finish}}}
	}
	w.Header().Set("X-OpenMindAI-Gateway", "go-native")
	payload := map[string]any{"model": req.Model, "messages": req.Messages, "timeout_ms": g.generationTimeout.Milliseconds()}
	if req.Temperature != nil {
		payload["temperature"] = *req.Temperature
	}
	if req.TopP != nil {
		payload["top_p"] = *req.TopP
	}
	if req.MaxTokens != nil {
		payload["max_tokens"] = *req.MaxTokens
	}
	err = g.native.Generate(ctx, payload, func(text string) error {
		if !req.Stream {
			if content.Len()+len(text) > 4<<20 {
				return errors.New("response too large")
			}
			content.WriteString(text)
			return nil
		}
		if !started {
			if e := send(chunk(map[string]any{"role": "assistant"}, nil)); e != nil {
				return e
			}
		}
		return send(chunk(map[string]any{"content": text}, nil))
	})
	if err != nil {
		if r.Context().Err() != nil {
			return
		}
		status, message := http.StatusBadGateway, "native inference failed"
		var remote *nativeworker.RemoteError
		if errors.Is(err, context.DeadlineExceeded) {
			status, message = http.StatusGatewayTimeout, "native generation deadline exceeded"
		} else if errors.As(err, &remote) {
			switch remote.Code {
			case "invalid_request", "context_limit":
				status = http.StatusBadRequest
			case "model_not_found":
				status = http.StatusNotFound
			case "resource_limit":
				status = http.StatusServiceUnavailable
			case "timeout":
				status = http.StatusGatewayTimeout
			}
			// Do not expose local filesystem paths from C++ model loading errors.
			if status != http.StatusBadGateway {
				message = remote.Message
			}
		}
		if started {
			_ = send(map[string]any{"error": map[string]string{"message": message, "type": "native_error"}})
		} else {
			writeError(w, status, message)
		}
		return
	}
	if !req.Stream {
		writeJSON(w, http.StatusOK, map[string]any{"id": id, "object": "chat.completion", "created": created, "model": req.Model, "choices": []any{map[string]any{"index": 0, "message": map[string]string{"role": "assistant", "content": content.String()}, "finish_reason": "stop"}}})
		return
	}
	if err = send(chunk(map[string]any{}, "stop")); err != nil {
		return
	}
	if _, err = io.WriteString(w, "data: [DONE]\n\n"); err == nil {
		_ = controller.Flush()
	}
}
