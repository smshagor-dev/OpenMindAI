#!/usr/bin/env python3
"""End-to-end test of the real, hash-pinned Kokoro checkpoint and generated WAV.

No unsafe Python pickle loading is performed. The Rust/Candle media_smoke
example does the actual inference and checks the existing ZIP admission policy.
Do not upload third-party model or voice weights as CI artifacts.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
from pathlib import Path
import shutil
import struct
import tempfile
import urllib.request
import wave

REPO = "hexgrad/Kokoro-82M"
REVISION = "04393022a04dcb701fd2061b8d5dbe23ac7be751"
FILES = {
    "kokoro-v1_0.pth": {
        "sha256": "496dba118d1a58f5f3db2efc88dbdc216e0483fc89fe6e47ee1f2c53f18ad1e4",
        "limit": 350_000_000,
        "role": "checkpoint",
        "format": "pth",
    },
    "config.json": {
        # SHA-256 measured on immutable revision in successful real inference
        # run #39274446841. Reject changed config bytes before model loading.
        "sha256": "5abb01e2403b072bf03d04fde160443e209d7a0dad49a423be15196b9b43c17f",
        "limit": 65_536,
        "role": "config",
        "format": "json",
    },
    "voices/af_heart.pt": {
        "sha256": "0ab5709b8ffab19bfd849cd11d98f75b60af7733253ad0d67b12382a102cb4ff",
        "limit": 2_000_000,
        "role": "voice",
        "format": "pt",
    },
}
DESTINATION = "models/audio/kokoro"
SAMPLE_RATE = 24_000
MODEL_ID = "kokoro-82m-onnx"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def get_asset(name: str, dest: Path, *, download: bool) -> tuple[int, str, str]:
    item = FILES[name]
    url = f"https://huggingface.co/{REPO}/resolve/{REVISION}/{name}"
    dest.parent.mkdir(parents=True, exist_ok=True)
    if download:
        part = dest.with_name(dest.name + ".partial")
        if part.exists():
            part.unlink()
        request = urllib.request.Request(
            url, headers={"User-Agent": "OpenMindAI-Kokoro-Live-Smoke/1"}
        )
        count = 0
        try:
            with urllib.request.urlopen(request, timeout=120) as response, part.open("wb") as out:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    count += len(chunk)
                    if count > item["limit"]:
                        raise ValueError(f"{name}: download exceeds verified size limit")
                    out.write(chunk)
            if count < 100:
                raise ValueError(f"{name}: downloaded file is empty/invalid")
            digest = sha256(part)
            if item["sha256"] and digest != item["sha256"]:
                raise ValueError(f"{name}: wrong SHA-256 {digest}")
            os.replace(part, dest)
        finally:
            if part.exists():
                part.unlink()
    if not dest.is_file() or dest.is_symlink():
        raise ValueError(f"{name}: required regular model asset missing")
    size = dest.stat().st_size
    if size < 100 or size > item["limit"]:
        raise ValueError(f"{name}: invalid file size {size}")
    digest = sha256(dest)
    if item["sha256"] and digest != item["sha256"]:
        raise ValueError(f"{name}: failed pinned SHA-256 verification")
    return size, digest, url


def prepare(root: Path, *, download: bool) -> None:
    root = root.resolve()
    dest = root / DESTINATION
    if dest.is_symlink():
        raise ValueError("Model package directory is a symlink")
    dest.mkdir(parents=True, exist_ok=True)
    records = {}
    for name in FILES:
        size, digest, url = get_asset(name, dest / name, download=download)
        records[name] = {"size": size, "sha256": digest, "url": url}
    config = json.loads((dest / "config.json").read_text(encoding="utf-8"))
    if not isinstance(config, dict) or not isinstance(config.get("vocab"), dict):
        raise ValueError("Pinned model config lacks Kokoro vocabulary")
    manifest = {
        "modelId": MODEL_ID,
        "repo": REPO,
        "files": [
            {
                "role": FILES[name]["role"],
                "filename": name,
                "format": FILES[name]["format"],
                "sizeBytes": data["size"],
                "sha256": data["sha256"],
                "actualSha256": data["sha256"],
                "verification": "verified",
                "sourceUrl": data["url"],
            }
            for name, data in records.items()
            if name != "kokoro-v1_0.pth"
        ],
    }
    (dest / "package-manifest.json").write_text(
        json.dumps(manifest, sort_keys=True, indent=2) + "\n", encoding="utf-8"
    )
    checkpoint = records["kokoro-v1_0.pth"]
    (dest / "model-manifest.json").write_text(
        json.dumps(
            {
                "modelId": MODEL_ID,
                "repo": REPO,
                "filename": "kokoro-v1_0.pth",
                "sizeBytes": checkpoint["size"],
                "sha256": checkpoint["sha256"],
                "actualSha256": checkpoint["sha256"],
                "verification": "verified",
            },
            sort_keys=True,
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )
    print("Kokoro package SHA-256 verified; manifests staged for real Rust/Candle inference")
    for name, data in records.items():
        print(f"{name}: {data['size']} bytes, sha256 {data['sha256']}")



def verify_production_install(root: Path) -> None:
    """Verify the *actual* model manager's manifests; never create or repair them."""
    root = root.resolve(strict=True)
    model_dir = root / DESTINATION
    if model_dir.is_symlink() or not model_dir.is_dir():
        raise ValueError("Kokoro installation directory missing or linked")
    expected = {
        name: get_asset(name, model_dir / name, download=False)
        for name in FILES
    }
    primary_path = model_dir / "model-manifest.json"
    dependency_path = model_dir / "package-manifest.json"
    for path in (primary_path, dependency_path):
        if not path.is_file() or path.is_symlink() or path.stat().st_size > 65536:
            raise ValueError(f"Production model manifest missing/unsafe: {path}")
    primary = json.loads(primary_path.read_text(encoding="utf-8"))
    pkg = json.loads(dependency_path.read_text(encoding="utf-8"))
    model_size, model_sha, _ = expected["kokoro-v1_0.pth"]
    if (primary.get("repo") != REPO
            or primary.get("filename") != "kokoro-v1_0.pth"
            or primary.get("sizeBytes") != model_size
            or primary.get("sha256") != model_sha
            or primary.get("actualSha256") != model_sha
            or primary.get("verification") != "verified"):
        raise ValueError("Production model downloader did not install a verified checkpoint manifest")
    if pkg.get("modelId") != MODEL_ID or pkg.get("repo") != REPO:
        raise ValueError("Production package-manifest identity mismatch")
    records = pkg.get("files")
    if not isinstance(records, list) or len(records) != 2:
        raise ValueError("Production downloader must install exactly the two Kokoro dependencies")
    by_role = {entry.get("role"): entry for entry in records if isinstance(entry, dict)}
    if set(by_role) != {"voice", "config"} or len(by_role) != len(records):
        raise ValueError("Missing/duplicate production voice or config dependency")
    for name in ("config.json", "voices/af_heart.pt"):
        entry = by_role[FILES[name]["role"]]
        size, digest, _ = expected[name]
        if (entry.get("filename") != name
                or entry.get("format") != FILES[name]["format"]
                or entry.get("sizeBytes") != size
                or entry.get("sha256") != digest
                or entry.get("actualSha256") != digest
                or entry.get("verification") != "verified"):
            raise ValueError(f"Production downloader dependency record invalid: {name}")
    print("Actual Model Download Manager package and manifests verified against pinned model hashes")


def verify_wav(path: Path, evidence: Path) -> dict:
    if not path.is_file() or path.is_symlink():
        raise ValueError(f"Real Kokoro speech output missing: {path}")
    with wave.open(str(path), "rb") as audio:
        channels = audio.getnchannels()
        frequency = audio.getframerate()
        width = audio.getsampwidth()
        frames = audio.getnframes()
        compression = audio.getcomptype()
        samples = audio.readframes(frames)
    if (channels, frequency, width, compression) != (1, SAMPLE_RATE, 2, "NONE"):
        raise ValueError("Kokoro emitted invalid audio: expected 24kHz mono PCM16 WAV")
    seconds = frames / SAMPLE_RATE
    if not (0.25 <= seconds <= 60):
        raise ValueError(f"Out-of-range WAV duration: {seconds:.3f}s")
    if len(samples) != frames * 2:
        raise ValueError("WAV sample count and payload length disagree")
    values = struct.iter_unpack("<h", samples)
    power = sum((value[0] / 32768) ** 2 for value in values) / frames
    rms = math.sqrt(power)
    if not math.isfinite(rms) or not (0.0005 < rms < 0.98):
        raise ValueError(f"Generated WAV is silent/clipped/invalid: RMS {rms}")
    report = {
        "model": MODEL_ID,
        "upstreamRevision": REVISION,
        "runnerOs": platform.system(),
        "runnerArchitecture": platform.machine(),
        "checkpointSha256": FILES["kokoro-v1_0.pth"]["sha256"],
        "configSha256": FILES["config.json"]["sha256"],
        "voiceSha256": FILES["voices/af_heart.pt"]["sha256"],
        "outputSha256": sha256(path),
        "durationSeconds": round(seconds, 3),
        "samples": frames,
        "sampleRate": frequency,
        "channels": channels,
        "sampleWidthBytes": width,
        "rms": round(rms, 7),
        "realModelInference": True,
    }
    evidence.parent.mkdir(parents=True, exist_ok=True)
    evidence.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(f"Real Kokoro audio validation PASS: {seconds:.2f}s at {frequency} Hz; RMS={rms:.5f}")
    return report


def self_test() -> None:
    # Every remote input is now hash-pinned, not merely revision-pinned.
    for name, item in FILES.items():
        checksum = item["sha256"]
        assert isinstance(checksum, str) and len(checksum) == 64, name
        assert all(ch in "0123456789abcdef" for ch in checksum), name
    with tempfile.TemporaryDirectory(prefix="kokoro-real-audio-qa-") as tmp:
        file = Path(tmp) / "sample.wav"
        report = Path(tmp) / "evidence.json"
        with wave.open(str(file), "wb") as output:
            output.setnchannels(1)
            output.setframerate(SAMPLE_RATE)
            output.setsampwidth(2)
            output.writeframes(b"\x20\x03" * 24_000)
        verified = verify_wav(file, report)
        assert verified["samples"] == SAMPLE_RATE
        assert verified["runnerOs"] == platform.system()
        assert json.loads(report.read_text())["sampleRate"] == SAMPLE_RATE
        with wave.open(str(file), "wb") as output:
            output.setnchannels(1)
            output.setframerate(SAMPLE_RATE)
            output.setsampwidth(2)
            output.writeframes(b"\0\0" * 24_000)
        try:
            verify_wav(file, report)
        except ValueError as error:
            assert "silent" in str(error)
        else:
            raise AssertionError("Silent WAV falsely passed real synthesis QA")
    print("Kokoro audio verifier self-test PASS; this fixture is not real model inference")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    commands = parser.add_subparsers(dest="command", required=True)
    setup = commands.add_parser("prepare")
    setup.add_argument("--root", type=Path, required=True)
    setup.add_argument("--download", action="store_true")
    installed = commands.add_parser("verify-install")
    installed.add_argument("--root", type=Path, required=True)
    quality = commands.add_parser("verify-wav")
    quality.add_argument("--wav", type=Path, required=True)
    quality.add_argument("--evidence", type=Path, required=True)
    commands.add_parser("self-test")
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.root, download=args.download)
        elif args.command == "verify-wav":
            verify_wav(args.wav, args.evidence)
        elif args.command == "verify-install":
            verify_production_install(args.root)
        else:
            self_test()
    except (ValueError, OSError, urllib.error.URLError, json.JSONDecodeError) as error:
        parser.exit(1, f"Kokoro real-model smoke FAILED: {error}\n")
