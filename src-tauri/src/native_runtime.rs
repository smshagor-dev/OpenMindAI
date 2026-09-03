use std::{env, fs, path::PathBuf};

use serde::Deserialize;

use crate::hardware::BackendKind;

pub const LLAMA_CPP_COMMIT: &str = env!("OPENMINDAI_NATIVE_LLAMA_COMMIT");
pub const ABI_TAG: &str = env!("OPENMINDAI_NATIVE_ABI_TAG");

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BackendLoading {
    #[default]
    Linked,
    Dynamic,
}

fn compiled_backend_loading() -> BackendLoading {
    if option_env!("OPENMINDAI_NATIVE_BACKEND_LOADING") == Some("dynamic") {
        BackendLoading::Dynamic
    } else {
        BackendLoading::Linked
    }
}

pub const WINDOWS_REQUIRED_DLLS: &[&str] =
    &["llama.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"];
pub const WINDOWS_VULKAN_DLL: &str = "ggml-vulkan.dll";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRuntimeContract {
    pub abi_tag: &'static str,
    pub llama_cpp_commit: &'static str,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NativeRuntimeBackend {
    Cpu,
    Vulkan,
}

impl NativeRuntimeBackend {
    pub fn supports(self, backend: &BackendKind) -> bool {
        matches!(
            (self, backend),
            (_, BackendKind::Cpu) | (Self::Vulkan, BackendKind::Vulkan)
        )
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledNativeRuntime {
    pub schema_version: u32,
    pub abi_tag: String,
    pub llama_cpp_commit: String,
    pub backend: NativeRuntimeBackend,
    #[serde(default)]
    pub backend_loading: BackendLoading,
}

impl InstalledNativeRuntime {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported native runtime manifest schema {}",
                self.schema_version
            ));
        }
        if self.abi_tag != ABI_TAG {
            return Err(format!(
                "native runtime ABI mismatch: {} != {ABI_TAG}",
                self.abi_tag
            ));
        }
        if !self.llama_cpp_commit.eq_ignore_ascii_case(LLAMA_CPP_COMMIT) {
            return Err(format!(
                "native runtime llama.cpp revision mismatch: {} != {LLAMA_CPP_COMMIT}",
                self.llama_cpp_commit
            ));
        }
        if self.backend_loading != compiled_backend_loading() {
            return Err(
                "native runtime backend loading mode does not match this application".into(),
            );
        }
        Ok(())
    }
}

pub const fn contract() -> NativeRuntimeContract {
    NativeRuntimeContract {
        abi_tag: ABI_TAG,
        llama_cpp_commit: LLAMA_CPP_COMMIT,
    }
}

pub fn detect_installed() -> Result<InstalledNativeRuntime, String> {
    if let Ok(value) = env::var("OPENMINDAI_NATIVE_RUNTIME_BACKEND") {
        let backend = parse_backend(&value)?;
        return Ok(InstalledNativeRuntime {
            schema_version: 1,
            abi_tag: ABI_TAG.to_string(),
            llama_cpp_commit: LLAMA_CPP_COMMIT.to_string(),
            backend,
            backend_loading: compiled_backend_loading(),
        });
    }

    let manifest_path = installed_manifest_path()?;
    let manifest = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "native runtime manifest unavailable at {}: {error}",
            manifest_path.display()
        )
    })?;
    let runtime: InstalledNativeRuntime = serde_json::from_str(&manifest)
        .map_err(|error| format!("invalid native runtime manifest: {error}"))?;
    runtime.validate()?;
    validate_windows_files(runtime.backend, runtime.backend_loading)?;
    Ok(runtime)
}

fn installed_manifest_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        let executable = env::current_exe()
            .map_err(|error| format!("cannot resolve OpenMindAI executable path: {error}"))?;
        let app_dir = executable
            .parent()
            .ok_or_else(|| "OpenMindAI executable has no parent directory".to_string())?;

        Ok(app_dir
            .join("resources")
            .join("native-runtime")
            .join("windows-x86_64")
            .join("native-runtime-manifest.json"))
    }

    #[cfg(not(target_os = "windows"))]
    Err("packaged native runtime discovery is currently implemented for Windows".to_string())
}

fn validate_windows_files(
    backend: NativeRuntimeBackend,
    loading: BackendLoading,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let executable = env::current_exe()
            .map_err(|error| format!("cannot resolve OpenMindAI executable path: {error}"))?;
        let app_dir = executable
            .parent()
            .ok_or_else(|| "OpenMindAI executable has no parent directory".to_string())?;
        for required in WINDOWS_REQUIRED_DLLS {
            let path = app_dir.join(required);
            if !path.is_file() {
                return Err(format!(
                    "native runtime library missing: {}",
                    path.display()
                ));
            }
        }
        if backend == NativeRuntimeBackend::Vulkan && loading == BackendLoading::Linked {
            let path = app_dir.join(WINDOWS_VULKAN_DLL);
            if !path.is_file() {
                return Err(format!(
                    "native Vulkan runtime library missing: {}",
                    path.display()
                ));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    let _ = (backend, loading);

    Ok(())
}

fn parse_backend(value: &str) -> Result<NativeRuntimeBackend, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "cpu" => Ok(NativeRuntimeBackend::Cpu),
        "vulkan" => Ok(NativeRuntimeBackend::Vulkan),
        other => Err(format!("unsupported native runtime backend: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_mode_mismatch_is_rejected() {
        let mut runtime = InstalledNativeRuntime {
            schema_version: 1,
            abi_tag: ABI_TAG.to_string(),
            llama_cpp_commit: LLAMA_CPP_COMMIT.to_string(),
            backend: NativeRuntimeBackend::Vulkan,
            backend_loading: compiled_backend_loading(),
        };
        assert!(runtime.validate().is_ok());
        runtime.backend_loading = match runtime.backend_loading {
            BackendLoading::Linked => BackendLoading::Dynamic,
            BackendLoading::Dynamic => BackendLoading::Linked,
        };
        assert!(runtime.validate().unwrap_err().contains("loading mode"));
    }

    #[test]
    fn old_manifests_default_to_linked_backends() {
        let runtime: InstalledNativeRuntime = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "abiTag": ABI_TAG,
            "llamaCppCommit": LLAMA_CPP_COMMIT,
            "backend": "cpu"
        }))
        .unwrap();
        assert_eq!(runtime.backend_loading, BackendLoading::Linked);
    }

    #[test]
    fn abi_tag_is_derived_from_pinned_commit() {
        assert_eq!(LLAMA_CPP_COMMIT.len(), 40);
        assert_eq!(ABI_TAG, format!("llama-cxx-{}", &LLAMA_CPP_COMMIT[..12]));
    }

    #[test]
    fn windows_bundle_contract_contains_core_libraries() {
        for required in ["llama.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"] {
            assert!(WINDOWS_REQUIRED_DLLS.contains(&required));
        }
    }

    #[test]
    fn cpu_runtime_does_not_claim_gpu_support() {
        assert!(NativeRuntimeBackend::Cpu.supports(&BackendKind::Cpu));
        assert!(!NativeRuntimeBackend::Cpu.supports(&BackendKind::Vulkan));
    }

    #[test]
    fn vulkan_runtime_supports_vulkan_and_cpu_fallback() {
        assert!(NativeRuntimeBackend::Vulkan.supports(&BackendKind::Vulkan));
        assert!(NativeRuntimeBackend::Vulkan.supports(&BackendKind::Cpu));
        assert!(!NativeRuntimeBackend::Vulkan.supports(&BackendKind::Cuda));
    }
}
