from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]

runtime = ROOT / "src-tauri/src/diffusion_runtime.rs"
text = runtime.read_text(encoding="utf-8")
old = "fn hide_console_window(command: &mut Command) {\n    #[cfg(target_os = \"windows\")]\n    {\n        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);\n    }\n}"
new = "fn hide_console_window(_command: &mut Command) {\n    #[cfg(target_os = \"windows\")]\n    {\n        _command.as_std_mut().creation_flags(CREATE_NO_WINDOW);\n    }\n}"
if old not in text:
    raise RuntimeError("hide_console_window anchor not found")
runtime.write_text(text.replace(old, new, 1), encoding="utf-8")

lib = ROOT / "src-tauri/src/lib.rs"
text = lib.read_text(encoding="utf-8")
start = text.index("fn generation_family_label(")
end = text.index("#[tauri::command]\nfn list_artifacts(", start)
text = text[:start] + text[end:]
lib.write_text(text, encoding="utf-8")

catalog = ROOT / "src-tauri/model-catalog.json"
data = json.loads(catalog.read_text(encoding="utf-8"))
canvas = next(model for model in data["models"] if model["id"] == "sdxl-base-1")
canvas["filenamePattern"] = "sd_xl_base_1.0.safetensors"
catalog.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
