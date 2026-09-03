// Package nativeworker owns one persistent, local Rust inference process.
package nativeworker

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os/exec"
	"sync/atomic"
)

const MaxFrame = 1 << 20

type Event struct {
	ID       uint64 `json:"id"`
	Type     string `json:"type"`
	Protocol int    `json:"protocol"`
	Text     string `json:"text"`
	Code     string `json:"code"`
	Message  string `json:"message"`
}

type RemoteError struct{ Code, Message string }

func (e *RemoteError) Error() string { return e.Message }

type Client struct {
	executable string
	args       []string
	gate       chan struct{}
	cmd        *exec.Cmd
	input      io.WriteCloser
	output     *bufio.Scanner
	ready      atomic.Bool
	sequence   uint64
	closed     bool
}

func New(executable string, args ...string) *Client {
	return &Client{executable: executable, args: args, gate: make(chan struct{}, 1)}
}
func (c *Client) acquire(ctx context.Context) error {
	select {
	case c.gate <- struct{}{}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
func (c *Client) reset() {
	c.ready.Store(false)
	if c.cmd != nil {
		_ = c.cmd.Process.Kill()
		_ = c.input.Close()
		_ = c.cmd.Wait()
	}
	c.cmd, c.input, c.output = nil, nil, nil
}

// withProcess serializes IPC. A cancelled request destroys its process so no
// queued tokens or wedged native call can contaminate the following request.
func (c *Client) withProcess(ctx context.Context, fn func() error) (err error) {
	if err = c.acquire(ctx); err != nil {
		return err
	}
	defer func() { <-c.gate }()
	if err = ctx.Err(); err != nil {
		return err
	}
	if c.closed {
		return errors.New("native worker is closed")
	}
	if c.cmd == nil {
		cmd := exec.Command(c.executable, c.args...)
		cmd.Stderr = io.Discard // llama diagnostics must never enter the JSON stream.
		input, e := cmd.StdinPipe()
		if e != nil {
			return e
		}
		output, e := cmd.StdoutPipe()
		if e != nil {
			_ = input.Close()
			return e
		}
		if e = cmd.Start(); e != nil {
			_ = input.Close()
			_ = output.Close()
			return e
		}
		c.cmd, c.input = cmd, input
		c.output = bufio.NewScanner(output)
		c.output.Buffer(make([]byte, 4096), MaxFrame)
	}
	process := c.cmd.Process
	killed := make(chan struct{})
	stop := context.AfterFunc(ctx, func() { _ = process.Kill(); close(killed) })
	defer func() {
		if !stop() {
			<-killed
		}
		if ctx.Err() != nil {
			err = ctx.Err()
		}
		if err != nil {
			c.reset()
		}
	}()
	if !c.ready.Load() {
		event, e := c.read()
		if e != nil {
			return e
		}
		if event.Type != "ready" || event.Protocol != 1 {
			return errors.New("unsupported native worker protocol")
		}
		c.ready.Store(true)
	}
	return fn()
}
func (c *Client) read() (Event, error) {
	var event Event
	if !c.output.Scan() {
		if err := c.output.Err(); err != nil {
			return event, err
		}
		return event, io.ErrUnexpectedEOF
	}
	if err := json.Unmarshal(c.output.Bytes(), &event); err != nil {
		return event, fmt.Errorf("invalid native frame: %w", err)
	}
	return event, nil
}
func (c *Client) Ready(ctx context.Context) error {
	if c.ready.Load() {
		return nil
	}
	return c.withProcess(ctx, func() error { return nil })
}
func (c *Client) Generate(ctx context.Context, request map[string]any, token func(string) error) error {
	return c.withProcess(ctx, func() error {
		c.sequence++
		request["id"] = c.sequence
		frame, err := json.Marshal(request)
		if err != nil {
			return err
		}
		if len(frame)+1 > MaxFrame {
			return &RemoteError{"invalid_request", "native request exceeds frame limit"}
		}
		if _, err = c.input.Write(append(frame, '\n')); err != nil {
			return err
		}
		total := 0
		for {
			event, err := c.read()
			if err != nil {
				return err
			}
			if event.ID != c.sequence {
				return errors.New("native response ID mismatch")
			}
			switch event.Type {
			case "token":
				total += len(event.Text)
				if total > 4<<20 {
					return errors.New("native response exceeds output limit")
				}
				if err = token(event.Text); err != nil {
					return err
				}
			case "done":
				return nil
			case "error":
				return &RemoteError{event.Code, event.Message}
			default:
				return errors.New("unexpected native response event")
			}
		}
	})
}
func (c *Client) Close() {
	c.gate <- struct{}{}
	defer func() { <-c.gate }()
	c.closed = true
	c.reset()
}
