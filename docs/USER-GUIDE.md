# Graphite Mail — User Guide

Graphite Mail is a mail and calendar app for Microsoft 365 (work or school accounts) on Linux.
Your mail is saved on your computer, so you can read, search and write even without internet.
When you are back online, everything syncs by itself.

## Install

**Ubuntu / Debian (.deb)**

```
sudo apt install ./graphite-mail_<version>_amd64.deb
```

Then open **Graphite Mail** from your applications menu, or type `graphite-mail` in a terminal.

**Any Linux (AppImage)**

```
chmod +x graphite-mail_<version>_amd64.AppImage
./graphite-mail_<version>_amd64.AppImage
```

**To remove it:** `sudo apt remove graphite-mail`. Your mail folder is not deleted (see "Where your data lives").

**Windows 10 / 11 (installer)**

Run `Graphite Mail_<version>_x64-setup.exe`. It installs for your user only (no administrator password), adds a Start-menu
entry, and fetches the WebView2 runtime from Microsoft if the computer doesn't have it. SmartScreen may warn because the
installer isn't signed: *More info → Run anyway*. Remove it from *Settings → Apps*; your mail folder is kept.

## First start

1. **Choose a data folder.** This is where your mail and calendar are stored. The suggested folder is fine; you can move it later in Settings.
2. **Sign in with Microsoft.** Your web browser opens. Sign in, then return to Graphite Mail.
   Your sign-in is kept in your system keyring (GNOME Keyring or KDE Wallet), never in a plain file.
3. The first sync downloads your mail. Large mailboxes take a while; you can already read what has arrived.

## Several accounts (Microsoft 365 and Gmail)

You can use more than one account at the same time.

- **Add an account:** gear → **Add Microsoft account** or **Add Google account**. Your browser opens; sign in and come back.
  Changed your mind? **Cancel sign-in** stops waiting (after two minutes Graphite Mail stops waiting by itself).
  Or **Add Gmail (app password)** to use Gmail without the browser sign-in (see below).
- **The sidebar** has one section per account (name and email on top, its folders below; click the header to fold it).
  With two or more accounts, **All Inboxes** at the top shows every account's Inbox together. A small coloured dot tells you which account a message belongs to.
- **Writing:** the **From** line lets you choose the account. Replies and forwards always go out from the account that received the message.
- **New Folder** asks which account it belongs to. A message can only be moved to a folder of its own account.
- **Remove an account:** gear → **Remove** next to it. Its downloaded mail and calendar are deleted from this computer; nothing is deleted on the server.

### Gmail specifics

- Gmail has **labels** instead of folders. A label shows as a folder; a message can have several labels and appears under each of them.
  **Archive** shows mail that is in no other folder ("All Mail" without the Inbox). Moving a message into a label also takes it out of the Inbox.
- The first sync downloads the last 90 days (up to 3000 messages); after that only changes are fetched.
- **Star** is the Flag button. Delete sends mail to Trash; Gmail empties its Trash by itself after 30 days (Graphite Mail does not erase mail for good).
- Forwarding does not include the original attachments (Gmail doesn't do that for apps).
- Google Calendar works like the Microsoft calendar; **Google Meet video call** adds a Meet link to a new event.

### Gmail with an app password

If **Sign in with Google** isn't available to you, you can connect any @gmail.com account with a Google **app password** instead.
Graphite Mail then reads mail over IMAP and sends over SMTP, directly with Gmail.

1. Turn on **2-Step Verification** for your Google account (myaccount.google.com → Security). App passwords only exist with it.
2. Open **myaccount.google.com/apppasswords**, type a name (for example "Graphite Mail") and click **Create**. Google shows a 16-letter code.
3. In Graphite Mail: gear → **Add Gmail (app password)** (or, on the first screen, **Use Gmail with an app password instead**).
   Enter your Gmail address, paste the code (the spaces don't matter) and click **Connect**.

Graphite Mail checks the code with Gmail's mail servers before it saves anything. The app password is kept in your system keyring only.
To stop Graphite Mail using it, remove the account in Settings, or revoke the app password on the same Google page.

Differences from a Google sign-in account:
- **No calendar**: app passwords only give access to mail.
- Your labels, Starred, Important and **All Mail** show as folders. A message with several labels is stored once and appears under each of them.
- **Rules** (and **Block sender**) are kept on this computer and run on new Inbox mail each time Graphite Mail checks for mail, so only while the app is open.
- Deleting from **Trash** erases a message for good. Forwarding includes the original attachments.
- The first sync downloads the last 90 days of each folder (up to 3000 messages); bigger messages (over 5 MB) are downloaded when you open them.

## Bcc

In the editor, press **Bcc** (at the end of the Cc row) to add blind copies. Bcc recipients get the message but are not
named in it: Microsoft 365 and Gmail deliver to them separately, and over IMAP/SMTP Graphite Mail hands them to the mail
server as envelope recipients only, without a Bcc header. Drafts keep their Bcc; replies and forwards start without one.

## Signatures and drafts

- **Signature:** gear → **Signature**. Write one per account and click **Save signature**. It is added to every new message, reply and forward. If you change the **From** account, the signature changes with it.
- **Drafts:** if you cancel a message you started, Graphite Mail asks whether to **Save draft**. While you write, it also saves automatically every 20 seconds. Find them under **Home → Drafts**: **Open** continues where you stopped, **Delete** removes it. Sending a message deletes its draft.
- Drafts live **on this computer only**; they don't show up in the Drafts folder of Outlook or Gmail.

## Attachments

- **Reading:** a message with files shows them as small chips under the header (name and size). Click a chip to **open** it with your normal program, or **Save** to keep a copy wherever you like. The file is downloaded the first time and then kept, so it also opens offline.
- **Safety:** programs and scripts (for example `.exe`, `.sh`, `.AppImage`, `.desktop`) are never opened straight from an email. Use **Save** and open them yourself only if you trust the sender.
- **Writing:** in the editor click **📎 Attach**, choose one or more files, and remove any with ✕. Up to **15 MB** in total. Attachments are kept with your message, so they are sent when you reconnect.
- **Forwarding** includes the original message's attachments (Outlook and Gmail).

## Rules

**Home → Rules** lets you make mail sort itself, for example "everything from my bank goes into the Bank folder".

1. Pick the account (rules belong to one account) and click **New rule**.
2. Choose when it applies: *From*, *Subject*, *To/Cc* or *words in the message* contain some text. If you fill in more than one, all of them must match.
3. Choose what to do: move to a folder, mark as read, star / mark important, or delete (to Trash).
4. **Save rule.**

The first time, Graphite Mail asks you to **Allow rules** for that account: one more sign-in in your browser to give the extra permission.
(Gmail with an app password needs no extra permission: its rules are kept on this computer and run while Graphite Mail is open.)

Rules are stored at Microsoft (Outlook inbox rules) or Google (Gmail filters), so they keep working **even when Graphite Mail is closed**. They apply to mail that arrives from then on. To use a rule on mail you already have, click **Run on existing mail…** and choose the folder.

Good to know:
- Outlook rules can't set a flag, so "flag" becomes **Mark as important** there. On Gmail it **stars** the message.
- Gmail filters have no off switch: a rule you switch off is removed from Gmail and put back when you switch it on.
- Rules you made in Outlook on the web or in Gmail settings don't show up in this list yet.

## Contacts

**View → Contacts** (or **Contacts** at the top of the sidebar) shows two groups, each with a count:
**Directory** (everyone in your organisation's address list) and **My contacts** (your own Outlook contacts, all folders).
Click a group's name to fold it.

- **Search** looks through both groups at once (name, address, job title, department). With several Microsoft 365
  accounts, the menu under the search box shows one account at a time.
- Click a person to see their addresses, phone numbers, job title, department, office and company,
  then **New email** or **Copy email**.
- **Writing:** in To, Cc and Bcc, type two letters and pick a suggestion with the arrow keys and Enter (or click it).
  Suggestions come from the contacts of the account you send from.
- The first time, click **Grant access**: one sign-in in your browser for the Contacts permissions. If your organisation
  doesn't allow it, the view says so; an administrator can approve Graphite Mail, then use Grant access again.
- Contacts are kept on this computer for offline use and updated in the background (your contacts every 30 minutes,
  the directory once a day). **Refresh contacts** (the ↻ button) updates them now.
- **Settings → Remove cached contacts** deletes an account's downloaded contacts from this computer (nothing changes
  at Microsoft); Refresh contacts brings them back. Removing the account deletes them too.
- **Gmail with an app password** has its own groups, kept on this computer: **People you've emailed** (everyone in the
  From/To/Cc of your synced mail, except no-reply and other automated senders, with how many messages you exchanged and
  when the last one came; updated after every sync) and **Imported**: **Import contacts…** reads a vCard (.vcf) file or
  a Google Contacts CSV export (several addresses and phone numbers per person; duplicates are merged by address).
  Google's own contacts need Google sign-in and come in a later update. Both groups work with the right-click menu,
  favourites, vCard export and To/Cc/Bcc suggestions. Gmail accounts added through Google sign-in have no Contacts group yet.

**Right-click a contact** (or press Shift+F10 / the menu key) for: **New email**, **New meeting** (an event with them
invited, in that account's calendar), **Search mail with this person** (mail from or to their addresses, searched on this
computer; the search box shows `with:address`), **Copy email address**, **Copy name and address**, **Copy phone number**,
**Add to / Remove from favourites**, **Export as vCard…**, **Block sender** and **Create rule for this sender…**.
When a contact has several addresses or phone numbers, a submenu lets you choose. Items that need an address are left
out for contacts without one.

- **Favourites** are kept on this computer for each account and shown in their own group at the top of the list.
- **Selecting several:** Ctrl+click, Shift+click for a range, the checkbox that appears on hover, or Ctrl+A in the list
  (Esc clears). Right-click the selection for **New email to all**, **New meeting with all**, **Copy all addresses**
  (separated by semicolons) and **Export selected as vCard**.
- **vCard files** (.vcf, version 3.0) open in most address books and phones.

## Everyday use

| To do this | Do this |
|---|---|
| Read a message | Click it in the list. Up/Down arrows move between messages. |
| Search | Type in the Search box. It searches subject, sender and the full text, offline too. |
| Write a new message | Click the pencil, or press **Ctrl+N** |
| Reply / Reply all / Forward | Buttons above the message, or **Ctrl+R** / **Ctrl+Shift+R** / **Ctrl+Shift+F** |
| Mark read or unread | The circle button, or **Ctrl+Shift+U** |
| Flag | The flag button |
| Move to a folder | The folder button, then pick the folder |
| Delete | The bin button or **Delete**. In *Deleted Items* it asks first, then deletes for good. |
| Get new mail now | The circular-arrow button. It also happens automatically every minute. |
| Light or dark | The half-moon button at the top right |

**Links** in a message open in your normal web browser. **Pictures from the internet** are hidden at first
to protect your privacy (senders can see when you open a message otherwise); press **Load** to show them.
Pictures that are part of the message itself show straight away.

## Working offline

Everything you do offline (read, flag, move, delete, reply, write) is saved and sent when you are back online.
At the bottom of the sidebar you see how many changes are **waiting to sync**. Click that line to see them.

- **Waiting** — will be sent automatically at the next sync.
- **Not accepted** — Microsoft refused the change (for example, the message was already deleted by someone else, or you
  no longer have permission). The reason is shown. **Try again now** sends it again; **Discard not accepted** gives up on it.

Nothing is thrown away without you seeing it.

## Calendar

Click **Calendar** at the top of the sidebar. Switch between **Week** and **Month**, use ‹ › to move, **Today** to jump back.

- Click an event to see details, **Join Teams meeting**, and to reply **Accept / Maybe / Decline**.
- Double-click an empty time (or press the pencil) to create an event. Add invitees by email address, tick **Teams meeting** if you want a link.
- The tick boxes under **Calendars** show or hide a calendar.

## Settings and your data folder

Click the **gear** in the bottom-left corner.

**Move to another folder…** moves your whole mail and calendar to a new place (for example a bigger disk).
It is designed so you cannot lose mail:

1. The new folder must be empty and have enough space.
2. Everything is copied and each file is checked after copying. Your current folder stays untouched.
3. Only when the copy has passed all checks does Graphite Mail switch over, and only then is the old folder cleaned up.
4. If the computer switches off in the middle, Graphite Mail puts things right the next time it starts.

Syncing pauses while the move runs. Keep the app open until it says **Done**.

### Where your data lives

- Your mail and calendar: the data folder you chose (inside it: `db/`, `blobs/`).
- A small pointer file remembering where that is: `~/.config/graphite-mail/bootstrap.json`.
- Your sign-in: the system keyring (entry named *graphite-mail*).

To keep a backup, copy the data folder while Graphite Mail is closed.

## Troubleshooting

**"Your Microsoft sign-in has expired."** Click the sign-out button (bottom-left), then sign in again.

**"Google didn't accept the app password."** Check the address, and paste the code again; if you revoked it, create a new one at
myaccount.google.com/apppasswords, then remove the account in Settings and add it again.
**"Google wants an app password … not your normal password."** Your normal Google password doesn't work here: create an app password
(it needs 2-Step Verification). **"Couldn't reach imap.gmail.com."** You're offline, or a firewall blocks mail ports 993 and 465.

**Sign-in asks me to unlock a keyring.** Graphite Mail stores your sign-in in the system keyring. Enter your login password when asked.
If you have no keyring installed: `sudo apt install gnome-keyring`.

**A link does nothing.** Graphite Mail asks your system to open it, so you need a default web browser set
(Settings → Default Applications). If it still fails you will see a message.

**The calendar failed to sync.** The status line at the bottom-left says why. A work or school account may need an administrator to allow calendar access.

**Something else is wrong.** Start the app from a terminal with `graphite-mail` and look at the lines beginning with `[graphite]` — send them to the developer.

## Privacy

Graphite Mail talks only to the services of your accounts: Microsoft (sign-in and Microsoft Graph) and Google (sign-in, the Gmail and Calendar APIs,
or for app-password accounts Gmail's mail servers imap.gmail.com and smtp.gmail.com). There is no tracking and no other server.
Emails cannot run scripts, and remote pictures are blocked until you allow them.
