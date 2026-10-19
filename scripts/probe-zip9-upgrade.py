#!/usr/bin/env python3
"""Probe ZIP 9 migration against exact, Cargo-verified upstream crate sources.

The registry source and desktop Cargo crate are copied to scratch space;
resolver changes never write into the checked-in Cargo.toml or Cargo.lock.
See docs/zip9-upstream-migration.md.
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
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
SOURCE_PATCH_FILES = {
    "any-tts": ("Cargo.toml", "src/models/kokoro/model.rs"),
    "candle-core": ("Cargo.toml", "src/npy.rs", "src/pickle.rs"),
}


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



def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_file_hashes(root: Path) -> dict[str, str]:
    """Fingerprint every file; never silently follow a registry source symlink."""
    files: dict[str, str] = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"Unexpected source symlink: {path}")
        if path.is_file():
            files[path.relative_to(root).as_posix()] = sha256_bytes(path.read_bytes())
    if not files:
        raise RuntimeError(f"Source tree is empty: {root}")
    return files


def source_tree_hash(files: dict[str, str]) -> str:
    digest = hashlib.sha256(b"openmind-zip9-source-tree-v1\0")
    for name, file_sha in sorted(files.items()):
        digest.update(name.encode("utf-8") + b"\0")
        digest.update(bytes.fromhex(file_sha))
    return digest.hexdigest()


def cargo_registry_checksum(lock: Path, crate: str, version: str) -> str:
    items = [
        p for p in tomllib.loads(lock.read_text(encoding="utf-8"))["package"]
        if p["name"] == crate and p["version"] == version
    ]
    if len(items) != 1:
        raise RuntimeError(f"Expected exactly one {crate}@{version} lockfile package")
    checksum = items[0].get("checksum", "")
    if not re.fullmatch(r"[a-f0-9]{64}", checksum):
        raise RuntimeError(f"Missing verified registry checksum for {crate}@{version}")
    return checksum


def source_patch_evidence(
    crate: str, version: str, original: Path, patched: Path, lock: Path,
) -> tuple[dict, str]:
    """Report exactly reviewed ZIP edits and fail if any other source file changed."""
    before = source_file_hashes(original)
    after = source_file_hashes(patched)
    if before.keys() != after.keys():
        raise RuntimeError(f"{crate}: upstream source file set unexpectedly changed")
    modified = sorted(name for name in before if before[name] != after[name])
    expected = sorted(SOURCE_PATCH_FILES[crate])
    if modified != expected:
        raise RuntimeError(f"{crate}: changed files {modified}, expected {expected}")
    changes = []
    diffs = []
    for name in modified:
        before_bytes = (original / name).read_bytes()
        after_bytes = (patched / name).read_bytes()
        old_lines = before_bytes.decode("utf-8").splitlines(keepends=True)
        new_lines = after_bytes.decode("utf-8").splitlines(keepends=True)
        diff = "".join(difflib.unified_diff(
            old_lines, new_lines, fromfile=f"a/{crate}/{name}",
            tofile=f"b/{crate}/{name}",
        ))
        if not diff:
            raise RuntimeError(f"{crate}: changed file missing unified diff: {name}")
        diffs.append(diff)
        changes.append({
            "path": name,
            "originalSha256": before[name],
            "patchedSha256": after[name],
        })
    report = {
        "crate": crate,
        "version": version,
        "registryChecksumFromCargoLock": cargo_registry_checksum(lock, crate, version),
        "originalTreeSha256": source_tree_hash(before),
        "patchedTreeSha256": source_tree_hash(after),
        "sourceFileCount": len(before),
        "changes": changes,
    }
    return report, "\n".join(diffs)


def write_patch_evidence(directory: Path, reports: list[dict], patches: dict[str, str]) -> None:
    """Export auditable, deterministic metadata and diffs, never unreviewed crate binaries."""
    directory.mkdir(parents=True, exist_ok=True)
    report = {
        "schemaVersion": 1,
        "purpose": "ZIP 9 candidate source provenance; not production remediation",
        "zipVersion": ZIP_TARGET,
        "cargoRegistrySource": "cargo fetch --locked; checksums pinned in src-tauri/Cargo.lock",
        "packages": sorted(reports, key=lambda r: r["crate"]),
    }
    evidence = {
        "zip9-source-provenance.json": json.dumps(report, sort_keys=True, indent=2) + "\n",
        **{f"zip9-{name}.patch": value for name, value in patches.items()},
    }
    for name, contents in evidence.items():
        with (directory / name).open("x", encoding="utf-8", newline="\n") as out:
            out.write(contents)

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



def stage_desktop_crate(source: Path, destination: Path) -> tuple[Path, Path]:
    """Copy the desktop crate, excluding build outputs, before Cargo resolution.

    Cargo's update subcommand writes Cargo.lock even when using --config patch.
    A throwaway manifest path prevents interrupts from changing the checked-in
    lockfile, including SIGKILL and system crashes that bypass finally blocks.
    """
    shutil.copytree(
        source, destination, symlinks=True, ignore=shutil.ignore_patterns("target"),
    )
    for name in ("Cargo.toml", "Cargo.lock"):
        original = source / name
        staged = destination / name
        if staged.read_bytes() != original.read_bytes():
            raise RuntimeError(f"Staged {name} did not match the checked-in input")
    return destination / "Cargo.toml", destination / "Cargo.lock"

def cargo_patch_override(crate: str, path: Path) -> str:
    """Emit a valid Cargo CLI TOML path on Windows, macOS, and Linux.

    Native Windows backslashes inside a TOML quoted string are interpreted as
    escapes (for example, \\t or \\U). A forward-slash absolute path and
    JSON/TOML-compatible quoting avoid that ambiguity and handle spaces.
    """
    if crate not in TARGETS:
        raise ValueError(f"Unexpected patched upstream crate: {crate}")
    return f"patch.crates-io.{crate}.path={json.dumps(path.resolve().as_posix())}"


def lock_versions(path: Path, name: str) -> list[str]:
    document = tomllib.loads(path.read_text(encoding="utf-8"))
    return [p["version"] for p in document["package"] if p["name"] == name]


def require_patched_zip_only(path: Path) -> None:
    """Reject missing, duplicated or unexpected ZIP versions in the resolved lock."""
    versions = lock_versions(path, "zip")
    if versions != [ZIP_TARGET]:
        raise RuntimeError(
            f"Expected only zip {ZIP_TARGET} in migrated Cargo.lock, found {versions}"
        )


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

    assert PureWindowsPath("C:/Users/runneradmin/AppData/Local/Temp/tts").as_posix() == (
        "C:/Users/runneradmin/AppData/Local/Temp/tts"
    )
    override = cargo_patch_override("candle-core", ROOT)
    assert override.startswith("patch.crates-io.candle-core.path=")
    assert json.loads(override.split("=", 1)[1]) == ROOT.resolve().as_posix()
    try:
        cargo_patch_override("unexpected-crate", ROOT)
    except ValueError:
        pass
    else:
        raise AssertionError("Unreviewed crate overrides must fail closed")

    # Simulate a resolver rewrite in an isolated working copy, including a
    # local build-output directory that must not be recursively copied.
    with tempfile.TemporaryDirectory(prefix="openmind-zip9-stage-test-") as temp:
        base = Path(temp)
        source = base / "source"
        source.mkdir()
        (source / "Cargo.toml").write_text('[package]\nname = "test"\n')
        (source / "Cargo.lock").write_text("original lock")
        (source / "target").mkdir()
        (source / "target" / "cache").write_text("do not copy")
        manifest, lock = stage_desktop_crate(source, base / "staged")
        assert manifest.read_bytes() == (source / "Cargo.toml").read_bytes()
        assert not (base / "staged" / "target").exists()
        lock.write_text("modified by simulated cargo update")
        assert (source / "Cargo.lock").read_text() == "original lock"

        example = base / "source-files"
        example.mkdir()
        (example / "file.txt").write_text("before\n")
        fingerprints = source_file_hashes(example)
        assert len(fingerprints) == 1
        assert source_tree_hash(fingerprints) == source_tree_hash(fingerprints)
        (example / "file.txt").write_text("after\n")
        assert source_tree_hash(fingerprints) != source_tree_hash(source_file_hashes(example))
        output = base / "evidence"
        write_patch_evidence(output, [{"crate": "test"}], {"test": "diff --fixture\n"})
        assert (output / "zip9-test.patch").read_text() == "diff --fixture\n"
        assert json.loads((output / "zip9-source-provenance.json").read_text())["schemaVersion"] == 1
        try:
            write_patch_evidence(output, [], {})
        except FileExistsError:
            pass
        else:
            raise AssertionError("Existing evidence must never be silently overwritten")

        migrated_lock = base / "migrated.lock"
        migrated_lock.write_text(
            '[[package]]\nname = "zip"\nversion = "9.0.4"\n',
            encoding="utf-8",
        )
        require_patched_zip_only(migrated_lock)
        for bad_version in ("7.2.0", "8.6.0", "9.0.3"):
            migrated_lock.write_text(
                '[[package]]\nname = "zip"\nversion = "' + bad_version + '"\n',
                encoding="utf-8",
            )
            try:
                require_patched_zip_only(migrated_lock)
            except RuntimeError:
                pass
            else:
                raise AssertionError(f"Unpatched ZIP {bad_version} passed validation")
    print("ZIP 9 probe self-test passed")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument(
        "--audit", action="store_true",
        help="Run unignored cargo audit on the temporary resolved lockfile",
    )
    parser.add_argument("--cargo-home", type=Path)
    parser.add_argument("--evidence-dir", type=Path, help="Output deterministic patched source provenance and diffs")
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
        scratch_manifest, scratch_lock = stage_desktop_crate(
            ROOT / "src-tauri", overlay / "src-tauri",
        )
        patches = []
        reports: list[dict] = []
        evidence_patches: dict[str, str] = {}
        for crate, version in TARGETS.items():
            src = registry_source(crate, version, cargo_home)
            dst = overlay / f"{crate}-{version}"
            shutil.copytree(src, dst)
            cargo_toml = dst / "Cargo.toml"
            cargo_toml.write_text(replace_zip_requirement(cargo_toml.read_text(encoding="utf-8")), encoding="utf-8")
            adapt_upstream_zip9_api(crate, dst)
            report, diff = source_patch_evidence(crate, version, src, dst, LOCK)
            reports.append(report)
            evidence_patches[crate] = diff
            patches.extend(["--config", cargo_patch_override(crate, dst)])
            print(f"Locally substituted {crate}@{version} zip requirement with {ZIP_TARGET}")
        evidence_dir = args.evidence_dir or (
            Path(os.environ["ZIP9_EVIDENCE_DIR"]) if os.environ.get("ZIP9_EVIDENCE_DIR") else None
        )
        if evidence_dir is not None:
            write_patch_evidence(evidence_dir, reports, evidence_patches)
            print(f"Written reviewable source provenance and patches into {evidence_dir}")

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
            '[package]\nname = "openmind-zip9-readers"\nversion = "0.0.0"\nedition = "2021"\n'
            '[dependencies]\ncandle-core = { path = "../candle-core-0.10.2" }\n'
            'zip = "=9.0.4"\n',
            encoding="utf-8",
        )

        # No Cargo update/check below refers to the checked-in manifest or lock.
        # Reuse the CI cache directory, rather than copying gigabytes of target.
        testing_env = os.environ.copy()
        testing_env["CARGO_TARGET_DIR"] = str(ROOT / "src-tauri" / "target")
        # Snapshot both inputs to detect unexpected external modifications.
        original_manifest = MANIFEST.read_bytes()
        original_lock = LOCK.read_bytes()
        try:
            run([
                "cargo", *patches, "update", "--manifest-path",
                str(scratch_manifest), "-p", "any-tts", "-p", "candle-core",
            ], env=testing_env)
            require_patched_zip_only(scratch_lock)
            if args.audit:
                # The audit uses the temporary ZIP 9-patched dependency graph.
                # Do not suppress RustSec advisories or audit production's
                # known-vulnerable lockfile under this flag.
                run(["cargo", "audit", "--file", str(scratch_lock)], env=testing_env)
            run([
                "cargo", *patches, "check", "--manifest-path",
                str(scratch_manifest), "--lib",
            ], env=testing_env)
            run([
                "cargo", "test", "--manifest-path", str(trial / "Cargo.toml"),
                "--test", "zip9_reader_regressions", "--", "--nocapture",
            ], env=testing_env)
            require_patched_zip_only(trial / "Cargo.lock")
            print("ZIP 9 compilation and synthetic NPZ/PTH regression probe PASSED.")
            if args.audit:
                print("Unignored RustSec audit of the temporary ZIP 9 lockfile PASSED.")
            print("Real pretrained model and platform-specific TTS validation are still required.")
        finally:
            if MANIFEST.read_bytes() != original_manifest or LOCK.read_bytes() != original_lock:
                raise RuntimeError(
                    "Checked-in Cargo inputs unexpectedly changed while running the isolated probe"
                )


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"ZIP 9 compatibility probe FAILED: {exc}", file=sys.stderr)
        sys.exit(1)
