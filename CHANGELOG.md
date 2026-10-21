# Changelog

## 1.10.0 — Contacts (Microsoft 365)
- New **Contacts** view (View → Contacts, or the sidebar): **Directory** (Global Address List) and **My contacts**, each
  with a count and foldable; one search across both; account filter with several Microsoft 365 accounts; details pane with
  addresses, phones, job title, department, office, company, **New email** and **Copy email**.
- To/Cc suggestions in the editor from the sending account's contacts (Microsoft 365 accounts).
- Sync: personal contacts of every contact folder through Graph delta queries (full listing where delta isn't offered,
  restart when a sync point expires); the directory through `/users`, paged (999 per page), with a pause between pages,
  `Retry-After` honoured on 429/503, and entries that are gone removed after each full listing; `/me/people` fills in titles
  and phone numbers that basic directory access can't read. Disabled accounts and entries without a mailbox are skipped.
- **New permissions**, asked for only on **Grant access**: `Contacts.Read`, `People.Read`, `User.ReadBasic.All`.
  If they are missing or the organisation blocks them, the view says so and offers Grant access again. Mail and calendar
  sync never depend on them (a refresh that fails with them falls back to the mail/calendar permissions).
- Privacy: contacts stay in the local database only and are never logged; **Remove cached contacts** per account
  (Settings); removing an account deletes its contacts.

## 1.9.1 — Settings → Accounts fixes
- App-password form toggles, scrolls into view and focuses the address field; readable text boxes in all themes;
  Connect explains why it's inactive; browser sign-in can be cancelled and times out after 2 minutes; builds without the
  Google client secret say "Google sign-in coming soon".

## 1.9.0 — Gmail with an app password
- IMAP/SMTP account type for @gmail.com with a Google app password (no OAuth client needed).

Earlier versions: see [PLAN.md](PLAN.md).
