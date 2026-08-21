use serde::Serialize;

use crate::{
    hardware::{BackendKind, HardwareProfile},
    model_registry::ModelRecord,
    performance::{PerformanceProfile, PerformanceProfileManager},
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelLaunchConfig {
    pub model_path: String,
    pub backend: BackendKind,
    pub device: Option<String>,
    pub gpu_layers: i32,
    pub context_size: u32,
    pub threads: usize,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub flash_attention: bool,
    pub mmap: bool,
    pub mlock: bool,
    pub parallelism: u32,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchPlan {
    pub config: ModelLaunchConfig,
    pub estimated_model_bytes: u64,
    pub estimated_context_bytes: u64,
    pub dedicated_vram_budget_bytes: Option<u64>,
    pub notes: Vec<String>,
}

pub struct ModelLaunchPlanner;

impl ModelLaunchPlanner {
    pub fn plan(model: &ModelRecord, hardware: &HardwareProfile, port: u16) -> LaunchPlan {
        let profile = PerformanceProfileManager::auto(hardware);
        Self::plan_with_profile(model, hardware, &profile, port)
    }

    pub fn plan_with_profile(
        model: &ModelRecord,
        hardware: &HardwareProfile,
        profile: &PerformanceProfile,
        port: u16,
    ) -> LaunchPlan {
        let selected_gpu = hardware
            .recommended_inference_gpu
            .as_ref()
            .and_then(|id| hardware.gpus.iter().find(|gpu| &gpu.id == id));
        let backend = selected_gpu
            .map(|gpu| {
                if gpu.recommended_backend != BackendKind::Cpu
                    && gpu.available_backends.contains(&gpu.recommended_backend)
                {
                    gpu.recommended_backend.clone()
                } else if gpu.available_backends.contains(&BackendKind::Vulkan) {
                    BackendKind::Vulkan
                } else {
                    BackendKind::Cpu
                }
            })
            .unwrap_or(BackendKind::Cpu);
        let dedicated_vram_budget_bytes = selected_gpu
            .and_then(|gpu| gpu.dedicated_vram_bytes)
            .map(safe_dedicated_vram_budget);
        let context_size = model.context_length.unwrap_or(8192).clamp(4096, 8192) as u32;
        let estimated_model_bytes = model.size_bytes.max(0) as u64;
        let estimated_context_bytes = estimate_context_bytes(context_size);
        let total_estimate = estimated_model_bytes
            .saturating_add(estimated_context_bytes)
            .saturating_add(768 * 1024 * 1024);
        let gpu_layers = match (backend.clone(), dedicated_vram_budget_bytes) {
            (BackendKind::Cuda, Some(budget)) if budget > total_estimate => 999,
            (BackendKind::Cuda, Some(budget)) if budget > estimated_model_bytes / 2 => 32,
            (BackendKind::Cuda, Some(_)) => 16,
            (BackendKind::Vulkan, Some(budget)) if budget > total_estimate => 999,
            (BackendKind::Vulkan, Some(budget)) if budget > estimated_model_bytes / 2 => 24,
            (BackendKind::Vulkan, Some(_)) => 12,
            _ => 0,
        };

        let mut notes = Vec::new();
        notes.push("Initial context is capped at 8192 tokens for milestone stability.".to_string());
        notes.push(
            "Dedicated VRAM only is used for GPU placement; shared memory is not counted."
                .to_string(),
        );
        if matches!(backend, BackendKind::Vulkan) {
            notes.push(
                "Flash attention disabled until llama.cpp/Vulkan capability probe proves support."
                    .to_string(),
            );
        } else if matches!(backend, BackendKind::Cuda) {
            notes.push("NVIDIA CUDA backend selected for local model launch.".to_string());
        }

        LaunchPlan {
            config: ModelLaunchConfig {
                model_path: model.path.clone(),
                backend,
                device: selected_gpu.map(|gpu| gpu.name.clone()),
                gpu_layers,
                context_size,
                threads: profile.cpu_threads,
                batch_size: 512,
                ubatch_size: 128,
                flash_attention: false,
                mmap: profile.mmap,
                mlock: false,
                parallelism: 1,
                host: "127.0.0.1".to_string(),
                port,
            },
            estimated_model_bytes,
            estimated_context_bytes,
            dedicated_vram_budget_bytes,
            notes,
        }
    }
}

fn safe_dedicated_vram_budget(total: u64) -> u64 {
    let reserve = (1536_u64 * 1024 * 1024).max(total / 5);
    total.saturating_sub(reserve)
}

fn estimate_context_bytes(context_size: u32) -> u64 {
    u64::from(context_size) * 1024 * 1024 / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{BackendProfile, CpuProfile, GpuInfo, GpuVendor, MemoryProfile};

    #[test]
    fn uses_dedicated_vram_only_for_gpu_plan() {
        let hardware = hardware_with_vram(8 * 1024 * 1024 * 1024, 32 * 1024 * 1024 * 1024);
        let model = model(2_497_280_256);
        let plan = ModelLaunchPlanner::plan(&model, &hardware, 8080);
        assert_eq!(plan.config.backend, BackendKind::Vulkan);
        assert!(plan.dedicated_vram_budget_bytes.unwrap() < 8 * 1024 * 1024 * 1024);
        assert_ne!(
            plan.dedicated_vram_budget_bytes.unwrap(),
            40 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn falls_back_to_cpu_without_gpu() {
        let mut hardware = hardware_with_vram(0, 0);
        hardware.gpus.clear();
        hardware.recommended_inference_gpu = None;
        let plan = ModelLaunchPlanner::plan(&model(2_497_280_256), &hardware, 8080);
        assert_eq!(plan.config.backend, BackendKind::Cpu);
        assert_eq!(plan.config.gpu_layers, 0);
    }

    #[test]
    fn prefers_cuda_for_nvidia_gpu() {
        let mut hardware = hardware_with_vram(8 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024);
        hardware.gpus[0].name = "NVIDIA GeForce RTX".to_string();
        hardware.gpus[0].vendor = GpuVendor::Nvidia;
        hardware.gpus[0].vendor_id = Some(0x10DE);
        hardware.gpus[0].available_backends =
            vec![BackendKind::Cpu, BackendKind::Cuda, BackendKind::Vulkan];
        hardware.gpus[0].recommended_backend = BackendKind::Cuda;
        hardware.backends.cuda = true;

        let plan = ModelLaunchPlanner::plan(&model(2_497_280_256), &hardware, 8080);

        assert_eq!(plan.config.backend, BackendKind::Cuda);
        assert!(plan.config.gpu_layers > 0);
    }

    fn model(size_bytes: i64) -> ModelRecord {
        ModelRecord {
            id: "qwen".to_string(),
            name: "Qwen3 4B".to_string(),
            family: Some("qwen3".to_string()),
            path: "model.gguf".to_string(),
            format: "gguf".to_string(),
            quantization: Some("Q4_K_M".to_string()),
            size_bytes,
            capabilities: "[\"chat\"]".to_string(),
            context_length: Some(40960),
            preferred_backend: Some("vulkan".to_string()),
            enabled: true,
            source_repository: Some("Qwen/Qwen3-4B-GGUF".to_string()),
            verification: Some("unverified".to_string()),
            state: crate::model_registry::ModelLifecycleState::Ready,
            created_at: "now".to_string(),
            updated_at: "now".to_string(),
        }
    }

    fn hardware_with_vram(dedicated_vram: u64, shared_memory: u64) -> HardwareProfile {
        HardwareProfile {
            operating_system: "test".to_string(),
            architecture: "x86_64".to_string(),
            cpu: CpuProfile {
                name: "cpu".to_string(),
                physical_cores: Some(6),
                logical_threads: 12,
            },
            memory: MemoryProfile {
                total_bytes: 16 * 1024 * 1024 * 1024,
                available_bytes: 10 * 1024 * 1024 * 1024,
            },
            gpus: vec![GpuInfo {
                id: "gpu0".to_string(),
                name: "AMD Radeon RX 580 2048SP".to_string(),
                vendor: GpuVendor::Amd,
                vendor_id: Some(0x1002),
                device_id: None,
                subsystem_id: None,
                revision: None,
                dedicated_vram_bytes: Some(dedicated_vram),
                dedicated_system_memory_bytes: Some(0),
                shared_memory_bytes: Some(shared_memory),
                luid: None,
                is_discrete: true,
                is_integrated: false,
                is_software: false,
                available_backends: vec![BackendKind::Cpu, BackendKind::Vulkan],
                recommended_backend: BackendKind::Vulkan,
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
        }
    }
}
