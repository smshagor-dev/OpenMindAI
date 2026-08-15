# Database

SQLite is stored at:

```text
OpenMindAI/data/database/openmind_ai.db
```

The backend enables WAL mode and foreign keys when opening the database. Migrations live in `src-tauri/migrations`.

Initial tables:

- `app_profiles`
- `conversations`
- `messages`
- `conversation_settings`
- `model_registry`
- `app_settings`
- `hardware_profiles`
- `runtime_profiles`

Streaming responses are represented as assistant messages with `status='streaming'`. If generation is stopped or fails, already generated content remains persisted and the status is updated to `cancelled` or `failed`.

Milestone 2 tests cover:

- WAL and foreign key pragmas
- migration startup on a fresh database
- file-backed restart persistence
- create, list, rename, pin, archive, and delete conversation operations
- interrupted assistant messages that survive reopening the database
- controlled error handling when the database path is unavailable

Application preferences are stored in `app_settings` under the `app.preferences` key. This includes theme, chat behavior, privacy toggles, runtime autostart, and UI preferences so settings travel with the OpenMindAI Root.

User personalization is stored in `app_settings` under `app.user_profile`. Profile fields include name, email, about, occupation, preferred name, response style, and custom instructions. When present, the backend writes this as hidden `system` context for conversations so future local inference can personalize responses without showing that context as a normal chat bubble.
