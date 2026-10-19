# Graphite Mail — brief for external testers

**What it is:** a Linux desktop mail + calendar client for Microsoft 365 and Gmail (Tauri 2, Rust backend, React UI). Offline-first: mail is stored locally and changes sync later.
**Build to test:** `graphite-mail_<version>_amd64.deb` (Ubuntu 24.04). Install: `sudo apt install ./graphite-mail_<version>_amd64.deb`, run `graphite-mail` (start it from a terminal and keep the `[graphite]` log lines).
**Source:** github.com/fasi747/graphite-mail (private; access on request).

## Please use test accounts only
Use a throwaway Microsoft 365 user and a throwaway Gmail account. Rules, bulk actions and attachments will be tried. Do not use real mailboxes. Tell us if you need test accounts.

## Known limitations (no need to report)
- The local database is **not encrypted** (SQLCipher is planned). Tokens are in the OS keyring.
- The package is **not signed**; no auto-update.
- Test builds are made **without** the Google client secret (Gmail sign-in is then disabled unless a test client is supplied).
- Drafts are local only; Gmail forward/attachment limits (15 MB total); Gmail "delete forever" does nothing (30-day Trash).
- Windows is not supported yet.

## Part 1 — Functional and bug testing
**Install / start-up:** clean install, upgrade over an older version, uninstall (`apt remove`), reinstall; splash screen (about 5 s) then main window; first-run folder choice; start with no network.
**Accounts:** add / remove Microsoft and Gmail accounts, add the same account twice, cancel the browser sign-in, expired or revoked token (revoke at myaccount.google.com/permissions / My Apps), sign out of all.
**Mail reading:** folders and sub-folders, unread counts, search (words in subject/sender/body), long lists (thousands of messages), HTML mail in light/dark, inline images, remote images blocked until "Load", links open in the browser, mailto links, big messages, odd encodings (Arabic, Chinese, emoji), plain-text mail.
**Actions:** read/unread, flag/star, move, archive, delete (to Trash), delete in Trash, new folder / subfolder / rename / delete, with each account.
**Compose:** new, reply, reply all, forward; From picker (replies must come from the receiving account); formatting; signature per account (swaps with From); drafts (autosave, save on cancel, reopen, delete); invalid addresses; very long text.
**Attachments:** open, save, large files (over 3 MB on Outlook, up to 15 MB), many files, odd file names (spaces, unicode, `../`), forward keeps originals, executables are never opened (only saved).
**Offline:** go offline, do actions and send, reconnect; "waiting to sync" details; rejected changes; kill the app mid-sync; flaky network.
**Rules:** create / edit / disable / delete on both accounts, every condition and action, "Run on existing mail", conflicting rules; check they really exist at Outlook / in Gmail filters.
**Calendar:** week and month views, create / delete events, RSVP, Teams and Google Meet links, all-day events, time zones and DST, recurring events, shared calendars, hide/show calendars.
**Multi-account:** All Inboxes, same message in two accounts, moving mail across accounts must be refused.
**Data folder move:** move to another disk, to a full disk, to a non-empty or nested folder, kill the app / power-cut in the middle (it must recover on next start).
**UI:** window buttons, resize, high-DPI, keyboard shortcuts, light/dark, notifications, very long names.
**Performance:** a mailbox with 20k+ messages: first sync time, memory, CPU while idle, UI responsiveness, database size.

## Part 2 — Security testing
**Credentials and tokens**
- Where are refresh/access tokens stored (should be the keyring only)? Any tokens, codes or secrets in logs, crash output, database, temp files or the `.deb`?
- OAuth: PKCE, `state` check, loopback listener (port, binding to 127.0.0.1 only, can another local process or a web page steal the code?), scope minimisation (Graph: Mail.ReadWrite, Mail.Send, Calendars.ReadWrite, MailboxSettings.ReadWrite on request; Google: gmail.modify, calendar, gmail.settings.basic on request).
- Behaviour on token revocation and on sign-out (are tokens and mail really removed?).

**Hostile email (most important)**
- HTML/CSS/SVG injection, scripts, event handlers, `<meta refresh>`, `<base>`, forms, `javascript:`/`data:`/`file:` links, CSS `url()` / `@import` tracking when remote content is blocked, `cid:` abuse, mXSS payloads against DOMPurify, very deep or huge HTML, malformed MIME and charsets, zip-bomb-like attachments, giant headers.
- Can email content reach the app's Tauri commands (IPC) or break out of the sandboxed frame? Can a link navigate the app window or open a non-http scheme?
- Tracking pixels and read-receipts while "remote content" is blocked.

**App hardening**
- Tauri configuration: a strict CSP is set (`default-src 'self'; img-src 'self' data: blob: https:; …`) — try to bypass it; capabilities are trimmed to event listening, the window buttons and file dialogs; link handling allows only http/https/mailto (`links.rs`) and email links are routed through a refused new-window request — try to bypass `on_navigation` / `on_new_window`, and to reach IPC commands from email content.
- Local files: permissions of the data folder, database and blobs (world-readable?), temp files for opened attachments (`/tmp/graphite-mail-attachments`), file-name handling (path traversal, symlinks), "Save as" overwrite behaviour.
- Dangerous file handling: executables/scripts must never be run by "Open".
- SQL injection (search, rules, folder names), FTS query injection, path injection in the data-folder move, JSON/IPC argument validation, integer/size limits.
- Denial of service: huge mailboxes, message with thousands of attachments, endless redirects, hanging servers (timeouts), memory growth.
- Offline queue integrity: tampering with the queue, duplicate sends, replay after partial failure.
- Multi-account isolation: can an action or reply ever use the wrong account's token?
- Rules: can a crafted rule or a malicious message cause mass deletion or forwarding?

**Supply chain and packaging**
- Run `cargo audit` and `cargo deny` (Rust), `npm audit` (JavaScript); review the main dependencies (tauri, wry, reqwest, rusqlite, keyring, mail-parser, mail-builder, DOMPurify).
- `.deb` contents and maintainer scripts, file ownership and permissions, desktop entry, no hidden network calls (the app should talk only to Microsoft and Google endpoints and nothing else — please confirm with a proxy such as mitmproxy).
- TLS: certificate validation, no cleartext requests.

**Privacy**
- What is written to disk and kept after sign-out or "Remove account"? Is anything sent to anyone other than Microsoft/Google?

## What we would like back
A short report with: findings listed by severity (Critical / High / Medium / Low / Info), exact steps to reproduce, expected vs. actual result, screenshots or the `[graphite]` log lines, app version (`dpkg -l graphite-mail`), Ubuntu version, and account type (Microsoft or Gmail). Please also list what you tested and found OK.
