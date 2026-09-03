package nativeworker

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"strings"
	"testing"
	"time"
)

func TestWorkerProcess(t *testing.T) {
	if os.Getenv("OPENMIND_TEST_PROCESS") != "1" {
		return
	}
	fmt.Println(`{"type":"ready","protocol":1}`)
	scanner := bufio.NewScanner(os.Stdin)
	for scanner.Scan() {
		var request struct {
			ID    uint64 `json:"id"`
			Model string `json:"model"`
		}
		if json.Unmarshal(scanner.Bytes(), &request) != nil {
			os.Exit(2)
		}
		switch request.Model {
		case "hang":
			time.Sleep(time.Minute)
		case "crash":
			os.Exit(3)
		case "wrong":
			fmt.Printf("{\"id\":%d,\"type\":\"done\"}\n", request.ID+1)
		case "oversize":
			fmt.Println(strings.Repeat("x", MaxFrame+1))
		case "error":
			fmt.Printf("{\"id\":%d,\"type\":\"error\",\"code\":\"model_not_found\",\"message\":\"unknown model\"}\n", request.ID)
		default:
			data, _ := json.Marshal(map[string]any{"id": request.ID, "type": "token", "text": fmt.Sprintf("বাংলা 🌼:%d", os.Getpid())})
			fmt.Println(string(data))
			fmt.Printf("{\"id\":%d,\"type\":\"done\"}\n", request.ID)
		}
	}
	os.Exit(0)
}
func helper(t *testing.T) *Client {
	t.Helper()
	t.Setenv("OPENMIND_TEST_PROCESS", "1")
	path, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	client := New(path, "-test.run=^TestWorkerProcess$")
	t.Cleanup(client.Close)
	return client
}
func call(t *testing.T, c *Client, model string) string {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	text := ""
	err := c.Generate(ctx, map[string]any{"model": model}, func(s string) error { text += s; return nil })
	if err != nil {
		t.Fatal(err)
	}
	return text
}
func TestReuseAndRecovery(t *testing.T) {
	c := helper(t)
	first := call(t, c, "ok")
	if !strings.HasPrefix(first, "বাংলা 🌼:") || first != call(t, c, "ok") {
		t.Fatal("worker not reused or Unicode damaged")
	}
	for _, mode := range []string{"hang", "crash", "wrong", "oversize", "error"} {
		t.Run(mode, func(t *testing.T) {
			timeout := 3 * time.Second
			if mode == "hang" {
				timeout = 100 * time.Millisecond
			}
			ctx, cancel := context.WithTimeout(context.Background(), timeout)
			defer cancel()
			err := c.Generate(ctx, map[string]any{"model": mode}, func(string) error { return nil })
			if err == nil {
				t.Fatal("expected error")
			}
			if mode == "hang" && !errors.Is(err, context.DeadlineExceeded) {
				t.Fatal(err)
			}
			after := call(t, c, "ok")
			if after == first {
				t.Fatal("failed worker was reused")
			}
			first = after
		})
	}
}
func TestConsumerCancellationAndClosedClient(t *testing.T) {
	c := helper(t)
	first := call(t, c, "ok")
	ctx, cancel := context.WithCancel(context.Background())
	err := c.Generate(ctx, map[string]any{"model": "ok"}, func(string) error { cancel(); return ctx.Err() })
	if !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
	if call(t, c, "ok") == first {
		t.Fatal("cancelled worker was reused")
	}
	c.Close()
	if err := c.Ready(context.Background()); err == nil {
		t.Fatal("closed client started")
	}
}
func TestQueuedCancellationDoesNotStealWorker(t *testing.T) {
	c := helper(t)
	first := call(t, c, "ok")
	c.gate <- struct{}{}
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Millisecond)
	defer cancel()
	if err := c.Generate(ctx, map[string]any{"model": "ok"}, func(string) error { return nil }); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatal(err)
	}
	<-c.gate
	if call(t, c, "ok") != first {
		t.Fatal("queued cancellation disrupted worker")
	}
}
