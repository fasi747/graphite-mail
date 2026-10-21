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

## Task 2 — Bcc (v1.10.3)  ✅
- Frontend (`src/Compose.tsx`): `bcc` state, "Bcc" link on the Cc row (`.fieldlink`, same borderless `.field` row style),
  the Bcc row appears below Cc and uses `RecipientInput` (same contact suggestions as To/Cc). Send validates all three
  lists and allows Bcc-only messages. Drafts save/restore `bcc`; `Mailbox.tsx` passes `bcc` when reopening a draft.
  Reply / reply-all / forward never pre-fill Bcc.
- Rust: `outbox::Draft.bcc` (serde default, so old queued items still parse); the queued payload carries `"bcc"`.
  - **Microsoft Graph**: `bccRecipients` on `/me/sendMail`, on `createReply/createReplyAll/createForward` + PATCH
    (attachments path), and on `/reply`, `/replyAll`, `/forward`. A forward's top-level `toRecipients` stays To only.
  - **Gmail API (Google sign-in)**: `build_raw()` takes `bcc`; it writes a `Bcc:` header because the Gmail API derives
    envelope recipients from the headers and strips Bcc on delivery (documented Gmail behaviour; cannot be observed in
    the sandbox, see "needs a real-world check").
  - **IMAP/SMTP (app password)**: `build_raw()` is called with no Bcc at all; the blind copies are added to the lettre
    `Envelope` (RCPT TO). The message data contains neither a Bcc header nor the address.
  - Drafts: `bcc_text` column, schema + `migrate()` + migration test.
- Tests (Rust +3 → 72): `outbox::blind_copies_never_leak_into_graph_to_or_cc` (new/reply/replyAll/forward against the
  fake Graph: address appears exactly once, in bccRecipients), `google::blind_copies_are_a_bcc_header_for_the_gmail_api_
  and_nothing_else`, the Gmail end-to-end test asserts the raw sent to the fake Gmail API, the IMAP end-to-end test
  asserts RCPT TO has the blind copy and the DATA has no Bcc header, the attachments test asserts the draft-with-files
  Graph body, storage migration test. Frontend +5 → 81 (Bcc link, suggestions, send payload split, Bcc-only send and
  validation, draft round trip, replies start empty).
- Decision: Bcc-only messages (no To) are allowed, as in Outlook/Gmail. If this isn't wanted, re-add the To check in
  `Compose.send()`.
- Fixed on the way: `Compose` wrote the signature into the editor after unmount (seen as an unhandled error in tests).
- Needs a real-world check: Gmail API strips the Bcc header for the other recipients (send a Bcc'd message from a
  Google-signed-in account and inspect what the To recipient receives); Outlook reply/forward with Bcc delivers to the
  blind copy; Gmail SMTP sent copy (Gmail's Sent Mail) doesn't show the Bcc (expected, same as other clients).
- Counts after Task 2: Rust 72, frontend 81. tsc clean. Version 1.10.3.

## Task 3 — Windows port (v1.11.0)  ✅ (code + CI; real-machine checks listed below)
### Audit of Linux-only assumptions (Rust + frontend) and what changed
| Area | Finding | Change |
|---|---|---|
| Free-space check (`migrate.rs`) | `libc::statvfs` + `std::os::unix` | `#[cfg(unix)]` statvfs, `#[cfg(windows)]` `GetDiskFreeSpaceExW` (windows-sys 0.61, already in the lock file). `libc` is now a unix-only dependency. |
| Keyring (`Cargo.toml`) | only `sync-secret-service` | added `windows-native` → Windows Credential Manager. Each keyring feature compiles only on its own platform. |
| Data folder default (`storage.rs`) | `dirs::data_dir()` = roaming AppData on Windows | `dirs::data_local_dir()` (`%LOCALAPPDATA%\graphite-mail`; unchanged on Linux). |
| Config pointer (`storage.rs`) | `XDG_CONFIG_HOME` only honoured by `dirs` on Linux | honoured explicitly on every platform, so the tests stay in a scratch folder on the Windows runner too. |
| Attachment names (`attachments.rs`) | only `/ \ NUL` and control chars replaced | also `< > : " \| ? *`, trailing dots/spaces, device names (CON, NUL, COM1–9, LPT1–9). |
| Folder picker (`App.tsx`, `Settings.tsx`) | `${picked}/graphite-mail` | `dataFolderFrom()` in `src/paths.ts` uses the separator the picked path already has. |
| Opening links / files | already `tauri-plugin-opener` (no `xdg-open`) | none needed. |
| Notifications | already `tauri-plugin-notification` (no D-Bus / `notify-send` in app code) | none needed. |
| System theme | `matchMedia(prefers-color-scheme)` in the WebView (WebView2 supports it) | none needed. |
| File dialogs | `tauri-plugin-dialog` | none needed. |
| Shell commands | none in the code base | — |
| Window (`decorations: false`, custom buttons, drag region, splash `skip_taskbar`) | all Tauri APIs | none; see checks below. |
| Sign-in redirect | loopback `127.0.0.1:<random>` listener, browser opened through the opener plugin | none; Windows loopback needs no firewall rule. |
| `main.rs` | already has `windows_subsystem = "windows"` for release | — |
- NSIS: `src-tauri/tauri.windows.conf.json` (Tauri's per-platform overlay) sets `targets: ["nsis"]`, `installMode:
  currentUser`, `installerIcon: icons/icon.ico`, `webviewInstallMode: downloadBootstrapper` (silent). The base config
  keeps deb + appimage for Linux, so a local `tauri build` on either platform does the right thing.
- CI: `.github/workflows/release.yml` — on `v*.*.*` tags: tests on ubuntu-22.04 and windows-latest (type check, vitest,
  cargo test), then `.deb` (ubuntu-22.04) and NSIS `.exe` (windows-latest, NASM installed for aws-lc-sys) on separate
  runners, versions checked against the tag, then a **draft** release with both files attached (`gh release`, built-in
  token). Only first-party actions (checkout, setup-node, cache, upload/download-artifact). The Google client secret
  is not referenced anywhere; a step fails the job if `src-tauri/google_oauth.json` is present.
- Verification done here: `cargo check --target x86_64-pc-windows-gnu --tests` **passes** (mingw toolchain installed in
  the sandbox), so every `cfg(windows)` branch and the Windows-only tests type-check. Linux: Rust 75 tests, frontend
  83, tsc clean.
- Tests added: `src-tauri/src/platform_tests.rs` (free space on temp + not-yet-existing folder; default data folder is
  absolute, per-user, local on Windows; Windows-safe attachment names; `#[cfg(windows)]`: drive-letter paths through
  `normalize()`, keyring round trip in the Credential Manager — these two run on the Windows CI runner), `src/paths.test.ts`.
### Can only be verified on a real Windows machine
1. The NSIS installer: per-user install path (`%LOCALAPPDATA%\Programs\Graphite Mail`), Start-menu shortcut, icon,
   uninstaller, WebView2 bootstrapper when the runtime is missing.
2. First run: config pointer in `%APPDATA%\graphite-mail`, data in `%LOCALAPPDATA%\graphite-mail`, folder picker with a
   backslash path, "Move data folder" (free-space check, rename/remove while SQLite has files open — Windows locks
   open files; `migrate.rs` closes the DB before switching, but this is the biggest risk).
3. Browser sign-in: default browser opens, redirect to `http://localhost:<port>` comes back, the window regains focus.
4. Keyring: tokens/app passwords in Credential Manager (test runs in CI). Credential Manager caps a secret at 2560
   bytes; Microsoft refresh tokens are usually 1–2 KB. If one is ever longer, sign-in would fail with a keyring
   error — worth watching.
5. Frameless window: resize from the edges, snap/maximise, the custom min/max/close buttons, double-click to maximise,
   the splash window (no taskbar entry, always on top).
6. Notifications: Windows toasts need the app registered with an AppUserModelID — the NSIS install provides the shortcut;
   running the bare `.exe` from `target/` may show nothing.
7. Attachments: open (temp folder under `%TEMP%`) and Save As; names with characters Windows refuses.
8. The `cargo test` run on the Windows runner itself (file-locking differences in the migration tests; see 2).
9. MSVC build: the sandbox cross-check used the GNU target; the CI uses MSVC (what Tauri supports). aws-lc-sys needs
   NASM + CMake there (the workflow installs NASM; CMake is on the runner image).
10. Whether pushing `.github/workflows/` is accepted (see below).
