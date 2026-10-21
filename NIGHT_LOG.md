# Night log — overnight/v1.11

(Summary is added at the top when the run ends.)

## Pre-check
- `git fetch origin && git checkout main && git pull` → main at 6ae8af6 (v1.10.1 tag present).
- package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json all said **1.10.1** → continued.
- Branch `overnight/v1.11` created from main.
- Environment: `GRAPHITE_GOOGLE_CLIENT_SECRET` is set in this sandbox. It is never read, printed or committed; every
  cargo / npm build or test runs with `env -u GRAPHITE_GOOGLE_CLIENT_SECRET`. No `.deb` is produced tonight.
- The sandbox had no GTK/WebKit development libraries; installed `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev
  librsvg2-dev libayatana-appindicator3-dev patchelf` with apt (sandbox only, nothing in the repo).
- Baseline before any change: Rust 69 tests pass, frontend 64 tests pass (12 files), `tsc --noEmit` clean.

## Task 1 — v1.10.2 fixes  ✅
### 1a Mail list context menu: no destructive item gets initial focus
- Root cause: `ContextMenu` started on the first *enabled* item. With several messages selected (or before the open
  message's details arrive) Reply / Reply All / Forward are disabled, so the first enabled item was **Delete**, and a
  first Enter deleted the selection.
- Fix: `MenuItem.destructive` flag (`src/ContextMenu.tsx`); `initialFocus()` in `src/menu.ts` picks the first enabled,
  non-destructive item, or `-1` when there is none. With `-1` the menu panel itself takes focus, Enter/Space do nothing,
  ArrowDown goes to the first enabled item and ArrowUp to the last. Mail list marks Delete, Block sender, Mark as spam;
  the Contacts menu marks Block sender (`src/contactMenu.ts`).
- Tests: `menu.test.ts` (initialFocus), `ContextMenu.test.tsx` (3 cases: skips Delete, Reply first when enabled, all-
  destructive menu focuses the panel and Enter is a no-op), `Mailbox.test.tsx` (right-click a real row while Reply is
  unavailable: focus is "Move to…", Enter opens the folder picker and never calls `delete_message`; and Delete still
  works when reached on purpose). The Mailbox tests mock `@tanstack/react-virtual` because jsdom has no layout.
### 1b First-run screen: Cancel sign-in, 2-minute timeout, cleanup, no late error
- New `src/signIn.ts` → `useBrowserSignIn()` hook, extracted from the v1.9.1 Settings code (attempt counter, timer,
  `cancel_sign_in` invoke, unmount cleanup). Settings → Accounts now uses the hook (behaviour unchanged, its 6 tests
  still pass). The first-run screen (`src/App.tsx`) uses it too: "Cancel sign-in" button, timeout note, the buttons
  come back, a late error or late success from a cancelled attempt is ignored.
- "Listener cleanup" read as: the timer is cleared and the attempt retired on unmount, and the Rust redirect listener
  is told to stop through `cancel_sign_in` (already existing in `auth.rs`, tested there).
- Tests: `App.test.tsx` +5 (cancel + late error ignored, stale success doesn't open the mailbox while a fresh one does,
  timeout at exactly 120 s, unmount clears the timer, app-password path clears the note).
- Version bumped to **1.10.2** (package.json, package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json). CHANGELOG entry added.
- Test counts after Task 1: Rust 69, frontend 76 (12 files). tsc clean.
- Needs a real-world check: pressing Cancel while a real browser tab is open (the tab then shows "Sign-in was cancelled
  or failed"); menu-key behaviour inside the real WebView.
