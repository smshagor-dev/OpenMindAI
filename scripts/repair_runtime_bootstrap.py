from pathlib import Path

runtime_path = Path("src-tauri/src/isolated_runtime.rs")
lines = runtime_path.read_text(encoding="utf-8").splitlines(keepends=True)

mac_start = next((i for i, line in enumerate(lines) if "async fn run_macos_sandbox(" in line), -1)
if mac_start < 0:
    raise SystemExit("macOS runtime function not found")

read_workspace = next(
    (i for i in range(mac_start, len(lines)) if "{workspace}" in lines[i] and "))" in lines[i]),
    -1,
)
if read_workspace < 0:
    raise SystemExit("macOS workspace read rule not found")
read_line = lines[read_workspace]
lines[read_workspace] = read_line.replace("))", ")", 1)
lines.insert(read_workspace + 1, read_line.replace("{workspace}", "{scratch}"))

write_marker = next(
    (i for i in range(read_workspace + 2, len(lines)) if "(allow file-write*" in lines[i]),
    -1,
)
if write_marker < 0:
    raise SystemExit("macOS write rule not found")
private_tmp = next(
    (i for i in range(write_marker, len(lines)) if "/private/tmp" in lines[i]),
    -1,
)
plain_tmp = next(
    (i for i in range(write_marker, len(lines)) if '(subpath \\"/tmp\\"))' in lines[i]),
    -1,
)
if private_tmp < 0 or plain_tmp < 0:
    raise SystemExit("macOS temporary write rules not found")
lines[private_tmp] = lines[plain_tmp].replace("/tmp", "{scratch}")
del lines[plain_tmp]
runtime_path.write_text("".join(lines), encoding="utf-8")

bootstrap_path = Path("scripts/bootstrap_runtime_guards.py")
text = bootstrap_path.read_text(encoding="utf-8")
label = '    "mac scratch profile",\n)'
pos = text.find(label)
if pos < 0:
    raise SystemExit("mac scratch profile patch label not found")
start = text.rfind("runtime = replace_once(", 0, pos)
end = text.find("runtime = replace_once(", pos + len(label))
if start < 0 or end < 0:
    raise SystemExit("mac scratch profile bootstrap span not found")
text = text[:start] + text[end:]
bootstrap_path.write_text(text, encoding="utf-8")
