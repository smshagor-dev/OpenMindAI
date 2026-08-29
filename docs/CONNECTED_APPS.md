# Connected apps

OpenMindAI treats connected services as internal assistant tools. Users connect or disconnect apps in **Settings → Apps**, then use normal language in Chat or Project Work.

There is no user-facing provider/action picker, raw JSON action console, or separate “Connected Work” surface. The assistant chooses an appropriate connected capability when relevant and returns the useful result in the conversation.

## UX contract

- **Settings → Apps** is only for connection state and one-time provider setup.
- **Chat and Project Work** are the interaction surfaces. Users ask for the outcome, not an API action.
- Connection secrets and OAuth tokens stay in the operating-system credential store and are never written into chat history.
- Remote mutating actions remain protected by backend approval and provider-permission guards.
- Provider data and tool output are treated as untrusted external data, not instructions.

## Supported app families

- Google Workspace: Gmail, Drive, Calendar, Contacts
- GitHub: repositories, files, issues, pull requests, Actions, releases
- Microsoft 365: Outlook, OneDrive, Calendar, Contacts
- Slack
- Notion
- Dropbox
- MCP servers

Provider-specific OAuth/client configuration may still be required by this self-hosted desktop application, but that setup stays inside **Settings → Apps** and never appears in Work.
