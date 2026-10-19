#!/usr/bin/env python3
"""Stage a verified ZIP 9 candidate into this review branch, never main."""
from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {"any-tts": "0.1.3", "candle-core": "0.10.2"}
PATCH = (
    '\n\n[patch.crates-io]\n'
    'any-tts = { path = "../vendor/any-tts-0.1.3" }\n'
    'candle-core = { path = "../vendor/candle-core-0.10.2" }\n'
)


def check_manifest(original: str, migrated: str) -> None:
    if migrated != original.rstrip() + PATCH:
        raise RuntimeError("Candidate changes more than approved ZIP 9 path patches")


def check_legacy_lock(lock: Path) -> None:
    packages = tomllib.loads(lock.read_text(encoding="utf-8"))["package"]
    versions = sorted(p["version"] for p in packages if p["name"] == "zip")
    if versions != ["7.2.0", "9.0.4"]:
        raise RuntimeError(f"Original Cargo.lock is not exact legacy baseline: {versions}")


def license_notice(candidate_vendor: Path) -> str:
    origins = {
        "any-tts": "https://github.com/TM9657/any-tts",
        "candle-core": "https://github.com/huggingface/candle",
    }
    notice = [
        "# Third-party source and modification notices", "",
        "Vendored from exact Cargo.lock-verified crates.io packages.",
        "Both crates are modified solely for reviewed ZIP 9 compatibility;",
        "their per-file SHA-256 and unified diffs are in the CI provenance.",
        "No Kokoro model weights are included.", "",
    ]
    for name, version in sorted(TARGETS.items()):
        f = candidate_vendor / f"{name}-{version}" / "Cargo.toml"
        package = tomllib.loads(f.read_text(encoding="utf-8"))["package"]
        if (package.get("name"), package.get("version"), package.get("license")) != (
            name, version, "MIT OR Apache-2.0"
        ):
            raise RuntimeError(f"Missing expected upstream license metadata: {name}")
        notice.extend([
            f"## {name} {version}", "",
            f"- Source: {origins[name]}",
            "- Original license declaration: MIT OR Apache-2.0",
            "- Redistribution here selects the Apache-2.0 option.",
            "- Changes: ZIP requirement and reviewed ZIP reader API adaptation.",
            "",
        ])
    apache = candidate_vendor / "candle-core-0.10.2" / "LICENSE"
    if not apache.is_file():
        raise RuntimeError("Missing canonical Apache license in candidate Candle crate")
    text = apache.read_text(encoding="utf-8")
    if "Apache License" not in text[:100] or "Version 2.0" not in text[:200]:
        raise RuntimeError("Unrecognized upstream license text")
    notice.extend([
        "Apache-2.0 text: vendor/LICENSE-APACHE-2.0, copied from Candle.",
        "The published any-tts crate has dual-license metadata but its",
        "registry archive contains no top-level LICENSE. Review original",
        "attributions and redistribution obligations before approving this PR.",
        "",
    ])
    return "\n".join(notice)


def stage(candidate: Path, apply: bool) -> None:
    candidate = candidate.resolve(strict=True)
    verify = ROOT / "scripts" / "verify-zip9-candidate.py"
    subprocess.run([sys.executable, str(verify), str(candidate)], check=True)
    manifest = ROOT / "src-tauri" / "Cargo.toml"
    lock = ROOT / "src-tauri" / "Cargo.lock"
    changed = (candidate / "src-tauri" / "Cargo.toml").read_text(encoding="utf-8")
    check_manifest(manifest.read_text(encoding="utf-8"), changed)
    check_legacy_lock(lock)
    for name, version in TARGETS.items():
        if (ROOT / "vendor" / f"{name}-{version}").exists():
            raise RuntimeError("Refusing to overwrite existing vendored sources")
    notice = license_notice(candidate / "vendor")
    print("Verified immutable candidate, exact manifest patch, and crate license declarations.")
    if not apply:
        print("Dry run: working tree unchanged. --apply is required on a review branch.")
        return
    status = subprocess.run(
        ["git", "status", "--porcelain", "--", "src-tauri/Cargo.toml",
         "src-tauri/Cargo.lock", "vendor"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    )
    if status.stdout.strip():
        raise RuntimeError("Refusing to stage over dirty Cargo inputs or vendor paths")
    (ROOT / "vendor").mkdir(exist_ok=True)
    for name, version in sorted(TARGETS.items()):
        source = candidate / "vendor" / f"{name}-{version}"
        target = ROOT / "vendor" / f"{name}-{version}"
        shutil.copytree(source, target)
    shutil.copy2(candidate / "vendor" / "candle-core-0.10.2" / "LICENSE",
                 ROOT / "vendor" / "LICENSE-APACHE-2.0")
    (ROOT / "vendor" / "THIRD_PARTY_NOTICES.md").write_text(notice, encoding="utf-8")
    shutil.copy2(candidate / "src-tauri" / "Cargo.lock", lock)
    manifest.write_text(changed, encoding="utf-8")
    print("Staged reviewed ZIP 9-only dependency sources; no commit or merge performed.")


def self_test() -> None:
    original = '[package]\nname = "fixture"\n'
    check_manifest(original, original.rstrip() + PATCH)
    for invalid in (original, original.rstrip() + PATCH + "\nextra = true\n"):
        try:
            check_manifest(original, invalid)
        except RuntimeError:
            continue
        raise AssertionError("Unapproved manifest change accepted")
    print("ZIP 9 production source staging self-test PASSED")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    try:
        if args.self_test:
            self_test()
        elif args.candidate is None:
            parser.error("--candidate is required unless --self-test")
        else:
            stage(args.candidate, args.apply)
    except (ValueError, OSError, RuntimeError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"ZIP 9 staging FAILED: {exc}\n")
