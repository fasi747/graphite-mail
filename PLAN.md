# Graphite Mail — Linux-first offline Office 365 mail & calendar client

Scope v1: Mail + Calendar, Microsoft Graph only. SQLCipher deferred. Windows optional.

| # | Milestone | Status |
|---|-----------|--------|
| 1 | Scaffold Tauri/React, Linux deps, .deb build pipeline | done |
| 2 | OAuth2 PKCE (loopback) + tokens in GNOME Keyring | done |
| 3 | First-run wizard: choose data folder, bootstrap.json, SQLite schema + FTS5 | done |
| 4 | Graph folder + message delta sync, body/attachment hydration (CAS blobs) | done |
| 5 | 3-pane UI (virtualized list, sandboxed HTML reader, light/dark), offline search | done |
| **6** | **50% checkpoint — .deb for Faisal to test on Ubuntu** | done |
| 7 | Offline mutation queue: read/flag/move/delete, compose/send/drafts, replay-before-pull | done |
| 8 | Calendar: calendarView delta, month/week views, RSVP, TZ mapping | done |
| 9 | Crash-safe data-folder migration (7-phase) | done |
| 10 | Linux release: .deb + AppImage, icon, user guide | done |
| 11 | (optional) Windows build | |

**v1.0.0 released** — tested on Ubuntu 24.04 with a real Microsoft 365 account.

## v1.2 — multiple accounts and Gmail

- [x] Multi-account backend (per-account keyring tokens, sync, queue, All Inboxes)
- [x] Multi-account UI (grouped sidebar, From picker, add/remove account, New Folder per account)
- [x] Gmail: sign-in (desktop client, PKCE), labels → folders, history-based sync, send/reply, star/read/move/trash, Google Calendar
- [x] Gmail tested with a real account (Faisal, v1.2.1)

## v1.3 — rules
- [x] Server-side rules: Outlook inbox rules + Gmail filters (conditions: from, subject, to/cc, words; actions: move, mark read, flag/star/important, delete)
- [x] "Run on existing mail" (applied locally, sent to the server through the offline queue)
- [ ] Tested with real accounts (rules)

## v1.4 — new logo and splash
- [x] New logo (vector sources in `assets/`), app icon regenerated, README logo
- [x] Animated splash screen; the main window stays hidden until the animation (3.4 s) has finished and the UI is ready

## v1.5 — attachments
- [x] Attachment list in the reader, open / save, downloaded on demand and kept offline (Outlook + Gmail)
- [x] Safe-open: scripts and programs can only be saved
- [x] Attach files when writing (Outlook draft + upload session for big files; Gmail MIME), forwards keep the original's files

## v1.6 — signatures, drafts, bigger window buttons
- [x] Signature per account (new, reply, forward; follows the From account)
- [x] Drafts saved on this computer (autosave, save-on-cancel, Drafts list)
- [x] Larger minimise / maximise / close buttons
