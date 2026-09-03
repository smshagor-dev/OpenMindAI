"""Explicit native adapter activation and rollback, scoped to one local profile."""
from __future__ import annotations

from contextlib import contextmanager
import hashlib
import json
import math
from pathlib import Path
import subprocess
import tempfile

from .pipeline import atomic_json, read_json, sha256


def active_path(directory: Path, model_id: str) -> Path:
    return directory / (hashlib.sha256(model_id.encode()).hexdigest() + ".json")


@contextmanager
def activation_lock(directory: Path):
    directory.mkdir(parents=True, exist_ok=True)
    lock = directory / ".activation.lock"
    try:
        lock.mkdir()
    except FileExistsError as error:
        raise RuntimeError("another activation is in progress") from error
    try:
        yield
    finally:
        lock.rmdir()


def checked_candidate(candidate: Path, *, profile_id: str, model_id: str,
                      base_gguf: Path) -> dict:
    value = read_json(candidate)
    if value.get("schema_version") != 1 or value.get("profile_id") != profile_id or value.get("model_id") != model_id:
        raise ValueError("candidate identity mismatch")
    score = value["evaluation"]
    baseline, proposed = score["baseline_loss"], score["candidate_loss"]
    if score.get("accepted") is not True or not all(math.isfinite(n) for n in (baseline, proposed)) or not 0 <= proposed < baseline or score["holdout_examples"] < 12:
        raise ValueError("candidate did not pass held-out evaluation")
    adapter = Path(value["adapter_path"])
    if not adapter.is_absolute() or adapter.stat().st_size > 512 * 1024 * 1024:
        raise ValueError("invalid adapter path/size")
    if sha256(base_gguf) != value["base_sha256"] or sha256(adapter) != value["adapter_sha256"]:
        raise ValueError("base or adapter checksum mismatch")
    return value


def probe_native(*, candidate: Path, profile_id: str, model_id: str,
                 base_gguf: Path, worker: Path, prompt: str) -> None:
    with tempfile.TemporaryDirectory(prefix="openmind-adapter-probe-") as temporary:
        directory = Path(temporary)
        active = directory / "active.json"
        atomic_json(active, {"schema_version": 1, "profile_id": profile_id, "model_id": model_id,
                             "candidate": str(candidate.resolve())})
        registry = directory / "models.json"
        atomic_json(registry, {model_id: {"path": str(base_gguf.resolve()), "context_size": 512,
                                         "personalization": str(active)}})
        request = {"id": 1, "model": model_id, "temperature": 0, "max_tokens": 16,
                   "timeout_ms": 25000, "messages": [{"role": "user", "content": prompt}]}
        result = subprocess.run([str(worker.resolve()), "--models", str(registry)],
                                input=json.dumps(request) + "\n", text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=30)
        if result.returncode != 0 or len(result.stdout) > 2**20:
            raise ValueError("native adapter probe failed")
        events = [json.loads(line) for line in result.stdout.splitlines()]
        if not events or events[0].get("type") != "ready" or events[-1].get("type") != "done" or events[-1].get("id") != 1 or any(e.get("type") == "error" for e in events) or not any(e.get("type") == "token" and e.get("text") for e in events):
            detail = [(event.get("type"), event.get("code"), event.get("message")) for event in events if event.get("type") != "token"]
            raise ValueError(f"native adapter did not produce a complete nonempty response: {detail}")


def activate(candidate: Path, *, directory: Path, profile_id: str, model_id: str,
             base_gguf: Path, worker: Path) -> Path:
    with activation_lock(directory):
        data = checked_candidate(candidate, profile_id=profile_id, model_id=model_id, base_gguf=base_gguf)
        target = active_path(directory, model_id)
        previous = read_json(target) if target.exists() else {
            "schema_version": 1, "profile_id": profile_id, "model_id": model_id, "candidate": None,
        }
        if previous.get("profile_id") != profile_id or previous.get("model_id") != model_id:
            raise ValueError("activation directory belongs to another profile/model")
        probe_native(candidate=candidate, profile_id=profile_id, model_id=model_id,
                     base_gguf=base_gguf, worker=worker, prompt=data["probe_prompt"])
        # Verify again after probing, before changing the active pointer.
        checked_candidate(candidate, profile_id=profile_id, model_id=model_id, base_gguf=base_gguf)
        atomic_json(target.with_suffix(".previous.json"), previous)
        atomic_json(target, {"schema_version": 1, "profile_id": profile_id, "model_id": model_id,
                             "candidate": str(candidate.resolve())})
        return target


def rollback(*, directory: Path, profile_id: str, model_id: str,
             base_gguf: Path, worker: Path, disable: bool = False) -> Path:
    with activation_lock(directory):
        target = active_path(directory, model_id)
        current = read_json(target)
        if current.get("profile_id") != profile_id or current.get("model_id") != model_id:
            raise ValueError("activation identity mismatch")
        previous = ({"schema_version": 1, "profile_id": profile_id, "model_id": model_id,
                     "candidate": None} if disable else read_json(target.with_suffix(".previous.json")))
        if previous.get("profile_id") != profile_id or previous.get("model_id") != model_id:
            raise ValueError("rollback identity mismatch")
        if previous.get("candidate") is not None:
            candidate = Path(previous["candidate"])
            data = checked_candidate(candidate, profile_id=profile_id, model_id=model_id, base_gguf=base_gguf)
            probe_native(candidate=candidate, profile_id=profile_id, model_id=model_id,
                         base_gguf=base_gguf, worker=worker, prompt=data["probe_prompt"])
        atomic_json(target.with_suffix(".previous.json"), current)
        atomic_json(target, previous)
        return target
