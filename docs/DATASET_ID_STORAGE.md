# Dataset ID storage and legacy migration

Dataset installs now live under
`datasets/openmindai/by-id-v1/<SHA-256 of exact UTF-8 dataset ID>/snapshot/main`.
The versioned prefix separates this scheme from legacy sanitized repository
names. IDs remain case-sensitive. Slash/underscore, case and Unicode differences
produce different lowercase hexadecimal keys even on case-insensitive disks.
Key lengths are fixed; absolute portable-root locations are not part of the hash.

Before downloading files, the existing per-root install lock protects a migration
check. A legacy `datasets/openmindai/<sanitized ID>/snapshot/main` directory moves
to the new path only when its bounded regular JSON manifest names that exact
`datasetId`, source `openmindai`, and revision `main`. Folder names alone never
establish identity. Directory rename preserves files, extra files and partials;
there is no copy/delete fallback on failure. Existing new destinations win and
are never combined with or replaced by a legacy directory.

Foreign, missing, unreadable, oversized or malformed manifests leave the legacy
folder untouched. Other legacy revisions are untouched. A pre-existing new
installation plus a legacy copy both remain intact; this fix does not deduplicate
context inventory. There is no attempt to recover files already overwritten by
older ID collisions. Unknown legacy data can still be discovered through its
manifest where valid, but a new install uses the isolated path.

Migration establishes identity, not integrity. Existing checksum/download behavior
still applies to downloaded files, and files without published upstream checksums
remain explicitly unverified. It does not avoid file downloads merely because a
legacy directory moved. Trusted ancestor directories and normal same-filesystem
rename semantics are required; general path-swap protection remains #394. Stable
linked migration destinations outside dataset storage are rejected before any
external directory is created. Physical drive removal, power loss, Windows device
execution and case-insensitive filesystem behavior were not tested locally.

Regression tests cover case-folded key uniqueness for old collision pairs,
Unicode IDs, bounded key lengths, exact manifest ownership, preservation of both
colliding installs, migration failure/ambiguous metadata, existing destination
preservation and a linked external destination. A real-source downloader test
runs two colliding installs against a local synthetic HTTP server and checks
manifest identity, independent payloads and preservation of the legacy owner's
extra file. The same regression fails with the main branch's legacy mapping.
