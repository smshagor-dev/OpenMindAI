# Confined rendered artifacts: partial #394

PDF/DOCX, image/video/voice and soundscape generation now reserve a UUID child
through a root directory capability and retain that child handle until publication.
Renderers receive a filename with its original extension in a private OS temporary
directory, not a portable-root pathname. After the existing signature check, the
same opened source file is streamed into an exclusive `.pending-UUID` under the
held destination, synced, closed and renamed there. Cleanup also uses that handle.
The private staging directory is removed on normal drop, including renderer errors.
Artifact metadata continues to store the portable relative destination.

This also removes ambient create_dir_all from ArtifactManager reservation. Text
artifacts retain their existing write_new_relative publication flow. No frontend,
model protocol, artifact schema or signature policy changes are included.

## Reproduction and validation

An extracted unchanged-main writer fixture renames generated/ after reservation,
substitutes an external link and creates the expected UUID directory outside the
root. The old renderer path and commit write the final PDF outside the root; the
containment assertion fails. The new production-module tests exercise the same
parent replacement plus root pathname replacement without outside writes.

Ten actual-module Linux tests pass, including stable external links, retained
parent/root handles, traversal rejection, final/staging symlink rejection, wrong
signatures, cleanup, collision guards and 48 concurrent requests across five
formats. Minimal format headers test byte publication, not document validity.
Focused Clippy with -D warnings, application/harness rustfmt, locked root metadata
and git diff --check pass. Dedicated Linux/macOS/Windows harness CI is added;
Windows uses real directory junction fixtures. Native Windows/macOS and full
Tauri integration are pending CI. No actual renderer/model, removable-drive or
physical desktop test was run locally; Linux GUI build dependencies are absent.

## Remaining boundaries

Do not close #394. The configured root and ancestors must remain trusted until
initial capability acquisition; the manager still lacks a lifetime root handle.
OS temporary-directory ancestry and the same-user renderer process are trusted.
A process that can rename an acquired directory object itself can relocate it;
this is not isolation from an equally privileged malicious OS user. Other writers,
cleanup paths and later pathname-based reads still require review.

The final existence check followed by rename is not atomic no-clobber against a
same-user race. File publication and SQLite status updates are not a joint
transaction (#353); a crash can leave pending/orphaned files or generating rows.
Cancellation while a child process survives may prevent temporary cleanup on
Windows. Staging plus streaming needs extra disk space and incurs a copy; there
is no new storage quota or bounded render-output size. Unix directory sync is
attempted after publication; Windows directory durability and removable-filesystem
power-loss behavior are not guaranteed. Signature checks are not complete format
validation (#347). Publication does not make a changed nominal root path readable.

Paused checkpoint: code and tests are committed/published; no branch writer
remains active after this run. Resume only after checking newer branch activity.

Native CI follow-up: first-head Linux/macOS focused renderer jobs passed. Windows
failed because the test demanded rename of a directory held open without delete
sharing (ERROR_ACCESS_DENIED/ERROR_SHARING_VIOLATION). The revised fixture asserts
those exact OS denials, then still tests successful publication/rollback through
the held original directory; where rename succeeds it substitutes the outside
junction and checks the moved directory. It does not claim Windows performed a
replacement when the OS prevented it. The same fixture repair covers three backup
tests inherited from #507. The full Linux suite also exposed maintenance's fixture
dropping its TempDir before backup; it now retains the guard through each test.
These repairs need fresh native CI. Focused renderer/backup Linux suites each pass
10 tests and strict Clippy after the repair; full local maintenance is not compiled.
