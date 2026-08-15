# Troubleshooting

## Windows warns "Windows protected your PC" when I run the installer

Expected for now — the installer isn't code-signed yet (see
[docs/RELEASE.md](RELEASE.md)). Click "More info" → "Run anyway" if you
trust where you downloaded it from, or verify it against the published
`SHA256SUMS.txt` first if you want extra assurance.

## The AI model or AI engine failed to download

Setup automatically resumes an interrupted download the next time it runs —
close and reopen OpenMindAI, or click "Retry" on the setup screen. Downloads
use HTTP range requests against the partial file, so you don't lose
progress and don't re-download from zero. If it keeps failing, check:

- You have internet access (this step needs it — see
  [docs/OFFLINE-MODE.md](OFFLINE-MODE.md)).
- You have enough free disk space (setup previews how much is needed and
  how much you have).
- Settings → Maintenance → "Repair OpenMindAI" — safe to run any time, only
  touches what's actually missing, never your conversations.

## GPU not detected / not accelerated

OpenMindAI detects your GPU and picks the best available backend for it
(CUDA for NVIDIA, HIP for AMD, SYCL for Intel, falling back to Vulkan, then
CPU). If none of those apply or the detected build doesn't actually run on
your hardware, it falls back automatically rather than failing outright —
chat still works, just on CPU instead of GPU. Settings shows which backend
is active. If you believe your GPU should be detected and isn't, Settings →
Maintenance → Diagnostics will show what was actually found.

## Vulkan unavailable

Vulkan support depends on `vulkaninfo` being available via your GPU driver.
Updating your graphics driver from your GPU vendor (NVIDIA/AMD/Intel)
usually resolves this. OpenMindAI still works without Vulkan — it falls
back to CPU.

## AI engine cannot start / chat doesn't respond

Try Settings → Maintenance → "Repair OpenMindAI" first. If that doesn't
help, check Settings → Maintenance → Logs ("View Recent Activity" or "Open
Logs Folder") for what actually happened — the raw local AI server output
is captured there alongside OpenMindAI's own structured log.

## "OpenMindAI cannot access your AI storage location"

This means your chosen `OPENMINDAI_ROOT` folder (see
[docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md)) isn't reachable anymore —
usually an external/secondary drive that's been unplugged or a folder that
was moved or deleted. OpenMindAI deliberately does not silently create a
new, different storage location in this situation, since that would look
like your models and conversations vanished. Reconnect the original drive,
or use the folder picker to locate it if it moved.

## Low disk space

Setup and the model/runtime downloaders check free space before starting
and refuse to begin a multi-gigabyte download without enough margin.
Diagnostics also warns under ~2 GB free. Free up space on the drive your
`OPENMINDAI_ROOT` lives on, or move your storage location to a drive with
more room (via a fresh setup, or manually moving the folder and updating
the saved location).

## The app feels slow / responses are slow

Without a supported GPU, OpenMindAI runs the model on CPU, which is
noticeably slower but fully functional. This is expected CPU-fallback
behavior, not a bug — see "GPU not detected" above if you believe GPU
acceleration should be available on your hardware.

## Something else

Settings → Maintenance → Diagnostics is the fastest way to see what
OpenMindAI itself thinks is wrong. If you need to report an issue, include
the diagnostics output and, if relevant, the recent log activity.
