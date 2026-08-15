# Hardware Detection

`HardwareProfiler` detects:

- operating system
- CPU name
- physical cores where available
- logical threads
- total RAM
- available RAM
- native Windows DXGI graphics adapters
- GPU vendor ID and device ID
- dedicated VRAM where DXGI reports it
- shared GPU memory
- software adapter classification
- conservative backend availability

GPU and VRAM values must not be faked. CUDA and Vulkan are reported only when local tools are discoverable or the validated llama.cpp runtime reports a usable backend. Non-Windows platforms currently keep the typed no-GPU fallback until native enumeration is added for those platforms.

Milestone 2.1 execution on the development machine reported:

- OS: Windows 11 Pro
- CPU: 12th Gen Intel(R) Core(TM) i5-12400F
- physical cores: 6
- logical threads: 12
- RAM: 16,976,588,800 bytes
- GPU 1: AMD Radeon RX 580 2048SP
- GPU 1 vendor ID: `0x1002`
- GPU 1 dedicated VRAM: 8,567,902,208 bytes
- GPU 1 shared memory: 8,488,294,400 bytes
- GPU 2: Microsoft Basic Render Driver
- GPU 2 software adapter: true
- hardware probes: CPU true, Vulkan true, CUDA false, SYCL false, HIP false, Metal false

Phase 2 Milestone 2 fresh probes reported:

- CPU: 12th Gen Intel(R) Core(TM) i5-12400F
- physical cores: 6
- logical threads: 12
- RAM: 16,976,588,800 bytes
- GPU: AMD Radeon RX 580 2048SP
- Vulkan device: `Vulkan0`
- llama.cpp device output: `AMD Radeon RX 580 2048SP (8192 MiB, 7402 MiB free)`

The launcher and planner treat dedicated VRAM separately from shared memory. Shared GPU memory is recorded for display only and is not added to the dedicated VRAM offload budget.
