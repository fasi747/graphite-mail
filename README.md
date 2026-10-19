<p align="center"><img src="docs/logo.png" width="220" alt="Graphite Mail"></p>

# Graphite Mail

A free, open-source, **offline-first Microsoft 365 mail & calendar client for Linux**.
Built with Tauri (Rust) + React. Uses Microsoft Graph and the Gmail / Google Calendar APIs — no IMAP/POP3.

- Microsoft 365 **and Gmail**, several accounts at once, with an **All Inboxes** view
- Full offline reading and search of your mailbox
- Read, flag, move, delete, reply and send while offline — changes sync when you reconnect
- You choose where your mail data is stored, and can move it safely later (crash-proof, verified copy)
- macOS-style interface with light and dark mode
- Sign-in tokens kept in your system keyring

![Light](docs/screenshot-light.png)
![Dark](docs/screenshot-dark.png)
![Calendar](docs/screenshot-calendar.png)

## Install

**Ubuntu / Debian** — download the `.deb` from the releases, then:

```
sudo apt install ./graphite-mail_<version>_amd64.deb
graphite-mail
```

**Any Linux** — download the `.AppImage`, then:

```
chmod +x graphite-mail_<version>_amd64.AppImage
./graphite-mail_<version>_amd64.AppImage
```

New to Graphite Mail? Read the **[User Guide](docs/USER-GUIDE.md)**.

## Build from source

Needs Node 20+, Rust, and on Ubuntu: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libdbus-1-dev libsecret-1-dev patchelf`.

Gmail sign-in needs your own Google OAuth "Desktop app" client. Put it in the untracked file `src-tauri/google_oauth.json`
(copy `src-tauri/google_oauth.example.json`) or set `GRAPHITE_GOOGLE_CLIENT_ID` / `GRAPHITE_GOOGLE_CLIENT_SECRET` when building.
Without it the app builds and works, but "Add Google account" says it isn't set up.

```
npm ci
npm run tauri build        # .deb and .AppImage in src-tauri/target/release/bundle/
cd src-tauri && cargo test # automated tests
```

## Status

Mail, calendar, offline changes and data-folder moves are working. See [PLAN.md](PLAN.md).
