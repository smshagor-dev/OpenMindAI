#!/usr/bin/env python3
"""Fail-closed source integrity gate for the *committed* ZIP 9 vendor snapshot.

The source SHA-256 values are derived from the Cargo-verified ZIP 9 candidate
artifact. The any-tts crate contains a non-source macOS .DS_Store file; only
that file is intentionally excluded from Git distribution and this checksum.
"""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import tempfile

ROOT = Path(__file__).resolve().parents[1]
EXPECTED = {
    "any-tts-0.1.3": (88, "51cfa35cb87dd4511b40649a58dc44c4fbf98f555ced82f4bd8dea1a0738f5a1"),
    "candle-core-0.10.2": (106, "c6d4d2ad38c187adfe63cd8268740d9be5a342d143d9a0a0d3e64affd0b18e45"),
}
REQUIRED_TEXT = (
    "any-tts 0.1.3", "candle-core 0.10.2", "Apache-2.0",
    "Rheosoph/any-tts", "ZIP 9",
)


def source_hashes(root: Path) -> dict[str, str]:
    if not root.is_dir() or root.is_symlink():
        raise ValueError(f"Vendored source directory unavailable or linked: {root}")
    hashes: dict[str, str] = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Vendored file symlink rejected: {path}")
        if not path.is_file():
            continue
        key = path.relative_to(root).as_posix()
        if path.name == ".DS_Store":
            raise ValueError(f"Unexpected macOS metadata in distributed source: {path}")
        hashes[key] = hashlib.sha256(path.read_bytes()).hexdigest()
    if not hashes:
        raise ValueError(f"Vendored source tree is empty: {root}")
    return hashes


def tree_digest(hashes: dict[str, str]) -> str:
    digest = hashlib.sha256(b"openmind-zip9-source-tree-v1\0")
    for name, file_hash in sorted(hashes.items()):
        digest.update(name.encode("utf-8") + b"\0")
        digest.update(bytes.fromhex(file_hash))
    return digest.hexdigest()


def verify(root: Path) -> None:
    for crate, (count, expected) in EXPECTED.items():
        source = root / "vendor" / crate
        hashes = source_hashes(source)
        actual = tree_digest(hashes)
        if len(hashes) != count or actual != expected:
            raise ValueError(
                f"{crate}: vendored source differs from audited package "
                f"(files {len(hashes)}/{count}, tree SHA-256 {actual}/{expected})"
            )
        print(f"{crate}: {count} reviewed files, SHA-256 {actual}")
    apache = root / "vendor" / "LICENSE-APACHE-2.0"
    candle_license = root / "vendor" / "candle-core-0.10.2" / "LICENSE"
    if (not apache.is_file() or not candle_license.is_file()
            or apache.read_bytes() != candle_license.read_bytes()):
        raise ValueError("Bundled Apache-2.0 source license missing or different")
    notices = root / "vendor" / "THIRD_PARTY_NOTICES.md"
    if not notices.is_file() or notices.is_symlink():
        raise ValueError("Required third-party source and modification notice is absent")
    text = notices.read_text(encoding="utf-8")
    if any(needle not in text for needle in REQUIRED_TEXT):
        raise ValueError("Third-party license/attribution notice is incomplete")
    print("ZIP 9 committed production vendor integrity PASSED")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="openmind-zip9-vendor-gate-") as temp:
        sample = Path(temp) / "vendor"
        sample.mkdir()
        (sample / "source.rs").write_bytes(b"original")
        good = source_hashes(sample)
        assert len(good) == 1 and tree_digest(good) == tree_digest(good)
        (sample / "source.rs").write_bytes(b"modified")
        assert tree_digest(good) != tree_digest(source_hashes(sample))
        (sample / ".DS_Store").write_bytes(b"not-source")
        try:
            source_hashes(sample)
        except ValueError:
            pass
        else:
            raise AssertionError("Unreviewed OS metadata must be rejected")
        (sample / ".DS_Store").unlink()
    print("ZIP 9 vendor integrity self-test PASSED")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        verify(ROOT)
