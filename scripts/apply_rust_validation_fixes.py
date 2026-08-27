from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:100]!r}")
    file.write_text(text.replace(old, new, 1))


replace_once(
    "src-tauri/src/document_generator.rs",
    '''    let mut warnings = Vec::new();
    let mut doc = PdfDocument::new(title);

    let regular = ParsedFont::from_bytes(REGULAR_FONT, 0, &mut warnings).ok_or_else(|| {''',
    '''    let mut font_warnings = Vec::new();
    let mut doc = PdfDocument::new(title);

    let regular = ParsedFont::from_bytes(REGULAR_FONT, 0, &mut font_warnings).ok_or_else(|| {''',
)
replace_once(
    "src-tauri/src/document_generator.rs",
    '''    let bold = ParsedFont::from_bytes(BOLD_FONT, 0, &mut warnings).ok_or_else(|| {''',
    '''    let bold = ParsedFont::from_bytes(BOLD_FONT, 0, &mut font_warnings).ok_or_else(|| {''',
)
replace_once(
    "src-tauri/src/document_generator.rs",
    '''    let mono = ParsedFont::from_bytes(MONO_REGULAR_FONT, 0, &mut warnings).ok_or_else(|| {''',
    '''    let mono = ParsedFont::from_bytes(MONO_REGULAR_FONT, 0, &mut font_warnings).ok_or_else(|| {''',
)
replace_once(
    "src-tauri/src/document_generator.rs",
    '''    doc.with_pages(pages);
    let bytes = doc.save(
        &PdfSaveOptions {''',
    '''    doc.with_pages(pages);
    let mut save_warnings = Vec::new();
    let bytes = doc.save(
        &PdfSaveOptions {''',
)
replace_once(
    "src-tauri/src/document_generator.rs",
    '''        &mut warnings,
    );''',
    '''        &mut save_warnings,
    );''',
)
replace_once(
    "src-tauri/src/document_generator.rs",
    '''    if !warnings.is_empty() {
        tracing::debug!(
            warning_count = warnings.len(),
            "PDF generated with warnings"
        );
    }''',
    '''    if !font_warnings.is_empty() || !save_warnings.is_empty() {
        tracing::debug!(
            font_warning_count = font_warnings.len(),
            save_warning_count = save_warnings.len(),
            "PDF generated with warnings"
        );
    }''',
)

for path, command_type in [
    ("src-tauri/src/hardware.rs", "std::process::Command"),
    ("src-tauri/src/runtime.rs", "Command"),
    ("src-tauri/src/lib.rs", "std::process::Command"),
]:
    replace_once(
        path,
        f'''fn hide_console_window(command: &mut {command_type}) {{\n    #[cfg(target_os = "windows")]\n    {{\n        command.creation_flags(CREATE_NO_WINDOW);\n    }}\n}}''',
        f'''fn hide_console_window(_command: &mut {command_type}) {{\n    #[cfg(target_os = "windows")]\n    {{\n        _command.creation_flags(CREATE_NO_WINDOW);\n    }}\n}}''',
    )
