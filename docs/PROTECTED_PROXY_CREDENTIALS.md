# Protected manual proxy preferences

Authenticated proxy userinfo is stored through the existing OS credential backend
(Windows Credential Manager, macOS Keychain, Linux secret-service helper). SQLite
and returned preferences contain only a canonical HTTP(S) root endpoint and an
opaque UUID reference. A new immutable slot is written before the sanitized JSON
is committed, so a failed DB commit cannot replace the previous credential. A
keyring failure never falls back to writing a new plaintext preference.

Legacy values migrate before other preference updates. The narrow SQLite update
preserves unknown fields and compares the previous proxy URL/reference to avoid
overwriting a concurrent edit. secure_delete is enabled and WAL checkpoint/truncate
is attempted. A single-connection fixture verifies live DB and VACUUM snapshot
bytes no longer contain the synthetic password. This is not a guarantee of physical
erasure from existing backups, external WAL snapshots or filesystem history;
rotate previously exposed credentials and remove old backups as appropriate.

The input is masked. Unrelated saves retain the opaque reference; editing the
address or using Stop using saved credentials unlinks it on the next save. Unused
keyring slots are retained (also after aborted saves); they may be removed with OS
credential management. They are not included in portable DB backups. Moving to a
different OS account/machine requires re-entering credentials.

Only manual mode retrieves the referenced secret. If retrieval fails at startup,
remote clients are pointed to a non-forwarding loopback port-zero proxy instead
of silently using direct internet. Local-host bypass remains. Explicitly selecting
Direct/System does not require unlocking a dormant manual credential. The existing
updater bridge still returns the runtime proxy URL to its frontend transport when
requested; it is a purposeful authenticated updater IPC exception, not preference
serialization. This patch does not rewrite system/environment proxy handling or
all third-party transport error formatting.

Validation: seven actual production-module Rust tests, including mock keyring
failures/immutable replacement/reference tampering and real SQLite migration/CAS/
backup tests; focused Clippy with -D warnings and Rust format checks passed. All
127 frontend tests, including actual SettingsDialog masking/unlink-save assertions,
passed. Lint: zero errors and two existing StudioHub hook warnings. TypeScript/Vite
build passed. No real OS vault service, authenticated proxy server, physical
Windows/macOS dialog or full local Tauri build was exercised. Native CI must
validate full settings/startup integration before merge. Dedicated three-OS
module tests use injected keyring callbacks, not actual OS secret services.

Also includes a mechanical BinaryProjectFileRow type alias fixing the existing
Rust 1.100 Clippy blocker from #504; the same fix is published separately to #505.
Existing RustSec audit and Windows dataset-junction fixture failures remain
separate and must not be represented as all-green.
