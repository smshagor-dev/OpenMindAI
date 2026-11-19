# Archived chats UI (partial #355)

Backend list/restore APIs and database regression tests already exist in main.
This change adds a visible "Archived chats" sidebar entry and a modal for
bounded archived listings, title filtering and in-place Restore. Restore uses
the existing backend command; on success the active conversation list is
updated without recreating messages, files or project links. Failures retain
the archived row for retry. Stale fetch replies are ignored when the dialog
closes and reopens.

Three source-level regression tests check navigation wiring, backend restore
integration/failed action behavior and stale async request guards. Tests must
also be run manually in the desktop shell.

Remaining gaps: backend archive pagination past 200 conversations, server-side
archived text search, keyboard focus trap/return, and real desktop UI tests.
Do not close #355 until these are validated.
