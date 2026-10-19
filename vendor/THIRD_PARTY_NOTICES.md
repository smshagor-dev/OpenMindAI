# Third-party source and modification notices

These Rust dependencies are vendored from exact Cargo.lock-pinned crates.io packages.
The upstream source and the ZIP 9 compatibility modifications have been reviewed through
the ZIP 9 candidate source-provenance artifacts. No Kokoro model weights are included.

## any-tts 0.1.3

- Upstream: https://github.com/Rheosoph/any-tts (formerly TM9657/any-tts)
- Original Git revision recorded in crates.io package: c547a85ec5a296b3e6db812a2f4b6d6ef76c173d
- Cargo package license: MIT OR Apache-2.0
- Redistribution choice: Apache-2.0
- Modified: Cargo.toml and src/models/kokoro/model.rs for ZIP 9 APIs
- Preserved upstream legal/community documentation alongside source

## candle-core 0.10.2

- Upstream: https://github.com/huggingface/candle
- Cargo package license: MIT OR Apache-2.0
- Redistribution choice: Apache-2.0
- Modified: Cargo.toml, src/npy.rs, and src/pickle.rs for ZIP 9 APIs
- Original upstream LICENSE remains in vendor/candle-core-0.10.2/LICENSE

The original Apache License 2.0 text is copied to vendor/LICENSE-APACHE-2.0.
The original published any-tts 0.1.3 archive does not include a top-level LICENSE file;
its Cargo manifest declares MIT OR Apache-2.0. Preserve all original notices and
reconfirm attribution and distribution obligations before a production release.

The upstream any-tts registry archive includes a macOS src/.DS_Store metadata file.
It is intentionally omitted from this Git repository and from the source-tree hash
of the 88 distributed upstream files; no executable Rust source is omitted.
The provenance artifact retains the hash of all 89 registry files.

Full source hashes and the exact adapters are documented in the security CI
and docs/zip9-upstream-migration.md. This notice does not imply a completed
real Kokoro speech synthesis or installer acceptance test.
