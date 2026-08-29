export type ConnectedProvider =
  | "google"
  | "github"
  | "microsoft"
  | "slack"
  | "notion"
  | "dropbox"
  | "mcp";

export interface GoogleWorkspaceStatus {
  configured: boolean;
  connected: boolean;
  clientId: string | null;
  email: string | null;
  expiresAt: number | null;
  scopes: string[];
}

export interface IntegrationStatus {
  provider: Exclude<ConnectedProvider, "google" | "github">;
  configured: boolean;
  connected: boolean;
  hasSecret: boolean;
  accountLabel: string | null;
  expiresAt: number | null;
  scopes: string[];
  config: Record<string, unknown>;
}

export interface ConnectedActionDefinition {
  provider: ConnectedProvider;
  action: string;
  label: string;
  description: string;
  mutating: boolean;
  example: Record<string, unknown>;
}

const action = (
  provider: ConnectedProvider,
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
): ConnectedActionDefinition => ({
  provider,
  action: actionName,
  label,
  description,
  mutating,
  example,
});

const google = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("google", actionName, label, description, mutating, example);

const github = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("github", actionName, label, description, mutating, example);

const microsoft = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("microsoft", actionName, label, description, mutating, example);

const slack = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("slack", actionName, label, description, mutating, example);

const notion = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("notion", actionName, label, description, mutating, example);

const dropbox = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("dropbox", actionName, label, description, mutating, example);

const mcp = (
  actionName: string,
  label: string,
  description: string,
  mutating: boolean,
  example: Record<string, unknown>,
) => action("mcp", actionName, label, description, mutating, example);

export const CONNECTED_ACTIONS: ConnectedActionDefinition[] = [
  google("gmail.search", "Gmail · Search", "Search the connected mailbox.", false, { query: "newer_than:7d", maxResults: 25 }),
  google("gmail.get", "Gmail · Read message", "Read a Gmail message including headers and body payload.", false, { messageId: "MESSAGE_ID" }),
  google("gmail.thread", "Gmail · Read thread", "Read a Gmail conversation thread with its messages.", false, { threadId: "THREAD_ID" }),
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
  github("file.delete", "GitHub · Delete file", "Delete one file and commit the deletion.", true, { repo: "owner/repo", path: "old.txt", branch: "main", message: "chore: remove old file" }),
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

  microsoft("account.get", "Microsoft 365 · Account", "Read the connected Microsoft account profile.", false, {}),
  microsoft("mail.list", "Outlook · Mail", "List recent Outlook messages.", false, { top: 25 }),
  microsoft("mail.get", "Outlook · Read message", "Read one Outlook message.", false, { messageId: "MESSAGE_ID" }),
  microsoft("mail.send", "Outlook · Send", "Send an email through Microsoft Graph.", true, { to: "person@example.com", subject: "Hello", body: "Message body" }),
  microsoft("mail.reply", "Outlook · Reply", "Reply to an Outlook message.", true, { messageId: "MESSAGE_ID", body: "Reply body" }),
  microsoft("mail.delete", "Outlook · Delete", "Delete an Outlook message.", true, { messageId: "MESSAGE_ID" }),
  microsoft("drive.list", "OneDrive · List/search", "List OneDrive root items or search by name/text.", false, { query: "report" }),
  microsoft("drive.get", "OneDrive · File metadata", "Read OneDrive item metadata.", false, { itemId: "ITEM_ID" }),
  microsoft("drive.download", "OneDrive · Download", "Read a OneDrive file as base64 (interactive limit 8 MB).", false, { itemId: "ITEM_ID" }),
  microsoft("drive.upload", "OneDrive · Upload", "Upload or replace a small file by path.", true, { path: "OpenMindAI/notes.txt", content: "Hello from OpenMindAI" }),
  microsoft("drive.delete", "OneDrive · Delete", "Delete a OneDrive item.", true, { itemId: "ITEM_ID" }),
  microsoft("calendar.events", "Outlook Calendar · Events", "List events in a date/time window.", false, { startDateTime: "2026-08-29T00:00:00Z", endDateTime: "2026-09-05T00:00:00Z" }),
  microsoft("calendar.create", "Outlook Calendar · Create", "Create an event.", true, { event: { subject: "OpenMindAI review", start: { dateTime: "2026-08-30T10:00:00", timeZone: "UTC" }, end: { dateTime: "2026-08-30T11:00:00", timeZone: "UTC" } } }),
  microsoft("calendar.update", "Outlook Calendar · Update", "Patch an event.", true, { eventId: "EVENT_ID", event: { subject: "Updated title" } }),
  microsoft("calendar.delete", "Outlook Calendar · Delete", "Delete an event.", true, { eventId: "EVENT_ID" }),
  microsoft("contacts.list", "Outlook Contacts · List", "List contacts.", false, { top: 100 }),
  microsoft("contacts.search", "Outlook Contacts · Search", "Search loaded contacts by name/email/phone/company.", false, { query: "Alice", top: 200 }),

  slack("account.get", "Slack · Account", "Validate the connected Slack workspace/account.", false, {}),
  slack("channels.list", "Slack · Conversations", "List channels, groups and conversations visible to the token.", false, { limit: 100 }),
  slack("channels.history", "Slack · History", "Read recent messages from a conversation.", false, { channel: "C0123456789", limit: 50 }),
  slack("channels.replies", "Slack · Thread", "Read replies in a Slack thread.", false, { channel: "C0123456789", ts: "1234567890.123456", limit: 50 }),
  slack("search.messages", "Slack · Search messages", "Search messages when the connected user token has search:read.", false, { query: "deployment failed", count: 50 }),
  slack("users.list", "Slack · Users", "List workspace members visible to the app.", false, {}),
  slack("chat.send", "Slack · Send message", "Post a message or thread reply.", true, { channel: "C0123456789", text: "Hello from OpenMindAI" }),
  slack("chat.update", "Slack · Edit message", "Edit a message posted by the connected app/user.", true, { channel: "C0123456789", ts: "1234567890.123456", text: "Updated message" }),
  slack("chat.delete", "Slack · Delete message", "Delete a message the connected identity can delete.", true, { channel: "C0123456789", ts: "1234567890.123456" }),
  slack("reactions.add", "Slack · Add reaction", "Add an emoji reaction.", true, { channel: "C0123456789", timestamp: "1234567890.123456", name: "white_check_mark" }),
  slack("reactions.remove", "Slack · Remove reaction", "Remove an emoji reaction.", true, { channel: "C0123456789", timestamp: "1234567890.123456", name: "white_check_mark" }),

  notion("account.get", "Notion · Connection", "Read the connected Notion bot/user identity.", false, {}),
  notion("search", "Notion · Search", "Search pages and data sources shared with the connection.", false, { query: "Roadmap", pageSize: 50 }),
  notion("page.get", "Notion · Read page", "Read page properties.", false, { pageId: "PAGE_ID" }),
  notion("block.children", "Notion · Read blocks", "Read child blocks for a page/block.", false, { blockId: "BLOCK_ID", pageSize: 100 }),
  notion("data_source.query", "Notion · Query data source", "Query a Notion data source with filters/sorts.", false, { dataSourceId: "DATA_SOURCE_ID", query: {} }),
  notion("comment.list", "Notion · Comments", "List comments for a block/page.", false, { blockId: "BLOCK_ID" }),
  notion("page.create", "Notion · Create page", "Create a page using a raw Notion page request object.", true, { page: { parent: { page_id: "PARENT_PAGE_ID" }, properties: { title: { title: [{ text: { content: "OpenMindAI page" } }] } } } }),
  notion("page.update", "Notion · Update page", "Patch page properties/icon/cover/archive state.", true, { pageId: "PAGE_ID", patch: { archived: false } }),
  notion("block.append", "Notion · Append blocks", "Append content blocks to a page/block.", true, { blockId: "BLOCK_ID", children: [{ object: "block", type: "paragraph", paragraph: { rich_text: [{ type: "text", text: { content: "Added by OpenMindAI" } }] } }] }),
  notion("comment.create", "Notion · Add comment", "Create a page or discussion comment.", true, { comment: { parent: { page_id: "PAGE_ID" }, rich_text: [{ type: "text", text: { content: "Comment from OpenMindAI" } }] } }),

  dropbox("account.get", "Dropbox · Account", "Read the connected Dropbox account profile.", false, {}),
  dropbox("files.list", "Dropbox · List folder", "List files/folders from a Dropbox path.", false, { path: "", recursive: false, limit: 100 }),
  dropbox("files.search", "Dropbox · Search", "Search files/folders.", false, { query: "report", path: "" }),
  dropbox("files.download", "Dropbox · Download", "Read a file as base64 (interactive limit 8 MB).", false, { path: "/report.pdf" }),
  dropbox("files.upload", "Dropbox · Upload", "Upload or overwrite a small file.", true, { path: "/OpenMindAI/notes.txt", content: "Hello from OpenMindAI", mode: "overwrite" }),
  dropbox("files.move", "Dropbox · Move/rename", "Move or rename a file/folder.", true, { fromPath: "/old.txt", toPath: "/archive/old.txt" }),
  dropbox("files.delete", "Dropbox · Delete", "Delete a file/folder.", true, { path: "/old.txt" }),

  mcp("tools.list", "MCP · List tools", "Discover tools exposed by the configured MCP server.", false, {}),
  mcp("tools.call", "MCP · Call tool", "Call an arbitrary remote MCP tool. OpenMindAI always requires explicit approval for MCP tool calls.", true, { name: "tool_name", arguments: {} }),
  mcp("resources.list", "MCP · List resources", "List resources exposed by the MCP server.", false, {}),
  mcp("resources.read", "MCP · Read resource", "Read an MCP resource URI.", false, { uri: "resource://example" }),
  mcp("prompts.list", "MCP · List prompts", "List server-provided prompt templates.", false, {}),
  mcp("prompts.get", "MCP · Get prompt", "Resolve a server-provided prompt template.", false, { name: "prompt_name", arguments: {} }),
];
