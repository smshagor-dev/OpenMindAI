# Coding-agent descriptor trust boundary

The localhost coding-agent gateway publishes a fresh per-process bearer token in
`coding-agent-endpoint.json` beside the installation pointer. The VS Code client
continues to use the existing descriptor location and protocol.

Before publishing a token, OpenMindAI checks that its containing directory is a
regular directory owned by the current account. On Unix it repairs the directory
to 0700 and creates/verifies the new file at 0600, independently of the umask.
Linked, multiply linked or foreign-owned descriptor files are rejected. A new
exclusive temporary file is flushed and renamed only after protection succeeds.

On Windows, the built-in Windows PowerShell ACL APIs install a protected DACL
with one FullControl allow rule for the current user SID. Directory inheritance
propagates this rule to children; the file also receives an explicit protected
DACL. The owner, protection flag and complete access-rule list are checked. An
elevated administrator can repair an Administrators-owned directory by assigning
ownership to the current user; other foreign ownership is rejected. Reparse
points and nonregular paths are rejected. No token appears in subprocess
arguments, environment, stdout or stderr. If PowerShell/ACL enforcement is
unavailable, gateway startup fails; it never publishes a permissive token file.

`OPENMINDAI_CODING_AGENT_DESCRIPTOR` must name a file in a **dedicated directory**:
that directory's permissions/ACL are restricted to the current account. Do not
point it at a shared directory, home-directory root or filesystem root. Other
OpenMindAI configuration in the default dedicated directory remains accessible
to the owning account.

This protects against other unprivileged local OS accounts. It does not protect
against malware already running as the same user, administrators/root with
privilege to override ACLs, a compromised OS, or an external filesystem that does
not honor permissions/ACLs. Trusted ancestor directories and normal local
filesystem path semantics are required; broader root-containment races are
tracked separately in #394. The gateway authenticates callers with the token;
this change does not add TLS or expose the listener beyond loopback.

Regression coverage checks secure creation/replacement and cleanup, repair of
permissive Unix directories, link/special-file rejection, and real cross-UID
read denial when the test runner has privilege to drop its child to a separate
UID. Windows tests execute the same ACL-setting and verification path as
production; they do not provision a second Windows account.
