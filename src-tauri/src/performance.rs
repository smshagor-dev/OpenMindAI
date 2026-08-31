use serde::Serialize;

use crate::hardware::{BackendKind, HardwareProfile};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub enum PerformanceMode {
    Auto,
    Eco,
    Balanced,
    Performance,
    Maximum,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceProfile {
    pub mode: PerformanceMode,
    pub recommended_backend: BackendKind,
    pub cpu_threads: usize,
    pub system_memory_budget_bytes: u64,
    pub vram_budget_bytes: Option<u64>,
    pub mmap: bool,
    pub flash_attention: bool,
}

pub struct PerformanceProfileManager;

impl PerformanceProfileManager {
    pub fn auto(hardware: &HardwareProfile) -> PerformanceProfile {
        let total_memory = hardware.memory.total_bytes;
        let reserve = reserve_for_system(total_memory);
        let system_memory_budget_bytes = total_memory.saturating_sub(reserve);
        let recommended_gpu = hardware
            .recommended_inference_gpu
            .as_ref()
            .and_then(|id| hardware.gpus.iter().find(|gpu| &gpu.id == id));
        let recommended_backend = recommended_gpu
            .map(|gpu| gpu.recommended_backend.clone())
            .unwrap_or(BackendKind::Cpu);
        let vram_budget_bytes = recommended_gpu
            .and_then(|gpu| gpu.dedicated_vram_bytes)
            .map(reserve_for_vram);

        let cpu_threads = hardware
            .cpu
            .physical_cores
            .unwrap_or_else(|| hardware.cpu.logical_threads.saturating_sub(2).max(1))
            .min(hardware.cpu.logical_threads.max(1))
            .max(1);
        let flash_attention = matches!(
            &recommended_backend,
            BackendKind::Cuda | BackendKind::Hip | BackendKind::Sycl | BackendKind::Metal
        );

        PerformanceProfile {
            mode: PerformanceMode::Auto,
            recommended_backend,
            cpu_threads,
            system_memory_budget_bytes,
            vram_budget_bytes,
            mmap: true,
            flash_attention,
        }
    }
}

fn reserve_for_system(total_memory: u64) -> u64 {
    let four_gib = 4 * 1024 * 1024 * 1024;
    let quarter = total_memory / 4;
    four_gib.max(quarter)
}

fn reserve_for_vram(total_vram: u64) -> u64 {
    let one_gib = 1024 * 1024 * 1024;
    total_vram.saturating_sub(one_gib.max(total_vram / 8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendProfile, CpuProfile, GpuInfo, GpuVendor, MemoryProfile};

    #[test]
    fn auto_keeps_memory_reserve() {
        let hardware = HardwareProfile {
            operating_system: "test".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "cpu".to_string(),
                physical_cores: Some(6),
                logical_threads: 12,
            },
            memory: MemoryProfile {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 8 * 1024 * 1024 * 1024,
            },
            gpus: vec![GpuInfo {
                id: "gpu0".to_string(),
                name: "gpu".to_string(),
                vendor: GpuVendor::Nvidia,
                vendor_id: Some(0x10DE),
                device_id: None,
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(8 * 1024 * 1024 * 1024),
                dedicated_system_memory_bytes: Some(0),
                shared_memory_bytes: Some(8 * 1024 * 1024 * 1024),
                luid: None,
                is_discrete: true,
                is_integrated: false,
                is_software: false,
                available_backends: vec![BackendKind::Cpu, BackendKind::Cuda],
                recommended_backend: BackendKind::Cuda,
            }],
            primary_gpu: Some("gpu0".to_string()),
            recommended_inference_gpu: Some("gpu0".to_string()),
            backends: BackendProfile {
                cpu: true,
                cuda: false,
                vulkan: true,
                sycl: false,
                hip: false,
                metal: false,
            },
        };

        let profile = PerformanceProfileManager::auto(&hardware);
        assert_eq!(profile.recommended_backend, BackendKind::Cuda);
        assert!(profile.system_memory_budget_bytes < hardware.memory.total_bytes);
        assert!(profile.vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
        assert_eq!(profile.cpu_threads, 6);
        assert!(profile.flash_attention);
    }
}
