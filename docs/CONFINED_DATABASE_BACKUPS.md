# Confined SQLite backup publication (partial #394)

The maintenance writer now opens the selected root as a cap-std capability and
opens backups/ beneath that root. SQLite VACUUM INTO writes exclusively in a
private tempfile directory, never through a pathname below portable storage.
The completed snapshot is streamed into an exclusively created pending file,
synced, renamed and cleaned through the same held backup directory capability.
Pending names are hidden from the backup listing. Existing naming/collision
retries and Unix 0600 permissions remain. tempfile was promoted from dev-only to
normal dependencies without changing its existing locked version or root lockfile.

Ten Linux tests import actual production backup/error modules. They cover external
symlinks; parent/root replacement after acquiring a capability; final/pending
links; injected streaming-read failure cleanup; SQLite failure; collision retries;
simultaneous/frozen-time snapshots; committed WAL rows and integrity checks. An
extracted unchanged-main writer fixture fails its containment assertion and
creates one snapshot outside the selected root after path validation/replacement.
The new writer's capability fixtures keep the outside directory empty.

Validation: focused cargo test (10 passed), Clippy with -D warnings, harness/app
cargo fmt, locked metadata and diff checks passed. Three-OS dedicated CI added;
Windows fixtures use PowerShell Junction creation. No physical removable-drive,
disk-full/power-cut, second-account attacker or full local Tauri test is claimed.
The maintenance listing wrapper is covered by native application CI, not the
focused harness. Full native CI also has an existing project-row type_complexity
blocker repaired in pending #505 and #506; don't call this branch all-green.
Windows workspace transaction Access Denied failures observed on those PRs are
additional material failures, separate from RustSec and the old junction fixture.

Limitations: #394 stays OPEN. Other PDF/media/path writers and lifetime root
identity are unfinished. Root/ancestors must remain trusted until the root
capability is opened; the OS staging directory/ancestors must remain trusted
while SQLite opens its private path. Holding a directory preserves object identity,
not the directory's global pathname if someone with sufficient permissions moves
it. Rename does not follow a destination link, but a same-user actor creating a
final-name entry between the last check and rename can replace that entry. This
is not an atomic no-clobber/CAS guarantee against actors already able to edit that
same directory. A post-publication directory-sync error may leave a valid snapshot
while reporting uncertain durability. Staging needs OS-temp free space as well as
backup-volume space; a crash can leave pending/private-temp files. Directory sync
is performed on Unix; no new Windows power-loss durability guarantee is claimed.
