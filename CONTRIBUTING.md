# Contributing to OpenMindAI

Thanks for taking the time to improve OpenMindAI. Contributions are welcome when they keep the project reliable, local-first, portable, and understandable to maintain.

## Before you start

For a bug, search the existing issues first. For a larger feature or architectural change, open a feature request before investing significant implementation time so the scope can be discussed early.

Security vulnerabilities should not be reported in a public issue. Follow [SECURITY.md](SECURITY.md) instead.

## Development setup

OpenMindAI uses a React/TypeScript frontend and a Rust/Tauri desktop backend.

Requirements:

- Node.js 24
- npm
- Rust stable with Cargo
- Platform dependencies required by Tauri

Clone the repository and install frontend dependencies:

```bash
git clone https://github.com/smshagor-dev/OpenMindAI.git
cd OpenMindAI
npm ci
```

Run the desktop application in development mode:

```bash
npm run tauri dev
```

## Validation

Please run the checks relevant to your change before opening a pull request.

Frontend:

```bash
npm run check:version
npm run lint
npm run build
```

Rust:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-features
```

GitHub Actions repeats these checks on supported runners. A pull request should not be treated as ready while required validation is failing.

## Pull requests

Keep each pull request focused on one problem. A useful pull request explains:

- what changed;
- why the change is needed;
- how it was tested;
- any user-facing, storage, security, updater, runtime, or portability impact.

Avoid unrelated formatting or refactors in the same change. Do not commit generated model weights, local databases, runtime downloads, cache files, logs, secrets, or user data.

When UI behavior changes, include screenshots or a short recording when that makes review materially easier.

## Code expectations

Prefer straightforward code over unnecessary abstraction. Preserve existing security boundaries and keep OS-specific behavior isolated where practical. Error states should be explicit instead of silently falling back to a different storage root, runtime, or destructive action.

Changes that affect persisted data should consider compatibility with existing installations. Changes to model/runtime downloads should preserve integrity verification and resumability where applicable.

## Commit messages

Use short, descriptive commit messages. Conventional-style prefixes such as `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, and `ci:` are encouraged but not mandatory.

## License

By contributing, you agree that your contribution may be distributed under the repository's [Apache License 2.0](LICENSE).
