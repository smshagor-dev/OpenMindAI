from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_once(
    "src-tauri/src/lib.rs",
    '''        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                app_error::AppError::internal("no preceding user message to regenerate from")
            })?;

        repo.delete_message(&assistant_message_id)?;
        user
''',
    '''        let user = history[..target_index]
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .cloned()
            .ok_or_else(|| {
                app_error::AppError::internal("no preceding user message to regenerate from")
            })?;
        if user.content.contains("[Mode: Image/Vision Review]") {
            return Err(app_error::AppError::InferenceFailed(
                "Vision responses cannot be regenerated without the original ephemeral image. Reattach the image and send it again."
                    .to_string(),
            ));
        }

        repo.delete_message(&assistant_message_id)?;
        user
''',
)

replace_once(
    "src-tauri/src/lib.rs",
    '''    if mode.eq_ignore_ascii_case("vision") {
        return Err(app_error::AppError::InferenceFailed(
            "Vision responses cannot be regenerated without the original ephemeral image. Reattach the image and send it again."
                .to_string(),
        ));
    }
    run_streaming_completion(
''',
    '''    run_streaming_completion(
''',
)

print("vision regeneration guard fixed")
