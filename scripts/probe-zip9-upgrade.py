#!/usr/bin/env python3
"""Probe ZIP 9 migration against exact, Cargo-verified upstream crate sources.

This intentionally works on disposable copies and NEVER edits the checked-in
Cargo.toml, Cargo.lock, installed crate cache, or production TTS dependency.
See docs/zip9-upstream-migration.md.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "src-tauri" / "Cargo.toml"
LOCK = ROOT / "src-tauri" / "Cargo.lock"
TARGETS = {"any-tts": "0.1.3", "candle-core": "0.10.2"}
ZIP_TARGET = "9.0.4"


def replace_zip_requirement(contents: str) -> str:
    """Change ONLY the zip dependency requirement, not other 7.x versions."""
    lines = contents.splitlines(keepends=True)
    section = ""
    edits = 0
    for index, line in enumerate(lines):
        if line.lstrip().startswith("["):
            section = line.strip()
        if section == "[dependencies.zip]" and re.match(r"^version\s*=\s*\"7(?:\.\d+)*\"", line):
            lines[index] = re.sub(r'^(version\s*=\s*)\"[^\"]+\"', r'\g<1>"' + ZIP_TARGET + '"', line)
            edits += 1
        elif section == "[dependencies]" and re.match(r"^zip\s*=", line):
            new = re.sub(r'(\bversion\s*=\s*)\"7(?:\.\d+)*\"', r'\g<1>"' + ZIP_TARGET + '"', line)
            if new != line:
                lines[index] = new
                edits += 1
    if edits != 1:
        raise ValueError(f"Expected exactly one zip 7 dependency requirement, found {edits}")
    return "".join(lines)


def replace_once(source: Path, old: str, new: str) -> None:
    original = source.read_text(encoding="utf-8")
    if original.count(old) != 1:
        raise RuntimeError(f"Expected exactly one reviewed ZIP 9 API call in {source}: {old[:75]}")
    source.write_text(original.replace(old, new), encoding="utf-8")


def adapt_upstream_zip9_api(crate: str, directory: Path) -> None:
    """Apply small, exact upstream ZIP 9 filename API changes; fail on drift."""
    if crate == "candle-core":
        npy = directory / "src" / "npy.rs"
        replace_once(
            npy,
            "let name = reader.name();\n                name.strip_suffix(NPY_SUFFIX).unwrap_or(name).to_owned()",
            "let name = reader.name()?;\n                name.strip_suffix(NPY_SUFFIX).unwrap_or(&name).to_owned()",
        )
        replace_once(
            npy,
            "let name = file.name();\n                name.strip_suffix(NPY_SUFFIX).unwrap_or(name).to_owned()",
            "let name = file.name()?;\n                name.strip_suffix(NPY_SUFFIX).unwrap_or(&name).to_owned()",
        )
        replace_once(
            directory / "src" / "pickle.rs",
            ".map(|f| f.to_string())\n        .collect::<Vec<String>>()",
            ".map(|f| f.map(|f| f.to_string()))\n        .collect::<std::result::Result<Vec<String>, _>>()?",
        )
    elif crate == "any-tts":
        replace_once(
            directory / "src" / "models" / "kokoro" / "model.rs",
            "let names: Vec<String> = zip.file_names().map(|s| s.to_string()).collect();",
            "let names: Vec<String> = zip.file_names()\n"
            "                .map(|name| name.map(|name| name.to_string()))\n"
            "                .collect::<Result<Vec<_>, _>>()\n"
            "                .map_err(|error| TtsError::WeightLoadError(format!(\"Invalid .pt ZIP name: {error}\")))?;",
        )
        replace_once(
            directory / "src" / "models" / "kokoro" / "model.rs",
            ".file_names()\n"
            "            .map(|name| name.to_string())\n"
            "            .collect::<Vec<_>>();",
            ".file_names()\n"
            "            .map(|name| name.map(|name| name.to_string()))\n"
            "            .collect::<std::result::Result<Vec<_>, _>>()\n"
            "            .map_err(|error| TtsError::WeightLoadError(format!(\"Invalid .pth ZIP name: {error}\")))?;",
        )


def registry_source(crate: str, version: str, cargo_home: Path) -> Path:
    paths = sorted((cargo_home / "registry" / "src").glob(f"*/{crate}-{version}/Cargo.toml"))
    verified = []
    for manifest_path in paths:
        metadata = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
        package = metadata.get("package", {})
        if package.get("name") == crate and package.get("version") == version:
            verified.append(manifest_path.parent)
    if len(verified) != 1:
        raise RuntimeError(f"Expected exactly one fetched {crate}@{version}, got {len(verified)}")
    return verified[0]


def run(command: list[str], env: dict[str, str] | None = None) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, check=True, cwd=ROOT, env=env)


def lock_versions(path: Path, name: str) -> list[str]:
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    return [p["version"] for p in document["package"] if p["name"] == name]


def self_test() -> None:
    a = '[package]\nversion = "7.2.0"\n[dependencies.zip]\nversion = "7.2.0"\ndefault-features = false\n'
    b = '[dependencies]\nzip = { version = "7.2", default-features = false }\nother = "7.2"\n'
    assert 'zip]\nversion = "9.0.4"' in replace_zip_requirement(a)
    assert 'zip = { version = "9.0.4"' in replace_zip_requirement(b)
    assert 'other = "7.2"' in replace_zip_requirement(b)
    for bad in ['[dependencies]\nzip = "9.0.4"\n', '[dependencies]\nzip = "7.2"\n']:
        try:
            replace_zip_requirement(bad)
        except ValueError:
            continue
        raise AssertionError("Unsupported ZIP dependency syntax must fail closed")
    print("ZIP 9 probe self-test passed")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--cargo-home", type=Path)
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    cargo_home = args.cargo_home or Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    document = tomllib.loads(LOCK.read_text(encoding="utf-8"))
    packages = {(p["name"], p["version"]) for p in document["package"]}
    for item in TARGETS.items():
        if item not in packages:
            raise RuntimeError(f"Unreviewed upstream version: {item}; reassess migration")
    if "7.2.0" not in lock_versions(LOCK, "zip"):
        raise RuntimeError("ZIP 7.2.0 no longer in lockfile; migration already changed")
    # Cargo fetch validates registry packages against Cargo.lock checksums.
    run(["cargo", "fetch", "--locked", "--manifest-path", str(MANIFEST)])
    with tempfile.TemporaryDirectory(prefix="openmind-zip9-probe-") as temp:
        overlay = Path(temp)
        patches = []
        for crate, version in TARGETS.items():
            src = registry_source(crate, version, cargo_home)
            dst = overlay / f"{crate}-{version}"
            shutil.copytree(src, dst)
            cargo_toml = dst / "Cargo.toml"
            cargo_toml.write_text(replace_zip_requirement(cargo_toml.read_text(encoding="utf-8")), encoding="utf-8")
            adapt_upstream_zip9_api(crate, dst)
            patches.extend(["--config", f'patch.crates-io.{crate}.path="{dst}"'])
            print(f"Locally substituted {crate}@{version} zip requirement with {ZIP_TARGET}")
        # Cargo cannot run integration tests in a non-workspace dependency
        # via -p. Use a disposable standalone harness that depends on the
        # exact patched Candle source instead of editing its registry files.
        trial = overlay / "trial"
        trial.mkdir()
        (trial / "tests").mkdir()
        shutil.copy2(
            ROOT / "scripts" / "zip9-reader-regressions.rs",
            trial / "tests" / "zip9_reader_regressions.rs",
        )
        (trial / "Cargo.toml").write_text(
            '[package]\\nname = "openmind-zip9-readers"\\nversion = "0.0.0"\\nedition = "2021"\\n'
            '[dependencies]\\ncandle-core = { path = "../candle-core-0.10.2" }\\n'
            'zip = "=9.0.4"\\n',
            encoding="utf-8",
        )
        # The production Cargo manifests and lockfile still remain untouched
        # apart from the temporary Cargo resolver probe guarded by finally.
        original = MANIFEST.read_text(encoding="utf-8")
        scratch_manifest = MANIFEST
        scratch_lock = LOCK
        try:
            # Cargo patch CLI config allows isolated source overrides; the
            # temporarily updated lockfile is restored in finally even on fail.
            original_lock = scratch_lock.read_bytes()
            try:
                run(["cargo", *patches, "update", "--manifest-path", str(MANIFEST), "-p", "any-tts", "-p", "candle-core"])
                versions = lock_versions(LOCK, "zip")
                if any(v.startswith(("7.", "8.")) for v in versions):
                    raise RuntimeError(f"Legacy zip versions still resolved: {versions}")
                run(["cargo", *patches, "check", "--manifest-path", str(MANIFEST), "--lib"])
                testing_env = os.environ.copy()
                testing_env["CARGO_TARGET_DIR"] = str(ROOT / "src-tauri" / "target")
                run([
                    "cargo", "test", "--manifest-path", str(trial / "Cargo.toml"),
                    "--test", "zip9_reader_regressions", "--", "--nocapture",
                ], env=testing_env)
                test_zip_versions = lock_versions(trial / "Cargo.lock", "zip")
                if any(v.startswith(("7.", "8.")) for v in test_zip_versions):
                    raise RuntimeError(
                        f"Standalone ZIP 9 reader harness unexpectedly links old ZIP: {test_zip_versions}"
                    )
                print("ZIP 9 compilation and synthetic NPZ/PTH regression probe PASSED.")
                print("Real pretrained model and platform-specific TTS validation are still required.")
            finally:
                scratch_lock.write_bytes(original_lock)
        finally:
            if scratch_manifest.read_text(encoding="utf-8") != original:
                raise RuntimeError("Unexpected source manifest mutation in ZIP 9 probe")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"ZIP 9 compatibility probe FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
