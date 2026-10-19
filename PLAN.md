# Graphite Mail — Linux-first offline Office 365 mail & calendar client

Scope v1: Mail + Calendar, Microsoft Graph only. SQLCipher deferred. Windows optional.

| # | Milestone | Status |
|---|-----------|--------|
| 1 | Scaffold Tauri/React, Linux deps, .deb build pipeline | done |
| 2 | OAuth2 PKCE (loopback) + tokens in GNOME Keyring | done |
| 3 | First-run wizard: choose data folder, bootstrap.json, SQLite schema + FTS5 | done |
| 4 | Graph folder + message delta sync, body/attachment hydration (CAS blobs) | done |
| 5 | 3-pane UI (virtualized list, sandboxed HTML reader, light/dark), offline search | done |
| **6** | **50% checkpoint — .deb for Faisal to test on Ubuntu** | passed |
| 7 | Offline mutation queue: read/flag/move/delete, compose/send/drafts, replay-before-pull | built – 0.8.2 fixes bug 4 |
| 8 | Calendar: calendarView delta, month/week views, RSVP, TZ mapping | built – awaiting test |
| 9 | Crash-safe data-folder migration (7-phase) | built – awaiting test |
| 10 | Linux release: .deb + AppImage, icon, user guide | |
| 11 | (optional) Windows build | |
