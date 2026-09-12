from pathlib import Path


def replace_exact(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"repair anchor missing in {path}: {old[:100]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_exact(
    "src-tauri/src/coding_control.rs",
    "INSERT INTO conversations (id, title, mode, created_at, updated_at) VALUES (?1, 'test', 'chat', ?2, ?2)",
    "INSERT INTO conversations (id, title, created_at, updated_at) VALUES (?1, 'test', ?2, ?2)",
)
replace_exact(
    "src-tauri/src/coding_control.rs",
    "INSERT INTO model_registry (id, name, path, format, quantization, capabilities_json, min_ram_bytes, min_vram_bytes, enabled, created_at, updated_at)\n             VALUES (?1, 'test', 'test.gguf', 'gguf', 'Q4', '[]', 0, NULL, 1, ?2, ?2)",
    "INSERT INTO model_registry (id, name, path, format, quantization, enabled, created_at, updated_at)\n             VALUES (?1, 'test', 'test.gguf', 'gguf', 'Q4', 1, ?2, ?2)",
)
replace_exact(
    "src-tauri/src/coding_control.rs",
    "VALUES (?1, ?2, 'assistant', '', 'complete', ?3, ?4, ?4)",
    "VALUES (?1, ?2, 'assistant', '', 'completed', ?3, ?4, ?4)",
)
replace_exact(
    "src-tauri/src/openagent_parallel.rs",
    "use std::{\n    sync::Arc,\n    time::{Duration, Instant},\n};",
    "use std::{\n    net::IpAddr,\n    sync::Arc,\n    time::{Duration, Instant},\n};",
)
replace_exact(
    "src-tauri/src/openagent_parallel.rs",
    '''    let host = url
        .host_str()
        .ok_or_else(|| AppError::internal("local model endpoint has no host"))?;
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(AppError::internal(
            "parallel sub-agents refuse non-loopback model endpoints",
        ));
    }
''',
    '''    let host = url
        .host_str()
        .ok_or_else(|| AppError::internal("local model endpoint has no host"))?;
    let normalized_host = host.trim_matches(|character| character == '[' || character == ']');
    let is_loopback = normalized_host.eq_ignore_ascii_case("localhost")
        || normalized_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !is_loopback {
        return Err(AppError::internal(
            "parallel sub-agents refuse non-loopback model endpoints",
        ));
    }
''',
)

print("parallel generated-source repairs applied")
