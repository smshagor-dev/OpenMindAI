# Document export workers: partial #347

The PDF/DOCX command asynchronously reserves a single process-wide render slot
before creating metadata or staging files. A cancelled queued call therefore has
no artifact to strand. Generation, format validation, publication and SQLite
completion execute on Tokio's blocking pool, not an async executor thread. No DB
mutex crosses the slot wait or render/validation step. The worker owns its permit
and completion even if the IPC caller stops waiting. Rendering panics become generic
failed outcomes; default panic hooks may still log panic details. Notification
failures are advisory and cannot change a completed export into a failed one.

Finalization checks registration inside an IMMEDIATE SQLite transaction. If the
chat/artifact was deleted during rendering, it discards only the final output this
StagedOutput actually published, through its held directory capability. A collision
is never removed. Unknown DB errors do not authorize deleting user files. This is
not a joint filesystem/SQLite crash transaction; DB failure/process exit may still
leave generating metadata, and broader durability limits remain.

Eight actual scheduling-module Linux tests passed: single-thread runtime progress
while a worker blocks, serialization, queued cancellation, detached running worker
slot ownership, errors, panic handling and completion. An inline-dispatch control
fails the same progress assertion after its blocking wait, reproducing the old
architecture's runtime stall. Seventeen actual artifact-module tests passed,
including owner-only publication discard/collision preservation. Strict focused
Clippy, app formatting, locked metadata and diff checks passed. Native tests added
for real PDF/DOCX generators/publication on the worker and deleted-conversation
SQLite finalization; those full app/Tauri tests are NOT locally run. Three-OS
scheduling CI is added. No physical desktop, visual/Word/page-layout or live model
smoke was performed. #347 remains open for those checks.

One active export per process intentionally limits CPU/memory pressure. Pending
requests still retain input strings; no new queue/input quota is added. Running
blocking work cannot be forcibly cancelled; an abandoned caller's export completes.
Slow DB/OS I/O, CPU saturation and process shutdown can still affect responsiveness;
this proves executor isolation, not latency guarantees or instant export. No render
progress/cancel UI or complete orphan recovery is added. Root/user-mutation limits
from #394 and known RustSec/Windows junction failures remain.

The native-feature needless_bool repair in native_chat_router.rs is identical to
that published on #514, with no eligibility behavior change or lint suppression.
It fixes an additional feature-CI blocker, distinct from RustSec/junction failures.
No schema, root dependency or frontend changes.

Paused checkpoint: code/tests committed/published; no writer active after this run.
Check newer activity/current native checks before resuming or merging.
