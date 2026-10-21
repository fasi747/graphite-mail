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
