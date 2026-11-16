# Local dataset context boundaries

Dataset context treats `openmindai-dataset.json` as untrusted input each time it
is used, including manifests copied into the storage folder manually.

The only ambient filesystem open is the configured portable root. Dataset
discovery and text reads then use `cap-std` directory handles. Each selected
dataset keeps its own open directory handle, so manifest paths cannot read a
parent or sibling dataset, or redirect reads outside that dataset through a
symlink. A path being replaced after discovery does not cause a fresh ambient
open at the replacement location.

Manifest paths must be nonempty portable relative paths with `/` separators.
Absolute paths, drive prefixes, backslashes, parent/current-directory segments,
empty segments, control characters, Windows stream syntax and ambiguous trailing
dots/spaces are rejected before scoring or displaying the file list.

Discovery skips symbolic directory links and linked manifests, records visited
canonical relative directories, and enforces depth and entry limits. Stable
symlinks in the final text-file component are skipped. Intermediate symlinks in
an explicitly listed text path may resolve only within that dataset's directory
capability; external targets are refused by the filesystem resolver. The
`datasets` and `openmindai` base directory components must be ordinary directories.

Only regular UTF-8 files are used. Reads check actual file metadata rather than
the manifest's claimed size, and also cap bytes read in case a file grows after
the metadata check. Unix opens are nonblocking so a replacement FIFO cannot
stall the reader during open. Malformed, unreadable or oversized manifests and
unsafe text entries are skipped without preventing retrieval from healthy
datasets. Diagnostic logs contain no file contents.

| Limit | Value |
| --- | --- |
| Manifest bytes | 1 MiB |
| Text bytes per file | 4 MiB |
| Retained paths per manifest | 4,096 |
| Discovery entries per request | 8,192 |
| Maximum discovery depth | 16 |
| Selected datasets | 4 |
| Considered text files per dataset | 6 |

Large datasets may still be installed; context retrieval skips oversized text
files. Split relevant text into smaller files when it should be retrieved.
These controls cover context reads, not the dataset downloader or the general
portable-root API. They are not a sandbox for a hostile program running with the
user's own OS account, and do not distinguish regular files from hard links.

The manifest and lockfile pin `cap-std`/`cap-primitives` 4.0.3. See the
[directory API](https://docs.rs/cap-std/4.0.3/cap_std/fs/struct.Dir.html) for the
relative filesystem model. Regression tests live alongside `dataset_context` and
run in the normal Rust CI on Linux, macOS and Windows. Unix handle-replacement
tests and the Windows junction test are platform-specific. Windows symlink
fixtures explicitly report when Developer Mode/privileges are unavailable.
