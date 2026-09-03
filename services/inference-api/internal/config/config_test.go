package config

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestNativeConfiguration(t *testing.T) {
	path := filepath.Join(t.TempDir(), "fixture")
	if err := os.WriteFile(path, []byte("fixture"), 0600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("OPENMINDAI_API_BACKEND", "native")
	t.Setenv("OPENMINDAI_NATIVE_WORKER", path)
	t.Setenv("OPENMINDAI_NATIVE_MODELS", path)
	t.Setenv("OPENMINDAI_API_ADDR", "127.0.0.1:11435")
	cfg, err := Load()
	if err != nil {
		t.Fatal(err)
	}
	if cfg.MaxInflight != 1 || cfg.MaxBodyBytes >= 1<<20 || cfg.GenerationTimeout != 120*time.Second {
		t.Fatalf("unsafe native limits: %+v", cfg)
	}
	for _, address := range []string{"0.0.0.0:1234", "example.com:1234", "[::]:1234"} {
		t.Setenv("OPENMINDAI_API_ADDR", address)
		if _, err = Load(); err == nil {
			t.Fatal("accepted public native listener")
		}
	}
	t.Setenv("OPENMINDAI_API_ADDR", "127.0.0.1:11435")
	t.Setenv("OPENMINDAI_API_GENERATION_TIMEOUT", "2h")
	if _, err = Load(); err == nil {
		t.Fatal("accepted excessive deadline")
	}
}
