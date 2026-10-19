//! Split storage: a fixed bootstrap pointer in ~/.config/graphite-mail/bootstrap.json
//! and a user-chosen, movable data vault holding the SQLite DB and blob store.

use anyhow::{Context, Result, anyhow};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Bootstrap {
    pub schema_version: u32,
    pub active_data_root: String,
    pub migration_state: String,
    pub pending_migration_target: Option<String>,
    /// While a finished move is being tidied up: the folder the data came from.
    #[serde(default)]
    pub previous_data_root: Option<String>,
}

pub(crate) static DB: Mutex<Option<Connection>> = Mutex::new(None);

fn config_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or(anyhow!("No config dir"))?
        .join("graphite-mail"))
}

fn bootstrap_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("bootstrap.json"))
}

pub fn default_data_dir() -> String {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("graphite-mail")
        .to_string_lossy()
        .into()
}

pub fn read_bootstrap() -> Option<Bootstrap> {
    let s = fs::read_to_string(bootstrap_path().ok()?).ok()?;
    serde_json::from_str(&s).ok()
}

/// Atomic write: temp file + fsync + rename + fsync parent directory.
pub(crate) fn write_bootstrap(b: &Bootstrap) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir)?;
    let tmp = dir.join("bootstrap.json.tmp");
    {
        let f = fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(&f, b)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, bootstrap_path()?)?;
    fs::File::open(&dir)?.sync_all()?;
    Ok(())
}

/// First-run: validate the chosen folder, create the vault layout and DB, then record the pointer.
pub fn initialize(data_root: &str) -> Result<()> {
    let root = Path::new(data_root);
    fs::create_dir_all(root).with_context(|| format!("Cannot create {data_root}"))?;
    let probe = root.join(".write_test");
    fs::write(&probe, b"ok").context("Folder is not writable")?;
    fs::remove_file(probe)?;
    for d in ["db", "blobs", "temp_staging"] {
        fs::create_dir_all(root.join(d))?;
    }
    open_db(root)?;
    write_bootstrap(&Bootstrap {
        schema_version: 1,
        active_data_root: data_root.into(),
        migration_state: "IDLE".into(),
        pending_migration_target: None,
        previous_data_root: None,
    })
}

/// Opens the DB from bootstrap on app start. Returns false when first-run setup is needed.
pub fn open_existing() -> Result<bool> {
    // A move that was interrupted (power cut, crash) is finished or undone first.
    crate::migrate::recover()?;
    match read_bootstrap() {
        Some(b) if Path::new(&b.active_data_root).join("db").exists() => {
            open_db(Path::new(&b.active_data_root))?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub(crate) fn connect(root: &Path) -> Result<Connection> {
    let conn = Connection::open(root.join("db").join("mail_core.sqlite"))?;
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

fn open_db(root: &Path) -> Result<()> {
    *DB.lock().unwrap() = Some(connect(root)?);
    Ok(())
}

/// Small in-place upgrades for databases created by older versions.
fn migrate(conn: &Connection) -> Result<()> {
    let has = |table: &str, col: &str| -> Result<bool> {
        let mut st = conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let names: Vec<String> = st.query_map([], |r| r.get(1))?.collect::<Result<_, _>>()?;
        Ok(names.iter().any(|n| n == col))
    };
    if !has("messages", "inline_done")? {
        // 0 = embedded pictures not fetched yet (checked on the next sync).
        conn.execute_batch("ALTER TABLE messages ADD COLUMN inline_done INTEGER DEFAULT 0")?;
    }
    if !has("offline_mutation_queue", "failed")? {
        // 1 = the server turned this change down; it stays listed so nothing is lost silently.
        conn.execute_batch(
            "ALTER TABLE offline_mutation_queue ADD COLUMN failed INTEGER DEFAULT 0",
        )?;
    }
    Ok(())
}

/// Tests share one global database, so they take turns.
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

pub fn with_db<T>(f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let g = DB.lock().unwrap();
    f(g.as_ref().ok_or(anyhow!("Database not open"))?)
}

pub fn blob_dir() -> Result<PathBuf> {
    Ok(Path::new(
        &read_bootstrap()
            .ok_or(anyhow!("Not set up"))?
            .active_data_root,
    )
    .join("blobs"))
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    parent_remote_id TEXT,
    display_name TEXT NOT NULL,
    well_known_name TEXT,
    delta_link TEXT,
    unread_count INTEGER DEFAULT 0,
    total_count INTEGER DEFAULT 0,
    UNIQUE(account_id, remote_id)
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    remote_id TEXT UNIQUE,
    internet_message_id TEXT,
    conversation_id TEXT,
    subject TEXT,
    sender_name TEXT,
    sender_address TEXT,
    to_recipients_json TEXT,
    cc_recipients_json TEXT,
    snippet TEXT,
    received_at INTEGER NOT NULL,
    is_read INTEGER DEFAULT 0,
    is_flagged INTEGER DEFAULT 0,
    is_draft INTEGER DEFAULT 0,
    has_attachments INTEGER DEFAULT 0,
    body_blob_hash TEXT,
    body_cached INTEGER DEFAULT 0,
    change_key TEXT,
    sync_status TEXT DEFAULT 'SYNCED'
);
CREATE INDEX IF NOT EXISTS idx_messages_folder_date ON messages(folder_id, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(account_id, conversation_id);

CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    message_id UNINDEXED, subject, sender_name, sender_address, body_plaintext,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    remote_attachment_id TEXT,
    filename TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    content_id TEXT,
    is_inline INTEGER DEFAULT 0,
    blob_hash TEXT,
    is_downloaded INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS calendars (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    color_hex TEXT NOT NULL,
    is_primary INTEGER DEFAULT 0,
    can_edit INTEGER DEFAULT 1,
    delta_link TEXT
);

CREATE TABLE IF NOT EXISTS calendar_events (
    id TEXT PRIMARY KEY,
    calendar_id TEXT NOT NULL REFERENCES calendars(id) ON DELETE CASCADE,
    remote_id TEXT UNIQUE,
    ical_uid TEXT NOT NULL,
    series_master_id TEXT,
    event_type TEXT NOT NULL,
    title TEXT NOT NULL,
    description_html TEXT,
    location TEXT,
    online_meeting_url TEXT,
    start_utc INTEGER NOT NULL,
    end_utc INTEGER NOT NULL,
    original_timezone TEXT NOT NULL,
    is_all_day INTEGER DEFAULT 0,
    organizer_json TEXT,
    attendees_json TEXT,
    my_response_status TEXT DEFAULT 'NONE',
    change_key TEXT,
    sync_status TEXT DEFAULT 'SYNCED'
);
CREATE INDEX IF NOT EXISTS idx_calendar_events_range ON calendar_events(calendar_id, start_utc, end_utc);

CREATE TABLE IF NOT EXISTS offline_mutation_queue (
    seq_id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    entity_type TEXT NOT NULL,
    local_entity_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    retry_count INTEGER DEFAULT 0,
    next_retry_at INTEGER DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn schema_applies() {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch(super::SCHEMA).unwrap();
        c.execute_batch(super::SCHEMA).unwrap(); // idempotent
    }
}
