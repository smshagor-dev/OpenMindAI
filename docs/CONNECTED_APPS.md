# Connected Apps

OpenMindAI is local-first. Connected apps are optional and require internet only when a remote action is used. Local chat, local models, conversation history, projects, artifacts, and maintenance features continue to work without any connected account.

## Security model

- OAuth access tokens, refresh tokens, PATs, integration tokens, client secrets, and MCP bearer tokens are stored in the operating-system credential store.
  - Windows: Credential Manager
  - macOS: Keychain
  - Linux: Secret Service (`secret-tool`)
- Only non-secret connector metadata (for example client ID, redirect URI, server URL, account label, expiry time and granted-scope names) is stored in the OpenMindAI SQLite database.
- Remote write/destructive actions require explicit approval in Connected Work. The backend enforces the approval independently of the UI.
- Interactive connector responses are size-bounded and requests are timeout-bounded.
- Use least-privilege permissions and restrict tokens to only the workspaces/repositories/files the user intends OpenMindAI to access.

## Google Workspace

Configure a Google Cloud Desktop OAuth client and enable the Gmail, Drive, Calendar and People APIs. Save the OAuth client ID and secret in **Settings → Connections**, then choose **Connect Google account**.

Supported areas include Gmail search/read/thread/send/reply/labels/archive/trash, Drive list/read/download/export/create/update/delete, Calendar read/create/update/delete, and Contacts list/search/read.

Google OAuth uses Authorization Code + PKCE + state validation and a loopback callback.

## GitHub

Connect a fine-grained personal access token in **Settings → Connections**. Restrict it to the intended repositories and grant only the required permissions, such as Contents, Pull requests, Issues, Actions/Workflows and Releases.

Supported areas include repository/branch/file/commit reads, file and multi-file commits, branches, issues/comments, pull requests, workflow runs/jobs/logs/dispatch/rerun/cancel, workflow-file changes, tags and releases.

## Microsoft 365

Create an application registration in Microsoft Entra and configure it as a **Mobile and desktop application / public client**.

Use this exact redirect URI unless you intentionally change the corresponding OpenMindAI setting:

```text
http://localhost:17894/oauth/microsoft
```

Recommended delegated permissions for the built-in action set:

```text
User.Read
Mail.ReadWrite
Mail.Send
Files.ReadWrite
Calendars.ReadWrite
Contacts.Read
offline_access
openid
profile
```

OpenMindAI uses Authorization Code + PKCE. A client secret is not required for the public desktop-client flow. The tenant defaults to `common`; organizations can replace it with a tenant ID/domain when required by policy.

Supported areas: Outlook Mail, OneDrive, Outlook Calendar and Contacts.

## Slack

Create a Slack app, add this OAuth redirect URI, configure scopes, and install the app to the target workspace:

```text
http://localhost:17895/oauth/slack
```

Default bot scopes requested by OpenMindAI:

```text
channels:read
channels:history
groups:read
groups:history
im:read
im:history
mpim:read
mpim:history
chat:write
reactions:write
users:read
```

Message search uses a user token with:

```text
search:read
```

The scope fields are editable in Settings so a workspace can reduce them. OAuth requires the Slack app Client ID and Client Secret. For private/internal testing, an existing bot/user token can be connected directly instead.

Supported areas: channels/conversations, history, threads, users, message search, send/edit/delete messages, and reactions.

## Notion

For multi-user OAuth, create a public Notion integration/connection and configure:

```text
http://localhost:17896/oauth/notion
```

Save the Client ID and Client Secret in **Settings → Connections**. For a private workspace, an internal integration token can be connected directly instead.

Notion only exposes pages/data sources that have been shared with the connection. OpenMindAI uses the `2026-03-11` API version for this connector.

Supported areas: search, page properties, block children, data-source queries, comments, page creation/update and block append.

## Dropbox

Create a Dropbox app and configure this redirect URI:

```text
http://localhost:17897/oauth/dropbox
```

OpenMindAI uses Authorization Code + PKCE and requests an offline refresh token when Dropbox returns one. Configure only the Dropbox file/account scopes needed for the chosen app access model. A generated access token can also be connected directly for private/testing use.

Supported areas: account, folder listing, file search, download, upload, move/rename and delete.

Interactive download/upload payloads are limited to 8 MB. Large-file transfer sessions can be added separately without weakening the interactive safety limit.

## MCP servers

MCP makes the connector ecosystem extensible without adding a bespoke provider module for every service. Configure a Streamable HTTP endpoint in **Settings → Connections**.

Remote endpoints must use HTTPS. Plain HTTP is accepted only for `localhost`, `127.0.0.1` or `::1`. An optional bearer token is stored in the OS credential store.

OpenMindAI supports:

- `tools/list`
- `tools/call`
- `resources/list`
- `resources/read`
- `prompts/list`
- `prompts/get`

Every generic `tools/call` is treated as a potentially mutating remote operation and requires explicit user approval. This conservative default prevents an unknown MCP tool from silently changing remote data.

MCP can be used to extend OpenMindAI to services such as project-management systems, CRMs, databases, knowledge bases, developer tools and internal company services whenever a compatible MCP server is available.

## Manual trust gates

CI can verify compilation, linting, unit tests, dependency audits and static security analysis. It cannot prove that a third-party OAuth application has been registered correctly or that a specific organization has granted the required permissions.

Before calling a provider production-ready for a deployment, perform a real-account test for that provider: connect, execute representative read actions, execute a safe approved write, disconnect/revoke, reconnect/refresh, and verify least-privilege behavior.