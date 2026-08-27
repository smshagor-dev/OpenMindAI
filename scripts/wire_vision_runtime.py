from pathlib import Path

runtime = Path("src-tauri/src/runtime.rs")
text = runtime.read_text()
old = '''            config.parallelism.to_string(),
        ];
        if config.gpu_layers > 0 {'''
new = '''            config.parallelism.to_string(),
        ];
        if let Some(mmproj_path) = discover_mmproj_sibling(&model_path) {
            args.push("--mmproj".to_string());
            args.push(mmproj_path.display().to_string());
        }
        if config.gpu_layers > 0 {'''
if old not in text:
    raise SystemExit("runtime args anchor not found")
text = text.replace(old, new, 1)

anchor = '''fn resolve_model_path(root: &PortableRootManager, path: &str) -> Result<PathBuf, AppError> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        Ok(candidate.to_path_buf())
    } else {
        root.resolve_relative(path)
    }
}
'''
helper = anchor + '''
fn discover_mmproj_sibling(model_path: &Path) -> Option<PathBuf> {
    let parent = model_path.parent()?;
    let mut candidates = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().starts_with("mmproj-"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}
'''
if anchor not in text:
    raise SystemExit("runtime helper anchor not found")
runtime.write_text(text.replace(anchor, helper, 1))

registry = Path("src-tauri/src/model_registry.rs")
text = registry.read_text()
old = '''        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        {
            found.push(path);
        }'''
new = '''        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
            && !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().starts_with("mmproj-"))
        {
            found.push(path);
        }'''
if old not in text:
    raise SystemExit("model registry anchor not found")
registry.write_text(text.replace(old, new, 1))
