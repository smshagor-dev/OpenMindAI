export type ConnectedProvider = "google" | "github";

export interface GoogleWorkspaceStatus {
  configured: boolean;
  connected: boolean;
  clientId: string | null;
  email: string | null;
  expiresAt: number | null;
  scopes: string[];
}

export interface ConnectedActionDefinition {
  provider: ConnectedProvider;
  action: string;
  label: string;
  description: string;
  mutating: boolean;
  example: Record<string, unknown>;
}

const google = (
  action: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
): ConnectedActionDefinition => ({ provider: "google", action, label, description, mutating, example });

const github = (
  action: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
): ConnectedActionDefinition => ({ provider: "github", action, label, description, mutating, example });

export const CONNECTED_ACTIONS: ConnectedActionDefinition[] = [
  google("gmail.search", "Gmail · Search", "Search the connected mailbox.", false, { query: "newer_than:7d", maxResults: 25 }),
  google("gmail.get", "Gmail · Read message", "Read a Gmail message including headers and body payload.", false, { messageId: "MESSAGE_ID" }),
  google("gmail.labels", "Gmail · List labels", "List mailbox labels.", false, {}),
  google("gmail.send", "Gmail · Send", "Send a new plain-text email.", true, { to: "person@example.com", subject: "Hello", body: "Message body" }),
  google("gmail.reply", "Gmail · Reply", "Reply in the original Gmail thread.", true, { messageId: "MESSAGE_ID", body: "Reply body" }),
  google("gmail.modify", "Gmail · Modify labels", "Add or remove Gmail label IDs.", true, { messageId: "MESSAGE_ID", addLabelIds: ["STARRED"], removeLabelIds: [] }),
  google("gmail.archive", "Gmail · Archive", "Remove a message from Inbox.", true, { messageId: "MESSAGE_ID" }),
  google("gmail.trash", "Gmail · Trash", "Move a Gmail message to Trash.", true, { messageId: "MESSAGE_ID" }),
  google("gmail.untrash", "Gmail · Restore", "Restore a Gmail message from Trash.", true, { messageId: "MESSAGE_ID" }),
  google("drive.list", "Drive · Search/list", "Search or list Drive files visible to the account.", false, { query: "trashed = false", pageSize: 50 }),
  google("drive.get", "Drive · File metadata", "Read Drive file metadata.", false, { fileId: "FILE_ID" }),
  google("drive.download", "Drive · Download", "Read binary file data as base64 (interactive limit 8 MB).", false, { fileId: "FILE_ID" }),
  google("drive.export", "Drive · Export Google file", "Export a Google Docs/Sheets/Slides file.", false, { fileId: "FILE_ID", mimeType: "application/pdf" }),
  google("drive.create", "Drive · Create/upload", "Create metadata, a folder, or upload content.", true, { metadata: { name: "notes.txt" }, mimeType: "text/plain", content: "Hello from OpenMindAI" }),
  google("drive.update", "Drive · Update", "Update Drive metadata and/or file content.", true, { fileId: "FILE_ID", metadata: { name: "renamed.txt" } }),
  google("drive.delete", "Drive · Delete", "Permanently delete a Drive file the account can delete.", true, { fileId: "FILE_ID" }),
  google("calendar.calendars", "Calendar · List calendars", "List calendars available to the account.", false, {}),
  google("calendar.events", "Calendar · Search events", "List/search events in a calendar.", false, { calendarId: "primary", timeMin: "2026-08-29T00:00:00Z", timeMax: "2026-09-05T00:00:00Z" }),
  google("calendar.create", "Calendar · Create event", "Create a calendar event.", true, { calendarId: "primary", event: { summary: "OpenMindAI review", start: { dateTime: "2026-08-30T10:00:00+03:00" }, end: { dateTime: "2026-08-30T11:00:00+03:00" } } }),
  google("calendar.update", "Calendar · Update event", "Patch an existing event.", true, { calendarId: "primary", eventId: "EVENT_ID", event: { summary: "Updated title" } }),
  google("calendar.delete", "Calendar · Delete event", "Delete an event.", true, { calendarId: "primary", eventId: "EVENT_ID" }),
  google("contacts.list", "Contacts · List", "List Google contacts.", false, { pageSize: 100 }),
  google("contacts.search", "Contacts · Search", "Search Google contacts.", false, { query: "Alice" }),
  google("contacts.get", "Contacts · Read", "Read one People API contact resource.", false, { resourceName: "people/c123" }),

  github("account.capabilities", "GitHub · Token capabilities", "Show account and token permission headers available from GitHub.", false, {}),
  github("repo.get", "GitHub · Repository", "Read repository metadata.", false, { repo: "owner/repo" }),
  github("branches.list", "GitHub · Branches", "List repository branches.", false, { repo: "owner/repo" }),
  github("file.get", "GitHub · Read file", "Read a repository file at a branch/ref.", false, { repo: "owner/repo", path: "README.md", ref: "main" }),
  github("commit.get", "GitHub · Commit", "Read a commit by SHA or ref.", false, { repo: "owner/repo", ref: "main" }),
  github("file.put", "GitHub · Create/update file", "Create or replace one repository file and commit it.", true, { repo: "owner/repo", path: "docs/note.md", branch: "main", message: "docs: update note", content: "# Note" }),
  github("file.delete", "GitHub · Delete file", "Delete one file and commit the deletion.", true, { repo: "owner/repo", path: "old.txt", branch: "main", message: "chore: remove old file", sha: "FILE_BLOB_SHA" }),
  github("branch.create", "GitHub · Create branch", "Create a branch from another branch.", true, { repo: "owner/repo", branch: "feature/openmind", sourceRef: "main" }),
  github("commit.multi_file", "GitHub · Multi-file commit", "Commit up to 100 file changes atomically through the Git data API.", true, { repo: "owner/repo", branch: "feature/openmind", message: "feat: update connected files", files: [{ path: "src/example.ts", content: "export const ready = true;\n" }, { path: "old.txt", delete: true }] }),
  github("issue.list", "GitHub · Issues", "List repository issues and PR-backed issue records.", false, { repo: "owner/repo", state: "open" }),
  github("issue.create", "GitHub · Create issue", "Create an issue.", true, { repo: "owner/repo", title: "Issue title", body: "Details", labels: [] }),
  github("issue.comment", "GitHub · Comment", "Add an issue or PR conversation comment.", true, { repo: "owner/repo", number: 1, body: "Comment from OpenMindAI" }),
  github("pr.list", "GitHub · Pull requests", "List pull requests.", false, { repo: "owner/repo", state: "open" }),
  github("pr.get", "GitHub · Read PR", "Read pull request metadata.", false, { repo: "owner/repo", number: 1 }),
  github("pr.create", "GitHub · Create PR", "Open a pull request.", true, { repo: "owner/repo", title: "Feature", head: "feature/openmind", base: "main", body: "Summary", draft: true }),
  github("pr.update", "GitHub · Update PR", "Update title/body/state/base for a pull request.", true, { repo: "owner/repo", number: 1, title: "Updated PR title" }),
  github("pr.merge", "GitHub · Merge PR", "Merge a pull request using merge/squash/rebase.", true, { repo: "owner/repo", number: 1, mergeMethod: "squash" }),
  github("actions.workflows", "GitHub Actions · Workflows", "List Actions workflows.", false, { repo: "owner/repo" }),
  github("actions.runs", "GitHub Actions · Runs", "List workflow runs, optionally for one workflow.", false, { repo: "owner/repo" }),
  github("actions.jobs", "GitHub Actions · Jobs", "List jobs for a workflow run.", false, { repo: "owner/repo", runId: 123456 }),
  github("actions.job_logs", "GitHub Actions · Job logs", "Read bounded job logs.", false, { repo: "owner/repo", jobId: 123456 }),
  github("actions.dispatch", "GitHub Actions · Dispatch", "Run a workflow_dispatch workflow.", true, { repo: "owner/repo", workflowId: "ci.yml", ref: "main", inputs: {} }),
  github("actions.rerun", "GitHub Actions · Rerun", "Rerun a workflow run.", true, { repo: "owner/repo", runId: 123456 }),
  github("actions.cancel", "GitHub Actions · Cancel", "Cancel an active workflow run.", true, { repo: "owner/repo", runId: 123456 }),
  github("actions.workflow.enable", "GitHub Actions · Enable workflow", "Enable a workflow.", true, { repo: "owner/repo", workflowId: "ci.yml" }),
  github("actions.workflow.disable", "GitHub Actions · Disable workflow", "Disable a workflow.", true, { repo: "owner/repo", workflowId: "ci.yml" }),
  github("release.list", "GitHub · Releases", "List releases.", false, { repo: "owner/repo" }),
  github("release.get", "GitHub · Read release", "Read a release by numeric ID.", false, { repo: "owner/repo", releaseId: 123 }),
  github("release.create", "GitHub · Create release", "Create a GitHub release.", true, { repo: "owner/repo", release: { tag_name: "v2.1.0", name: "OpenMindAI v2.1.0", draft: true, prerelease: false } }),
  github("release.update", "GitHub · Update release", "Update a GitHub release.", true, { repo: "owner/repo", releaseId: 123, release: { draft: false } }),
  github("release.delete", "GitHub · Delete release", "Delete a GitHub release object.", true, { repo: "owner/repo", releaseId: 123 }),
  github("tag.create", "GitHub · Create tag ref", "Create a lightweight tag ref at a SHA or branch head.", true, { repo: "owner/repo", tag: "v2.1.0", ref: "main" }),
];
