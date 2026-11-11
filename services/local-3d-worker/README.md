# OpenMindAI Local 3D GPU Worker

Optional local processor for Desktop → 3D Studio → Local. This worker creates **real
GLB 2.0 polygon meshes**, not 2.5D images. It currently supports deterministic
procedural shapes and a simple architecture preset. **It is not a learned
text-to-3D generative model**, and neither a production-quality neural mesh
engine nor reference-image reconstruction is implemented locally.

## Prerequisites

- Python 3.10+ available as \`py -3\` (Windows) or \`python3\` (macOS/Linux).
- A working OpenCL GPU driver/runtime (AMD, Intel or NVIDIA) visible to
  PyOpenCL. Merely having VRAM is not enough; the OpenCL driver must work.
- \`pip install numpy pyopencl\` into the Python interpreter selected above.
  Install/build prerequisites from your vendor OS toolchain if pip cannot
  install PyOpenCL.

Example Windows:

\`\`\`powershell
py -3 -m pip install numpy pyopencl
py -3 -c "import pyopencl as cl; print([d.name for p in cl.get_platforms() for d in p.get_devices(device_type=cl.device_type.GPU)])"
\`\`\`

Example Linux:

\`\`\`bash
python3 -m pip install numpy pyopencl
python3 -c 'import pyopencl as cl; print([d.name for p in cl.get_platforms() for d in p.get_devices(device_type=cl.device_type.GPU)])'
\`\`\`

Opening **Local → 3D Studio** asks the Tauri backend to launch the bundled
\`worker.py\` from its trusted application resource path. In a development
checkout it loads \`services/local-3d-worker/worker.py\`. The worker binds only
\`127.0.0.1:8765\`, never \`0.0.0.0\`. A native-only request marker is required;
the worker does not set CORS headers and rejects browser Origin headers.
This limits accidental browser access, **but is not authentication against a
malicious process running on the same computer**.

The Desktop app owns the worker process it starts and terminates it on exit.
You can also click **Stop local worker**. On a missing GPU, driver or Python
package, the UI displays an explicit error. No hidden CPU-only substitute or
Cloud fallback runs.

## Jobs and output

POST \`/jobs\` with \`prompt\`, \`mode\` (\`object\` or \`architecture\`) and
\`request_key\`. GET \`/jobs/{uuid}\` polls the generated job status. GET
\`/jobs/{uuid}/mesh\` returns a Base64 GLB when complete, downloadable from the
Desktop UI. Generated files are stored under
\`~/OpenMindAI/3DStudio/<uuid>.glb\` by default. Set
\`OPENMINDAI_3D_OUTPUT_DIR\` before starting the desktop application to
override the output directory (the packaged launcher currently uses the
default when no custom environment is present).

Transforms are dispatched to the OpenCL device; the mesh topology is
assembled on the CPU and serialized into a standards-compliant GLB on the CPU.
Hence local GPU work is real, but this is **not end-to-end GPU residency**.

## Validate

No GPU required for unit/HTTP contract tests:

\`\`\`bash
python3 -m unittest discover -s services/local-3d-worker -p 'test_*.py' -v
\`\`\`

For real hardware acceptance, start Desktop and check GPU name/status,
generate an Object and Architecture GLB, open the exported files in a GLB
viewer, then verify that the job ran on your actual GPU. CI fakes the GPU
transform and does not count as physical-hardware validation.
