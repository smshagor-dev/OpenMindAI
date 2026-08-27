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

# GPU classification helpers are production code only on Windows, but remain
# compiled under tests on other platforms so the vendor/backend unit tests keep
# exercising the same logic.
for signature in [
    "pub fn classify_vendor(vendor_id: Option<u32>, is_software: bool) -> GpuVendor {",
    "pub fn classify_backends(vendor: &GpuVendor, vulkan_available: bool) -> Vec<BackendKind> {",
    "fn recommended_backend(vendor: &GpuVendor, has_vulkan: bool) -> BackendKind {",
]:
    replace_once(
        "src-tauri/src/hardware.rs",
        signature,
        '#[cfg(any(target_os = "windows", test))]\n' + signature,
    )

# Prefer an explicit branch over the clippy-obfuscated bool Option chain.
for path in ["src-tauri/src/model_catalog.rs", "src-tauri/src/model_package.rs"]:
    replace_once(
        path,
        '''    pattern
        .ends_with('*')
        .then_some(true)
        .unwrap_or_else(|| parts.last().is_none_or(|last| value.ends_with(last)))''',
        '''    if pattern.ends_with('*') {
        true
    } else {
        parts.last().is_none_or(|last| value.ends_with(last))
    }''',
    )

# Collapse project-file persistence and the Tauri command wire contract into a
# typed request object instead of carrying seven independent file fields.
replace_once(
    "src-tauri/src/projects.rs",
    "use serde::Serialize;",
    "use serde::{Deserialize, Serialize};",
)
replace_once(
    "src-tauri/src/projects.rs",
    '''pub struct ProjectFile {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub content_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub added_at: String,
}

pub struct ProjectRepository<'a> {''',
    '''pub struct ProjectFile {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub content_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub added_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFileInput {
    pub project_id: String,
    pub name: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub content_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

pub struct ProjectRepository<'a> {''',
)
replace_once(
    "src-tauri/src/projects.rs",
    '''    pub fn add_file(
        &self,
        project_id: &str,
        name: &str,
        size_bytes: i64,
        mime_type: Option<&str>,
        content_text: Option<&str>,
        status: &str,
        error: Option<&str>,
    ) -> Result<ProjectFile, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO project_files (id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                project_id,
                name.trim(),
                size_bytes,
                mime_type,
                content_text,
                sanitize_file_status(status),
                error,
                now
            ],
        )?;
        self.touch_project(project_id)?;
        self.find_file(&id)
    }''',
    '''    pub fn add_file(&self, input: &ProjectFileInput) -> Result<ProjectFile, AppError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.database.connection().execute(
            "INSERT INTO project_files (id, project_id, name, size_bytes, mime_type, content_text, status, error, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                input.project_id,
                input.name.trim(),
                input.size_bytes,
                input.mime_type.as_deref(),
                input.content_text.as_deref(),
                sanitize_file_status(&input.status),
                input.error.as_deref(),
                now
            ],
        )?;
        self.touch_project(&input.project_id)?;
        self.find_file(&id)
    }''',
)
replace_once(
    "src-tauri/src/projects.rs",
    '''        repo.add_file(
            &project.id,
            "brief.md",
            42,
            Some("text/markdown"),
            Some("# Brief\\nShip the polished version."),
            "ready",
            None,
        )
        .unwrap();''',
    '''        repo.add_file(&ProjectFileInput {
            project_id: project.id.clone(),
            name: "brief.md".to_string(),
            size_bytes: 42,
            mime_type: Some("text/markdown".to_string()),
            content_text: Some("# Brief\\nShip the polished version.".to_string()),
            status: "ready".to_string(),
            error: None,
        })
        .unwrap();''',
)

replace_once(
    "src-tauri/src/lib.rs",
    "use projects::{project_context_message, Project, ProjectFile, ProjectRepository};",
    "use projects::{project_context_message, Project, ProjectFile, ProjectFileInput, ProjectRepository};",
)
replace_once(
    "src-tauri/src/lib.rs",
    '''fn add_project_file(
    project_id: String,
    name: String,
    size_bytes: i64,
    mime_type: Option<String>,
    content_text: Option<String>,
    status: String,
    error: Option<String>,
    state: State<AppState>,
) -> Result<ProjectFile, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).add_file(
        &project_id,
        &name,
        size_bytes,
        mime_type.as_deref(),
        content_text.as_deref(),
        &status,
        error.as_deref(),
    )
}''',
    '''fn add_project_file(
    input: ProjectFileInput,
    state: State<AppState>,
) -> Result<ProjectFile, app_error::AppError> {
    let db = state
        .database
        .lock()
        .map_err(|_| app_error::AppError::internal("database lock poisoned"))?;
    ProjectRepository::new(&db).add_file(&input)
}''',
)

replace_once(
    "src/api.ts",
    '''    call<ProjectFile>("add_project_file", {
      projectId,
      name,
      sizeBytes,
      mimeType,
      contentText,
      status,
      error,
    }),''',
    '''    call<ProjectFile>("add_project_file", {
      input: {
        projectId,
        name,
        sizeBytes,
        mimeType,
        contentText,
        status,
        error,
      },
    }),''',
)
replace_once(
    "src/api.ts",
    '''  if (command === "add_project_file") {
    return Promise.resolve({
      id: crypto.randomUUID(),
      projectId: (args?.projectId as string) || "",
      name: (args?.name as string) || "file",
      sizeBytes: (args?.sizeBytes as number) || 0,
      mimeType: (args?.mimeType as string) || null,
      contentText: (args?.contentText as string) || null,
      status: (args?.status as ProjectFile["status"]) || "tracked",
      error: (args?.error as string) || null,
      addedAt: now,
    } as T);
  }''',
    '''  if (command === "add_project_file") {
    const input = (args?.input as Record<string, unknown> | undefined) ?? {};
    return Promise.resolve({
      id: crypto.randomUUID(),
      projectId: (input.projectId as string) || "",
      name: (input.name as string) || "file",
      sizeBytes: (input.sizeBytes as number) || 0,
      mimeType: (input.mimeType as string) || null,
      contentText: (input.contentText as string) || null,
      status: (input.status as ProjectFile["status"]) || "tracked",
      error: (input.error as string) || null,
      addedAt: now,
    } as T);
  }''',
)
