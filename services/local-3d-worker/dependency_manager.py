"""Pinned, hash-verified optional dependency provisioning for Desktop 3D Studio."""
from __future__ import annotations

import hashlib
import json
import os
import ctypes
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import venv

MANIFEST = Path(__file__).with_name("dependency-manifest.json")
REQUIRED_FREE_BYTES = 400 * 1024 * 1024


class InstallError(RuntimeError):
    pass


def load_manifest(path: Path = MANIFEST) -> dict:
    return load_manifest_from_data(json.loads(path.read_text(encoding="utf-8")))


def load_manifest_from_data(data: dict) -> dict:
    if data.get("schemaVersion") != 1 or not isinstance(data.get("supportedTargets"), list):
        raise InstallError("Dependency manifest schema is not supported")
    for target in data["supportedTargets"]:
        for package in target.get("packages", []):
            parsed = urllib.parse.urlparse(package.get("url", ""))
            digest = package.get("sha256", "")
            filename = package.get("filename", "")
            if parsed.scheme != "https" or parsed.hostname != "files.pythonhosted.org":
                raise InstallError("Manifest package source is not the approved PyPI artifact host")
            if Path(filename).name != filename or len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest):
                raise InstallError("Manifest contains an invalid filename or SHA-256")
            if not isinstance(package.get("size"), int) or package["size"] <= 0:
                raise InstallError("Manifest package size is invalid")
    return data


def target_for(manifest: dict, version_info=sys.version_info, system=platform.system(), machine=platform.machine()) -> dict | None:
    os_name = {"Windows": "windows", "Linux": "linux", "Darwin": "macos"}.get(system)
    architecture = {"AMD64": "x86_64", "x86_64": "x86_64", "aarch64": "aarch64", "ARM64": "aarch64"}.get(machine)
    abi = f"cp{version_info.major}{version_info.minor}"
    return next((t for t in manifest["supportedTargets"]
                 if t.get("os") == os_name and t.get("architecture") == architecture
                 and t.get("abi") == abi and t.get("implementation") == "CPython"), None)


def _python_in(venv_path: Path) -> Path:
    return venv_path / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


def _active_healthy(venv_path: Path) -> bool:
    python = _python_in(venv_path)
    if not python.is_file():
        return False
    check = "import numpy, pyopencl, platformdirs, pytools, siphash24, typing_extensions, mako, markupsafe"
    try:
        return subprocess.run([str(python), "-c", check], capture_output=True, text=True, timeout=20).returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


def _pid_is_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    if os.name == "nt":
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel32.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        kernel32.OpenProcess.restype = ctypes.c_void_p
        kernel32.GetExitCodeProcess.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong)]
        kernel32.GetExitCodeProcess.restype = ctypes.c_int
        kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel32.OpenProcess(0x1000, 0, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if handle:
            code = ctypes.c_ulong()
            try:
                return bool(kernel32.GetExitCodeProcess(handle, ctypes.byref(code)) and code.value == 259)
            finally:
                kernel32.CloseHandle(handle)
        # Access denied means the process exists but cannot be inspected. An
        # invalid parameter/handle means it no longer exists; fail closed.
        return ctypes.get_last_error() == 5
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError as exc:
        # Windows reports ERROR_INVALID_PARAMETER for an unknown PID.
        if getattr(exc, "winerror", None) == 87:
            return False
        return True


def _emit(payload: dict) -> None:
    print(json.dumps(payload, separators=(",", ":")), flush=True)


def inspect(root: Path, manifest: dict, base_python=sys.executable) -> dict:
    target = target_for(manifest)
    active = root / "active"
    state = "ready" if _active_healthy(active) else ("repair_required" if active.exists() else "not_installed")
    download_bytes = sum(p["size"] for p in target["packages"]) if target else 0
    probe = root
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    available_bytes = shutil.disk_usage(probe).free
    return {
        "manifestVersion": manifest["manifestVersion"],
        "supported": target is not None,
        "target": {k: target[k] for k in ("os", "architecture", "python", "abi")} if target else None,
        "basePython": str(base_python),
        "state": state,
        "downloadBytes": download_bytes,
        "requiredFreeBytes": download_bytes + REQUIRED_FREE_BYTES if target else 0,
        "availableBytes": available_bytes,
        "packages": [{k: p[k] for k in ("name", "version", "size", "license", "reason")} for p in target["packages"]] if target else [],
        "installScope": "OpenMindAI app-managed isolated environment",
        "managedEnvironmentPath": str(active),
        "systemPythonModified": False,
        "gpuDriverInstall": False,
    }


def _download(package: dict, destination: Path, cancel_file: Path | None = None) -> None:
    request = urllib.request.Request(package["url"], headers={"User-Agent": "OpenMindAI-Desktop/3D-Dependency-Manager"})
    digest = hashlib.sha256()
    written = 0
    with urllib.request.urlopen(request, timeout=30) as response, destination.open("xb") as output:
        if urllib.parse.urlparse(response.geturl()).hostname != "files.pythonhosted.org":
            raise InstallError("Package download redirected away from the approved artifact host")
        while True:
            if cancel_file and cancel_file.exists():
                raise InstallError("Installation cancelled during package download")
            block = response.read(1024 * 1024)
            if not block:
                break
            written += len(block)
            if written > package["size"]:
                raise InstallError(f"Downloaded size exceeded manifest for {package['name']}")
            digest.update(block)
            output.write(block)
    if written != package["size"] or digest.hexdigest() != package["sha256"]:
        destination.unlink(missing_ok=True)
        raise InstallError(f"Size or SHA-256 verification failed for {package['name']}")


def _cpu_worker_smoke() -> dict:
    import importlib.util
    worker_path = Path(__file__).with_name("worker.py")
    spec = importlib.util.spec_from_file_location("openmind_worker_smoke", worker_path)
    if spec is None or spec.loader is None:
        raise InstallError("Could not load the bundled geometry worker")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    glb, vertices, triangles = module.make_glb("dependency manager smoke", "object", scene_spec={
        "schemaVersion": 1,
        "objects": [{"name": "Dependency smoke cube", "primitive": "box", "dimensions": [1, 1, 1],
                     "position": [0, 0, 0], "rotation": [0, 0, 0], "color": [0.4, 0.5, 0.6],
                     "metallic": 0, "roughness": 0.7}],
    })
    if not isinstance(glb, bytes) or not glb.startswith(b"glTF"):
        raise InstallError("CPU worker smoke test did not produce a GLB")
    return {"cpuWorkerSmoke": "passed", "glbBytes": len(glb), "vertices": vertices, "triangles": triangles}


def install(root: Path, manifest: dict, base_python=sys.executable, cancel_file: Path | None = None) -> dict:
    target = target_for(manifest)
    if target is None:
        raise InstallError("No approved dependency set matches this OS, architecture, and Python ABI")
    packages = target["packages"]
    required = sum(p["size"] for p in packages) + REQUIRED_FREE_BYTES
    root.mkdir(parents=True, exist_ok=True)
    if shutil.disk_usage(root).free < required:
        raise InstallError(f"Insufficient storage: need {required} free bytes including the safety reserve")
    lock = root / "install.lock"
    try:
        lock.mkdir()
    except FileExistsError as exc:
        owner_file = lock / "owner.json"
        try:
            owner = json.loads(owner_file.read_text(encoding="utf-8"))
            owner_pid = int(owner["pid"])
        except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError):
            owner_pid = -1
        age = max(0.0, time.time() - lock.stat().st_mtime)
        if _pid_is_alive(owner_pid) or (owner_pid < 0 and age < 30):
            raise InstallError("Another dependency installation is already running") from exc
        shutil.rmtree(lock)
        lock.mkdir()
    (lock / "owner.json").write_text(json.dumps({"pid": os.getpid()}), encoding="utf-8")

    stage: Path | None = None
    active = root / "active"
    backup = root / "previous"
    moved_old = False
    try:
        # The exclusive lock makes app-owned staging directories from a killed
        # prior run safe to clean. Never traverse outside this runtime root.
        for stale_stage in root.glob("stage-*"):
            if stale_stage.is_dir() and stale_stage.parent == root:
                shutil.rmtree(stale_stage)
        stage = Path(tempfile.mkdtemp(prefix="stage-", dir=root))
        # Recover a process interruption between moving the old environment and
        # atomically activating the staged replacement.
        if backup.exists() and not active.exists():
            backup.rename(active)
        elif backup.exists() and active.exists():
            if not _active_healthy(active):
                shutil.rmtree(active)
                backup.rename(active)
            else:
                shutil.rmtree(backup)
        wheels = stage / "wheels"
        wheels.mkdir()
        total = len(packages)
        for index, package in enumerate(packages, 1):
            if cancel_file and cancel_file.exists():
                raise InstallError("Installation cancelled")
            wheel = wheels / package["filename"]
            _download(package, wheel, cancel_file)
            _emit({"event": "download", "package": package["name"], "completed": index, "total": total})
        if cancel_file and cancel_file.exists():
            raise InstallError("Installation cancelled")

        environment = stage / "environment"
        venv.EnvBuilder(with_pip=True, clear=True).create(environment)
        py = _python_in(environment)
        command = [str(py), "-m", "pip", "install", "--no-index", "--no-deps", "--disable-pip-version-check"]
        command.extend(str(wheels / p["filename"]) for p in packages)
        subprocess.run(command, check=True, capture_output=True, text=True, timeout=180)
        verify = "import numpy, pyopencl, platformdirs, pytools, siphash24, typing_extensions, mako, markupsafe; print(numpy.__version__); print(pyopencl.VERSION_TEXT)"
        subprocess.run([str(py), "-c", verify], check=True, capture_output=True, text=True, timeout=30)
        shutil.rmtree(wheels)

        if cancel_file and cancel_file.exists():
            raise InstallError("Installation cancelled")
        if backup.exists():
            shutil.rmtree(backup)
        if active.exists():
            active.rename(backup)
            moved_old = True
        environment.rename(active)
        try:
            smoke = subprocess.run([str(_python_in(active)), str(Path(__file__)), "--worker-smoke"],
                                   check=True, capture_output=True, text=True, timeout=45)
            smoke_data = json.loads(smoke.stdout.strip().splitlines()[-1])
            if smoke_data.get("cpuWorkerSmoke") != "passed":
                raise InstallError("Activated worker smoke test failed")
        except Exception:
            if active.exists():
                shutil.rmtree(active)
            if moved_old and backup.exists():
                backup.rename(active)
            raise
        if backup.exists():
            shutil.rmtree(backup)
        result = {"state": "installed", "manifestVersion": manifest["manifestVersion"],
                  "environmentPython": str(_python_in(active)), "packageCount": total,
                  "packageInstallSmoke": "passed", "gpuDriverInstalled": False, **smoke_data}
        _emit({"event": "complete", **result})
        return result
    except Exception:
        if moved_old and not active.exists() and backup.exists():
            backup.rename(active)
        raise
    finally:
        if stage is not None and stage.exists():
            shutil.rmtree(stage)
        if lock.exists():
            shutil.rmtree(lock)
        if cancel_file:
            cancel_file.unlink(missing_ok=True)


def main() -> int:
    if len(sys.argv) >= 2 and sys.argv[1] == "--worker-smoke":
        _emit(_cpu_worker_smoke())
        return 0
    if len(sys.argv) != 3 or sys.argv[1] not in {"inspect", "install"}:
        print("usage: dependency_manager.py inspect|install APP_DATA_DIR", file=sys.stderr)
        return 2
    root = Path(sys.argv[2]).resolve() / "3d-runtime"
    try:
        manifest = load_manifest()
        result = inspect(root, manifest)
        if sys.argv[1] == "install":
            result = install(root, manifest, cancel_file=root / "install.cancel")
        _emit(result)
        return 0
    except Exception as exc:
        _emit({"state": "failed", "error": f"{type(exc).__name__}: {exc}"})
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
