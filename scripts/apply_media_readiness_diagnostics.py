from pathlib import Path

preflight_path = Path("src-tauri/src/media_preflight.rs")
preflight = preflight_path.read_text(encoding="utf-8")

old_impl = '''impl MediaPreflightReport {
    pub(crate) fn ensure_ready(&self) -> Result<(), AppError> {
        let blockers = self
            .checks
            .iter()
            .filter(|check| check.status == PreflightStatus::Error)
            .map(|check| format!("{}: {}", check.label, check.detail))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            return Ok(());
        }
        Err(AppError::ArtifactGenerationFailed(format!(
            "{} is not ready for local generation. {}",
            self.model_name,
            blockers.join("; ")
        )))
    }
}
'''
new_impl = '''impl MediaPreflightReport {
    pub(crate) fn ensure_ready(&self) -> Result<(), AppError> {
        let blockers = self
            .checks
            .iter()
            .filter(|check| check.status == PreflightStatus::Error)
            .map(|check| format!("{}: {}", check.label, check.detail))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            return Ok(());
        }
        Err(AppError::ArtifactGenerationFailed(format!(
            "{} is not ready for local generation. {}",
            self.model_name,
            blockers.join("; ")
        )))
    }

    pub(crate) fn issue_summary(&self) -> Option<String> {
        let issues = self
            .checks
            .iter()
            .filter(|check| check.status != PreflightStatus::Ok)
            .map(|check| format!("{}: {}", check.label, check.detail))
            .collect::<Vec<_>>();
        (!issues.is_empty()).then(|| issues.join("; "))
    }
}
'''
if old_impl not in preflight:
    raise SystemExit("media preflight report impl anchor not found")
preflight = preflight.replace(old_impl, new_impl, 1)

old_preflight = '''pub(crate) fn preflight(
    root: &PortableRootManager,
    artifact_kind: &str,
) -> Result<Option<MediaPreflightReport>, AppError> {
    let hardware = HardwareProfiler::detect();
    preflight_with_environment(
        root,
        artifact_kind,
        &hardware,
        available_bytes_for_path(root.root()),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}
'''
new_preflight = '''pub(crate) fn preflight(
    root: &PortableRootManager,
    artifact_kind: &str,
) -> Result<Option<MediaPreflightReport>, AppError> {
    let hardware = HardwareProfiler::detect();
    preflight_for_hardware(root, artifact_kind, &hardware)
}

pub(crate) fn preflight_for_hardware(
    root: &PortableRootManager,
    artifact_kind: &str,
    hardware: &HardwareProfile,
) -> Result<Option<MediaPreflightReport>, AppError> {
    preflight_with_environment(
        root,
        artifact_kind,
        hardware,
        available_bytes_for_path(root.root()),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}
'''
if old_preflight not in preflight:
    raise SystemExit("media preflight function anchor not found")
preflight = preflight.replace(old_preflight, new_preflight, 1)
preflight_path.write_text(preflight, encoding="utf-8")

maintenance_path = Path("src-tauri/src/maintenance.rs")
maintenance = maintenance_path.read_text(encoding="utf-8")

old_import = '''use crate::{
    app_error::AppError,
    database::Database,
'''
new_import = '''use crate::{
    app_error::AppError,
    artifacts::media_preflight,
    database::Database,
'''
if old_import not in maintenance:
    raise SystemExit("maintenance import anchor not found")
maintenance = maintenance.replace(old_import, new_import, 1)

media_checks = '''    for (id, label, kind) in [
        ("mediaCanvas", "OpenMindAI Canvas", "image"),
        ("mediaMotion", "OpenMindAI Motion", "video"),
        ("mediaSpeak", "OpenMindAI Speak", "audio"),
    ] {
        let report = media_preflight::preflight_for_hardware(root, kind, hardware)?
            .ok_or_else(|| AppError::internal(format!("no media preflight configured for {kind}")))?;
        let detail = report.issue_summary();
        let status = if detail.is_some() {
            CheckStatus::Warning
        } else {
            CheckStatus::Ok
        };
        checks.push(DiagnosticCheck::new(id, label, status, detail));
    }

'''
anchor = '''    checks.push(match available_bytes_for_path(root.root()) {
'''
if anchor not in maintenance:
    raise SystemExit("maintenance disk-space anchor not found")
maintenance = maintenance.replace(anchor, media_checks + anchor, 1)

test_anchor = '''        assert_eq!(by_id("runtime").status, CheckStatus::Warning);
        assert_eq!(by_id("model").status, CheckStatus::Warning);
'''
test_replacement = '''        assert_eq!(by_id("runtime").status, CheckStatus::Warning);
        assert_eq!(by_id("model").status, CheckStatus::Warning);
        assert_eq!(by_id("mediaCanvas").status, CheckStatus::Warning);
        assert_eq!(by_id("mediaMotion").status, CheckStatus::Warning);
        assert_eq!(by_id("mediaSpeak").status, CheckStatus::Warning);
'''
if test_anchor not in maintenance:
    raise SystemExit("maintenance diagnostics test anchor not found")
maintenance = maintenance.replace(test_anchor, test_replacement, 1)
maintenance_path.write_text(maintenance, encoding="utf-8")

Path("scripts/apply_media_readiness_diagnostics.py").unlink(missing_ok=True)
