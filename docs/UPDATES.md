# Updates

OpenMindAI has two independent update systems: **application updates**
(a new OpenMindAI release) and **model updates** (a new/updated AI model).
They're versioned and checked separately — an app release and a model
release don't have to happen together.

## Application updates

Settings → Updates has:

- **Automatically check for updates** (on by default) — checks in the
  background shortly after startup, never blocks opening the app, and never
  surfaces an error if the check fails (e.g. no internet, or the update
  server is unreachable) — it just quietly tries again next time.
- **Automatically download application updates** (off by default) — if
  enabled, a found update downloads and installs itself in the background.
  It never restarts the app for you; you always confirm the restart
  yourself, either from a Windows notification or the Settings → Updates
  panel.
- A manual **"Check for Updates Now"** button, which — if it finds an
  update — shows a **"Download & Install Update"** button with a real
  progress bar, and a **"Restart Now"** button once it's ready.

The update pipeline is built on Tauri's updater plugin and does real
download/verify/install/restart, not just a check. It only ever replaces
this application's own installed files (found via Windows' own
app-registration metadata) — it has no reference to `OPENMINDAI_ROOT` and
cannot touch your models, database, or conversations, by construction (the
installer and your data live in separate locations — see
[docs/PORTABLE-STORAGE.md](PORTABLE-STORAGE.md)).

**Current status:** the client-side pipeline above is fully implemented and
was tested against a local mock update manifest. The *real* production
update server endpoint is not live yet — `tauri.conf.json`'s updater
`endpoints` still points at a placeholder
(`https://REPLACE-BEFORE-RELEASE.invalid/...`). Until that's replaced with a
real, hosted update manifest, update checks will simply never find an
update in production — which is a safe, quiet no-op, not a broken state
(see [Offline Mode](OFFLINE-MODE.md)). See
[docs/RELEASE.md](RELEASE.md) for what's needed to finish this.

## Model updates

Settings → Updates also has:

- **Notify me when recommended model updates are available** (on by
  default).
- **Automatically download large AI model updates** (off by default, since
  these are multi-gigabyte downloads).
- A manual **"Check for Model Updates"** button showing each catalog
  entry's installed/compatible/update-available status.

See [docs/MODEL-MANAGEMENT.md](MODEL-MANAGEMENT.md) for why v1's model
catalog realistically never reports an update today (it has exactly one
entry, matching the model already installed) and how that's still real,
working infrastructure rather than a stub.

## Update channel

Only **Stable** exists in v1. The setting is there for future use but has
nothing else to select yet.
