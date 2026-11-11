# Model inventory refresh

Rediscovery updates size, inferred/manifest family and quantization, context
length and capabilities on an existing row. IDs, creation time, saved enabled
preference and backend preference remain intact. Existing rows now receive the
same basic GGUF validation as newly registered files before metadata updates.

List reads recheck scoped file availability, basic GGUF header/size and required
package dependencies. Missing paths project as NotInstalled; invalid/unreadable
files or incomplete packages project as Failed. Such returned records are disabled
without changing the saved enabled preference. Restoring a valid file makes its
original row available again; intentionally disabled models stay disabled. Even
before rediscovery, a valid replacement's actual size is returned to launch
planning. Explicitly failed manifest verification cannot activate a package.

Missing/incomplete rows are retained rather than deleted. Conversation model IDs
and user preferences survive temporary drive/file loss. The selector excludes
unavailable/disabled rows, labels an unavailable saved selection, and permits a
restored model again. Backend chat routing cannot let an unavailable saved ID
supersede an enabled fallback. The catalog ignores unavailable registry rows when
resolving an installed path. Explicit user deletion remains separate.

These are quick header/path/package-presence checks, not full integrity hashing
or a parse of every GGUF tensor/metadata record. Custom imports retain the existing
header-only trust policy; Unverified is not upgraded to Verified. Manifest-based
attributes are refreshed from the current manifest, with filename inference as
before. Full integrity semantics remain #413. Small filesystem reads are added to
list operations; no file watcher, running-runtime termination or cross-process
filesystem snapshot is added. A file can still change after inspection.

Regression tests use sparse synthetic GGUF-header files and real SQLite. They
cover different-size and changed-manifest replacement, unchanged IDs/preferences,
missing/restored files and conversation linkage, invalid replacements, incomplete
packages, actual launch-planner byte estimates, catalog presence and saved-ID
routing. Real-source before/after tests fail against main's old registry for stale
size and deleted-but-enabled models. A production React selector DOM regression
covers unavailable options and restoration. No real model inference, physical
USB removal or desktop visual interaction was performed locally.
