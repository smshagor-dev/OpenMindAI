use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use sysinfo::System;

#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE,
    DXGI_ERROR_NOT_FOUND,
};

static DETECTED_HARDWARE: OnceLock<RwLock<Option<HardwareProfile>>> = OnceLock::new();
#[cfg(not(test))]
static BACKGROUND_SCAN_STARTED: OnceLock<()> = OnceLock::new();

#[derive(Debug, Serialize)]
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

impl HardwareProfile {
    fn clone_fields(&self) -> Self {
        Self {
            operating_system: self.operating_system.clone(),
            architecture: self.architecture.clone(),
            cpu: self.cpu.clone(),
            memory: self.memory.clone(),
            gpus: self.gpus.clone(),
            primary_gpu: self.primary_gpu.clone(),
            recommended_inference_gpu: self.recommended_inference_gpu.clone(),
            backends: self.backends.clone(),
        }
    }

    fn latest_or_self(&self) -> Self {
        if let Some(profile) = latest_detected_hardware() {
            profile
        } else {
            self.clone_fields()
        }
    }
}

impl Clone for HardwareProfile {
    fn clone(&self) -> Self {
        self.latest_or_self()
    }
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
    /// Returns immediately with a minimal startup snapshot and starts the real
    /// CPU/RAM/GPU profile on a background thread. AppState can safely keep the
    /// startup value because cloning HardwareProfile automatically picks up the
    /// completed background profile when it becomes available.
    pub fn detect() -> HardwareProfile {
        #[cfg(test)]
        {
            detect_full()
        }

        #[cfg(not(test))]
        {
            if let Some(profile) = latest_detected_hardware() {
                return profile;
            }

            if BACKGROUND_SCAN_STARTED.set(()).is_ok() {
                std::thread::spawn(|| {
                    let profile = detect_full();
                    store_detected_hardware(profile);
                });
            }

            startup_snapshot()
        }
    }

    /// Inference must never launch from the lightweight startup placeholder.
    /// If the background scan has not completed yet, finish one real profile
    /// synchronously here. This cost is paid only when a prompt races startup,
    /// and prevents an entire session from being pinned to CPU because the
    /// first model was launched before DXGI/GPU discovery completed.
    pub fn for_inference(fallback: &HardwareProfile) -> HardwareProfile {
        #[cfg(test)]
        {
            return fallback.clone_fields();
        }

        #[cfg(not(test))]
        {
            if let Some(profile) = latest_detected_hardware() {
                return profile;
            }

            let profile = detect_full();
            let return_value = profile.clone_fields();
            store_detected_hardware(profile);
            return_value
        }
    }
}

fn store_detected_hardware(profile: HardwareProfile) {
    let cache = DETECTED_HARDWARE.get_or_init(|| RwLock::new(None));
    if let Ok(mut current) = cache.write() {
        *current = Some(profile);
    }
}

fn latest_detected_hardware() -> Option<HardwareProfile> {
    DETECTED_HARDWARE
        .get()
        .and_then(|cache| cache.read().ok())
        .and_then(|profile| profile.as_ref().map(HardwareProfile::clone_fields))
}

#[cfg(not(test))]
fn startup_snapshot() -> HardwareProfile {
    let logical_threads = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);

    HardwareProfile {
        operating_system: std::env::consts::OS.to_string(),
        architecture: std::env::consts::ARCH.to_string(),
        cpu: CpuProfile {
            name: "Detecting hardware".to_string(),
            physical_cores: None,
            logical_threads,
        },
        memory: MemoryProfile {
            total_bytes: 0,
            available_bytes: 0,
        },
        gpus: Vec::new(),
        primary_gpu: None,
        recommended_inference_gpu: None,
        backends: BackendProfile {
            cpu: true,
            cuda: false,
            vulkan: false,
            sycl: false,
            hip: false,
            metal: cfg!(target_os = "macos"),
        },
    }
}

fn detect_full() -> HardwareProfile {
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
    let has_nvidia = gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Nvidia && !gpu.is_software);
    let has_amd = gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Amd && !gpu.is_software);
    let has_intel = gpus
        .iter()
        .any(|gpu| gpu.vendor == GpuVendor::Intel && !gpu.is_software);
    let has_hardware_gpu = gpus.iter().any(|gpu| !gpu.is_software);

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
            cuda: has_nvidia,
            vulkan: cfg!(target_os = "windows") && has_hardware_gpu,
            sycl: has_intel,
            hip: has_amd,
            metal: cfg!(target_os = "macos"),
        },
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
        // The packaged Windows llama.cpp path is Vulkan-first for AMD. HIP
        // remains discoverable as an alternative, but choosing it here when
        // only a Vulkan runtime is installed creates a mismatched launch plan.
        GpuVendor::Amd if has_vulkan => BackendKind::Vulkan,
        GpuVendor::Amd => BackendKind::Hip,
        GpuVendor::Intel => BackendKind::Sycl,
        GpuVendor::Unknown if has_vulkan => BackendKind::Vulkan,
        GpuVendor::MicrosoftSoftware | GpuVendor::Unknown => BackendKind::Cpu,
    }
}

#[cfg(target_os = "windows")]
fn enumerate_gpus() -> Vec<GpuInfo> {
    let factory: IDXGIFactory1 = match unsafe { CreateDXGIFactory1() } {
        Ok(factory) => factory,
        Err(_) => return Vec::new(),
    };

    // External probes are intentionally not used. Runtime validation remains
    // the source of truth for the exact backend that can be launched.
    let mut gpus = Vec::new();
    let mut index = 0;
    loop {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => break,
        };

        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            let is_software = (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0;
            gpus.push(gpu_from_dxgi_desc(index, desc, !is_software));
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
    fn windows_amd_prefers_vulkan_launch_plan() {
        assert_eq!(recommended_backend(&GpuVendor::Amd, true), BackendKind::Vulkan);
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
