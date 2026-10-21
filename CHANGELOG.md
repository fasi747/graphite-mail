# Changelog

## 1.10.2 — Safer menus and first-run sign-in
- Mail list right-click menu: the keyboard never starts on Delete, Block sender or Mark as spam. When the items above
  them are unavailable (several messages selected, or the message still loading) focus starts on the next safe item,
  and if nothing safe is enabled the menu itself takes focus, so the first Enter can never delete anything. Arrow keys
  still reach every item. The same rule applies to the Contacts menu (Block sender).
- First-run screen: **Cancel sign-in** while the browser sign-in waits, a 2-minute time limit with a message, and a
  cancelled or timed-out sign-in can no longer bring a late error or open the mailbox. The sign-in logic is now shared
  with Settings → Accounts (one hook, `src/signIn.ts`), and leaving the screen clears its timer.

## 1.10.1 — Contacts right-click menu
- Right-click a contact (or Shift+F10 / the menu key) for New email, New meeting (contact invited, in that account's
  calendar), Search mail with this person (local search, `with:address` in the search box), Copy email address,
  Copy name and address, Copy phone number, Add to / Remove from favourites, Export as vCard, Block sender and
  Create rule for this sender. Several addresses or phone numbers: a submenu to choose. No address: email items hidden.
- Multi-select in the contact list: Ctrl+click, Shift+click ranges, hover checkboxes, Ctrl+A, Esc. The selection's menu:
  New email to all, New meeting with all, Copy all addresses (semicolon-separated), Export selected as vCard.
- Favourites, stored locally per account (removed with the account), shown in a Favourites group at the top.
- vCard 3.0 export (.vcf): N, FN, ORG, TITLE, every EMAIL and TEL, escaped and folded as RFC 2426 says.
- The shared right-click menu gained submenus (→ / Enter / hover to open, ← / Esc to go back) and stays inside the window.
- No new Microsoft Graph permissions.

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
