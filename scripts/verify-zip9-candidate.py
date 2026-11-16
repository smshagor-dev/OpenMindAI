#!/usr/bin/env python3
"""Independently verify a downloaded ZIP 9 candidate artifact before promotion.

In particular, upload-artifact defaults to excluding hidden files. Missing
.cargo-checksum.json and other dotfiles change the source tree despite green
build checks on the runner before upload.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import tempfile
import tomllib

TARGETS = {"any-tts": "0.1.3", "candle-core": "0.10.2"}
PATCHED_FILES = {
    "any-tts": {"Cargo.toml", "src/models/kokoro/model.rs"},
    "candle-core": {"Cargo.toml", "src/npy.rs", "src/pickle.rs"},
}
HEX_SHA = re.compile(r"[0-9a-f]{64}\Z")


def file_hash(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_hashes(root: Path) -> dict[str, str]:
    files: dict[str, str] = {}
    if not root.is_dir() or root.is_symlink():
        raise ValueError(f"Missing or symlinked vendor directory: {root}")
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Symlink in copied vendor source: {path}")
        if path.is_file():
            files[path.relative_to(root).as_posix()] = file_hash(path)
    if not files:
        raise ValueError(f"Missing vendor source files: {root}")
    return files


def tree_hash(files: dict[str, str]) -> str:
    digest = hashlib.sha256(b"openmind-zip9-source-tree-v1\0")
    for name, checksum in sorted(files.items()):
        digest.update(name.encode("utf-8") + b"\0")
        digest.update(bytes.fromhex(checksum))
    return digest.hexdigest()


def verify_candidate(root: Path) -> None:
    if not root.is_dir() or root.is_symlink():
        raise ValueError(f"Candidate does not exist or is a symlink: {root}")
    manifest_path = root / "src-tauri" / "Cargo.toml"
    lock_path = root / "src-tauri" / "Cargo.lock"
    report_path = root / "review" / "zip9-source-provenance.json"
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if report.get("schemaVersion") != 1 or report.get("zipVersion") != "9.0.4":
        raise ValueError("Unexpected ZIP 9 provenance schema or version")
    packages = report.get("packages")
    if not isinstance(packages, list) or len(packages) != len(TARGETS):
        raise ValueError("Expected exactly two pinned ZIP 9 source reports")
    by_crate = {p["crate"]: p for p in packages}
    if len(by_crate) != len(packages) or set(by_crate) != set(TARGETS):
        raise ValueError("Unexpected or duplicate source provenance entry")
    patches = manifest.get("patch", {}).get("crates-io", {})
    if set(patches) != set(TARGETS):
        raise ValueError("Path-pinned Cargo sources do not match approved targets")

    items = lock.get("package", [])
    versions = [p["version"] for p in items if p["name"] == "zip"]
    if versions != ["9.0.4"]:
        raise ValueError(f"Candidate Cargo.lock has unexpected ZIP packages: {versions}")
    for crate, version in sorted(TARGETS.items()):
        expected_relative = f"../vendor/{crate}-{version}"
        if patches[crate] != {"path": expected_relative}:
            raise ValueError(f"{crate}: unsafe/unexpected Cargo patch path")
        entries = [p for p in items if p["name"] == crate]
        if len(entries) != 1 or entries[0]["version"] != version or "source" in entries[0]:
            raise ValueError(f"{crate}: Cargo.lock does not use the pinned local source")
        vendor_root = root / "vendor" / f"{crate}-{version}"
        hashes = source_hashes(vendor_root)
        evidence = by_crate[crate]
        if evidence.get("version") != version:
            raise ValueError(f"{crate}: wrong pinned crate version")
        if not HEX_SHA.fullmatch(evidence.get("registryChecksumFromCargoLock", "")):
            raise ValueError(f"{crate}: original registry checksum absent")
        if evidence.get("sourceFileCount") != len(hashes):
            raise ValueError(
                f"{crate}: artifact contains {len(hashes)} files, "
                f"but verified source contained {evidence.get('sourceFileCount')}"
            )
        actual = tree_hash(hashes)
        if actual != evidence.get("patchedTreeSha256"):
            raise ValueError(f"{crate}: uploaded vendor tree SHA-256 mismatch")
        changed = evidence.get("changes", [])
        if not isinstance(changed, list) or {
            entry.get("path") for entry in changed
        } != PATCHED_FILES[crate] or len(changed) != len(PATCHED_FILES[crate]):
            raise ValueError(f"{crate}: unapproved patched source file list")
        for entry in changed:
            name = entry["path"]
            if hashes.get(name) != entry.get("patchedSha256"):
                raise ValueError(f"{crate}: patched file SHA-256 mismatch: {name}")
            if not HEX_SHA.fullmatch(entry.get("originalSha256", "")):
                raise ValueError(f"{crate}: invalid original source hash: {name}")
        crate_manifest = tomllib.loads((vendor_root / "Cargo.toml").read_text())
        if (crate_manifest.get("package", {}).get("name") != crate
                or crate_manifest["package"]["version"] != version):
            raise ValueError(f"{crate}: vendored package identity mismatch")
        diff = (root / "review" / f"zip9-{crate}.patch").read_text(encoding="utf-8")
        for name in PATCHED_FILES[crate]:
            if f"+++ b/{crate}/{name}" not in diff:
                raise ValueError(f"{crate}: missing review diff: {name}")
        print(f"Verified {crate}@{version}: {len(hashes)} files, SHA-256 {actual}")
    print("Downloaded ZIP 9 source-pinned candidate integrity PASSED")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="openmind-zip9-artifact-selftest-") as tmp:
        root = Path(tmp)
        (root / "review").mkdir()
        (root / "src-tauri").mkdir()
        (root / "vendor").mkdir()
        patches = []
        lock = ['version = 4', '', '[[package]]', 'name = "zip"', 'version = "9.0.4"']
        reports = []
        for crate, version in sorted(TARGETS.items()):
            vendor = root / "vendor" / f"{crate}-{version}"
            vendor.mkdir()
            (vendor / "Cargo.toml").write_text(
                f'[package]\nname = "{crate}"\nversion = "{version}"\n'
            )
            # The important regression: CI artifact round-trip must preserve
            # hidden registry metadata, not just regular Rust source files.
            (vendor / ".cargo-checksum.json").write_text('{"files":{}}')
            changes = []
            diff = ""
            for name in sorted(PATCHED_FILES[crate]):
                path = vendor / name
                path.parent.mkdir(parents=True, exist_ok=True)
                if name != "Cargo.toml":
                    path.write_text("test fixture")
                changes.append({
                    "path": name,
                    "patchedSha256": file_hash(path),
                    "originalSha256": "a" * 64,
                })
                diff += f"--- a/{crate}/{name}\n+++ b/{crate}/{name}\n"
            (root / "review" / f"zip9-{crate}.patch").write_text(diff)
            hashes = source_hashes(vendor)
            reports.append({
                "crate": crate,
                "version": version,
                "registryChecksumFromCargoLock": "b" * 64,
                "patchedTreeSha256": tree_hash(hashes),
                "sourceFileCount": len(hashes),
                "changes": changes,
            })
            patches.append(f'{crate} = {{ path = "../vendor/{crate}-{version}" }}')
            lock += ['', '[[package]]', f'name = "{crate}"', f'version = "{version}"']
        (root / "src-tauri" / "Cargo.toml").write_text(
            "[patch.crates-io]\n" + "\n".join(patches) + "\n"
        )
        (root / "src-tauri" / "Cargo.lock").write_text("\n".join(lock) + "\n")
        (root / "review" / "zip9-source-provenance.json").write_text(
            json.dumps({"schemaVersion": 1, "zipVersion": "9.0.4", "packages": reports})
        )
        verify_candidate(root)
        (root / "vendor" / "any-tts-0.1.3" / ".cargo-checksum.json").unlink()
        try:
            verify_candidate(root)
        except ValueError as exc:
            assert "artifact contains" in str(exc)
        else:
            raise AssertionError("Missing hidden vendor metadata must fail")
    print("ZIP 9 artifact verifier self-test PASSED")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("candidate", type=Path, nargs="?")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        directory = args.candidate or (
            Path(os.environ["ZIP9_ROUNDTRIP_DIR"])
            if "ZIP9_ROUNDTRIP_DIR" in os.environ else None
        )
        if directory is None:
            parser.error("Provide a candidate directory or ZIP9_ROUNDTRIP_DIR")
        verify_candidate(directory)
