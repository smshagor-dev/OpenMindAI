use serde::{Deserialize, Serialize};
use sysinfo::System;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_ERROR_NOT_FOUND,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub operating_system: String,
    pub architecture: String,
    pub cpu: CpuProfile,
    pub memory: MemoryProfile,
    pub gpus: Vec<GpuInfo>,
    pub primary_gpu: Option<String>,
    pub recommended_inference_gpu: Option<String>,
    pub backends: BackendProfile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuProfile {
    pub name: String,
    pub physical_cores: Option<usize>,
    pub logical_threads: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProfile {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackendKind {
    Cpu,
    Cuda,
    Vulkan,
    Sycl,
    Hip,
    Metal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    MicrosoftSoftware,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub id: String,
    pub name: String,
    pub vendor: GpuVendor,
    pub vendor_id: Option<u32>,
    pub device_id: Option<u32>,
    pub subsystem_id: Option<u32>,
    pub revision: Option<u32>,
    pub dedicated_vram_bytes: Option<u64>,
    pub dedicated_system_memory_bytes: Option<u64>,
    pub shared_memory_bytes: Option<u64>,
    pub luid: Option<String>,
    pub is_discrete: bool,
    pub is_integrated: bool,
    pub is_software: bool,
    pub available_backends: Vec<BackendKind>,
    pub recommended_backend: BackendKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProfile {
    pub cpu: bool,
    pub cuda: bool,
    pub vulkan: bool,
    pub sycl: bool,
    pub hip: bool,
    pub metal: bool,
}

pub struct HardwareProfiler;

impl HardwareProfiler {
    pub fn detect() -> HardwareProfile {
        let mut system = System::new_all();
        system.refresh_all();
        let cpus = system.cpus();
        let cpu_name = cpus
            .first()
            .map(|cpu| cpu.brand().to_string())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        let gpus = enumerate_gpus();
        let primary_gpu = gpus.first().map(|gpu| gpu.id.clone());
        let recommended_inference_gpu = choose_recommended_gpu(&gpus).map(|gpu| gpu.id.clone());
        // Mirrors the per-GPU vendor classification below (`classify_backends`/
        // `recommended_backend`) -- these are the summary flags the frontend
        // actually reads for its backend-label display (SettingsDialog.tsx),
        // so leaving them hardcoded false understated real AMD/Intel GPUs.
        let has_amd = gpus
            .iter()
            .any(|gpu| gpu.vendor == GpuVendor::Amd && !gpu.is_software);
        let has_intel = gpus
            .iter()
            .any(|gpu| gpu.vendor == GpuVendor::Intel && !gpu.is_software);

        HardwareProfile {
            operating_system: System::long_os_version()
                .unwrap_or_else(|| std::env::consts::OS.to_string()),
            architecture: std::env::consts::ARCH.to_string(),
            cpu: CpuProfile {
                name: cpu_name,
                physical_cores: system.physical_core_count(),
                logical_threads: cpus.len(),
            },
            memory: MemoryProfile {
                total_bytes: system.total_memory(),
                available_bytes: system.available_memory(),
            },
            gpus,
            primary_gpu,
            recommended_inference_gpu,
            backends: BackendProfile {
                cpu: true,
                cuda: command_available("nvidia-smi"),
                vulkan: command_available("vulkaninfo"),
                sycl: has_intel,
                hip: has_amd,
                metal: cfg!(target_os = "macos"),
            },
        }
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn classify_vendor(vendor_id: Option<u32>, is_software: bool) -> GpuVendor {
    if is_software {
        return GpuVendor::MicrosoftSoftware;
    }

    match vendor_id {
        Some(0x10DE) => GpuVendor::Nvidia,
        Some(0x1002) | Some(0x1022) => GpuVendor::Amd,
        Some(0x8086) => GpuVendor::Intel,
        Some(0x1414) => GpuVendor::MicrosoftSoftware,
        _ => GpuVendor::Unknown,
    }
}

#[cfg(any(target_os = "windows", test))]
pub fn classify_backends(vendor: &GpuVendor, vulkan_available: bool) -> Vec<BackendKind> {
    let mut backends = vec![BackendKind::Cpu];
    match vendor {
        GpuVendor::Nvidia => backends.push(BackendKind::Cuda),
        GpuVendor::Amd => backends.push(BackendKind::Hip),
        GpuVendor::Intel => backends.push(BackendKind::Sycl),
        GpuVendor::MicrosoftSoftware | GpuVendor::Unknown => {}
    }
    if vulkan_available && !matches!(vendor, GpuVendor::MicrosoftSoftware) {
        backends.push(BackendKind::Vulkan);
    }
    backends
}

fn choose_recommended_gpu(gpus: &[GpuInfo]) -> Option<&GpuInfo> {
    gpus.iter()
        .filter(|gpu| !gpu.is_software)
        .max_by_key(|gpu| gpu.dedicated_vram_bytes.unwrap_or(0))
}

#[cfg(any(target_os = "windows", test))]
fn recommended_backend(vendor: &GpuVendor, has_vulkan: bool) -> BackendKind {
    match vendor {
        GpuVendor::Nvidia => BackendKind::Cuda,
        GpuVendor::Amd => BackendKind::Hip,
        GpuVendor::Intel => BackendKind::Sycl,
        GpuVendor::Unknown if has_vulkan => BackendKind::Vulkan,
        GpuVendor::MicrosoftSoftware | GpuVendor::Unknown => BackendKind::Cpu,
    }
}

fn command_available(command: &str) -> bool {
    let mut command = std::process::Command::new(command);
    command
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console_window(&mut command);
    command.status().is_ok()
}

fn hide_console_window(_command: &mut std::process::Command) {
    #[cfg(target_os = "windows")]
    {
        _command.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg(target_os = "windows")]
fn enumerate_gpus() -> Vec<GpuInfo> {
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(factory) => factory,
        Err(_) => return Vec::new(),
    };

    let vulkan_available = command_available("vulkaninfo");
    let mut gpus = Vec::new();
    let mut index = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => break,
        };

        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            gpus.push(gpu_from_dxgi_desc(index, desc, vulkan_available));
        }
        index += 1;
    }
    gpus
}

#[cfg(target_os = "windows")]
fn gpu_from_dxgi_desc(index: u32, desc: DXGI_ADAPTER_DESC1, vulkan_available: bool) -> GpuInfo {
    let name = utf16_description(&desc.Description);
    let is_software = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0;
    let vendor = classify_vendor(Some(desc.VendorId), is_software);
    let available_backends = classify_backends(&vendor, vulkan_available);
    let recommended_backend = recommended_backend(&vendor, vulkan_available);
    let dedicated_vram_bytes = u64::try_from(desc.DedicatedVideoMemory).ok();

    GpuInfo {
        id: format!("{:04x}:{:04x}:{}", desc.VendorId, desc.DeviceId, index),
        name,
        vendor,
        vendor_id: Some(desc.VendorId),
        device_id: Some(desc.DeviceId),
        subsystem_id: Some(desc.SubSysId),
        revision: Some(desc.Revision),
        dedicated_vram_bytes,
        dedicated_system_memory_bytes: u64::try_from(desc.DedicatedSystemMemory).ok(),
        shared_memory_bytes: u64::try_from(desc.SharedSystemMemory).ok(),
        luid: Some(format!(
            "{}:{}",
            desc.AdapterLuid.HighPart, desc.AdapterLuid.LowPart
        )),
        is_discrete: dedicated_vram_bytes.unwrap_or(0) > 0 && !is_software,
        is_integrated: dedicated_vram_bytes.unwrap_or(0) == 0 && !is_software,
        is_software,
        available_backends,
        recommended_backend,
    }
}

#[cfg(target_os = "windows")]
fn utf16_description(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|item| *item == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end]).trim().to_string()
}

#[cfg(not(target_os = "windows"))]
fn enumerate_gpus() -> Vec<GpuInfo> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_schema_with_cpu_fallback() {
        let profile = HardwareProfiler::detect();
        assert!(profile.backends.cpu);
        assert!(!profile.architecture.is_empty());
    }

    #[test]
    fn classifies_known_gpu_vendors_by_id() {
        assert_eq!(classify_vendor(Some(0x10DE), false), GpuVendor::Nvidia);
        assert_eq!(classify_vendor(Some(0x1002), false), GpuVendor::Amd);
        assert_eq!(classify_vendor(Some(0x8086), false), GpuVendor::Intel);
        assert_eq!(
            classify_vendor(Some(0x1414), true),
            GpuVendor::MicrosoftSoftware
        );
        assert_eq!(classify_vendor(Some(0xFFFF), false), GpuVendor::Unknown);
    }

    #[test]
    fn handles_software_adapter_without_gpu_backends() {
        let vendor = classify_vendor(Some(0x1414), true);
        let backends = classify_backends(&vendor, true);
        assert_eq!(backends, vec![BackendKind::Cpu]);
        assert_eq!(recommended_backend(&vendor, true), BackendKind::Cpu);
    }

    #[test]
    fn represents_multiple_adapters() {
        let gpus = vec![
            GpuInfo {
                id: "intel".to_string(),
                name: "Intel Integrated".to_string(),
                vendor: GpuVendor::Intel,
                vendor_id: Some(0x8086),
                device_id: Some(1),
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(0),
                dedicated_system_memory_bytes: Some(0),
                shared_memory_bytes: Some(1024),
                luid: None,
                is_discrete: false,
                is_integrated: true,
                is_software: false,
                available_backends: vec![BackendKind::Cpu, BackendKind::Sycl],
                recommended_backend: BackendKind::Sycl,
            },
            GpuInfo {
                id: "nvidia".to_string(),
                name: "NVIDIA Dedicated".to_string(),
                vendor: GpuVendor::Nvidia,
                vendor_id: Some(0x10DE),
                device_id: Some(2),
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(8 * 1024 * 1024 * 1024),
                dedicated_system_memory_bytes: Some(0),
                shared_memory_bytes: Some(1024),
                luid: None,
                is_discrete: true,
                is_integrated: false,
                is_software: false,
                available_backends: vec![BackendKind::Cpu, BackendKind::Cuda],
                recommended_backend: BackendKind::Cuda,
            },
        ];

        assert_eq!(choose_recommended_gpu(&gpus).unwrap().id, "nvidia");
    }

    #[test]
    fn prints_detected_hardware_for_milestone_report() {
        let profile = HardwareProfiler::detect();
        println!("{}", serde_json::to_string_pretty(&profile).unwrap());
        assert!(profile.backends.cpu);
    }
}
