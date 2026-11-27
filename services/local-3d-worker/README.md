# OpenMindAI Local 3D Geometry Worker

Geometry adapter for Desktop 3D Studio Local generation. Desktop runs its selected local llama.cpp model through the native Rust scheduler, validates the bounded scene specification, and sends only that data to this worker. Both object and architecture generation use a prompt-driven schemaVersion 1 plan with 1–32 named primitives. The worker creates editable GLB 2.0 polygon meshes with separate nodes and PBR materials. Missing plans are rejected rather than replaced by a generic sphere, box or house. The legacy schemaVersion 2 residential geometry helper is retained for legacy fixture compatibility, not accepted for new HTTP jobs. It does not execute model-produced code and is not a neural mesh reconstruction engine.

## Runtime and dependency status

Python 3.10+ is required (`py -3` on Windows or `python3` on macOS/Linux). Set `OPENMINDAI_3D_PYTHON` to select a specific executable, including a user-created isolated virtual environment. CPU procedural geometry uses Python's standard library only. NumPy, PyOpenCL, an OpenCL ICD, and a compatible GPU are optional acceleration dependencies. Vulkan llama.cpp support does not imply OpenCL availability.

The native `/health` endpoint reports Python, package, OpenCL platform, device, and kernel readiness diagnostics. If OpenCL is missing or fails initialization, CPU geometry remains available. The application does not currently download or install Python packages: this checkout has no pinned, checksum-verified wheel manifest. It will not modify global Python. OpenCL drivers are OS-level components and are never installed or replaced by the worker.

The worker binds only `127.0.0.1:8765`; requests require a native marker and browser Origin headers are rejected. This limits accidental browser access but is not authentication against other local processes. Desktop owns and terminates the worker process it starts.

## Jobs and output

Rust sends a prompt, mode, idempotency key, and validated `scene_spec` to `POST /jobs`. `GET /jobs/{uuid}` returns worker state and `GET /jobs/{uuid}/mesh` returns the generated Base64 GLB. Desktop persists the mesh as a project asset only after validating the GLB, then completes the SQLite job. Temporary files default to `~/OpenMindAI/3DStudio/<uuid>.glb`; set `OPENMINDAI_3D_OUTPUT_DIR` to override this.

Rust owns durable job state, bounded concurrency, cancellation, timeouts, local model inference, scene validation, output validation, and project asset publication. OpenCL transforms are selected only after platform/device discovery and kernel compilation succeed. Otherwise transforms, topology assembly, and GLB serialization run on CPU. This is not end-to-end GPU residency; CPU primitive construction is not neural reconstruction.

## Validate

```bash
python3 -m unittest discover -s services/local-3d-worker -p 'test_*.py' -v
```

Tests cover CPU geometry and HTTP behavior without a physical GPU. Fake GPU transforms do not count as physical hardware acceptance.