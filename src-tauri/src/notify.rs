//! Desktop notifications for newly arrived Inbox mail.
//! A small key/value table remembers the newest message already announced, so nothing is shown twice
//! and the very first sync (the whole mailbox) never floods the desktop.

use crate::storage;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use tauri_plugin_notification::NotificationExt;

pub fn get_setting(key: &str) -> Option<String> {
    storage::with_db(|db| {
        db.execute_batch("CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")?;
        Ok(db.query_row("SELECT value FROM app_settings WHERE key=?1", [key], |r| r.get(0)).optional()?)
    })
    .ok()
    .flatten()
}

pub fn set_setting(key: &str, value: &str) -> Result<()> {
    storage::with_db(|db| {
        db.execute_batch("CREATE TABLE IF NOT EXISTS app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")?;
        db.execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    })
}

pub fn enabled() -> bool {
    get_setting("notifications").as_deref() != Some("off")
}

#[derive(Debug, PartialEq)]
pub struct NewMail {
    pub sender: String,
    pub subject: String,
}

/// Unread Inbox messages newer than the last announced one; advances the marker.
pub fn collect_new() -> Result<Vec<NewMail>> {
    let newest: Option<i64> = storage::with_db(|db| {
        Ok(db.query_row(
            "SELECT MAX(m.received_at) FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.well_known_name='inbox'",
            [],
            |r| r.get(0),
        )?)
    })?;
    let Some(newest) = newest else { return Ok(vec![]) };
    let marker = get_setting("notified_up_to").and_then(|v| v.parse::<i64>().ok());
    set_setting("notified_up_to", &newest.to_string())?;
    let Some(since) = marker else { return Ok(vec![]) }; // first run: just remember where we are
    storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT COALESCE(m.sender_name, m.sender_address, 'Someone'), COALESCE(m.subject, '(no subject)')
             FROM messages m JOIN folders f ON f.id=m.folder_id
             WHERE f.well_known_name='inbox' AND m.is_read=0 AND m.received_at > ?1
             ORDER BY m.received_at DESC LIMIT 50",
        )?;
        let rows = st.query_map([since], |r| Ok(NewMail { sender: r.get(0)?, subject: r.get(1)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Shows one notification per message, or a single summary when many arrive at once.
pub fn announce(app: &tauri::AppHandle) {
    if !enabled() {
        let _ = collect_new(); // keep the marker moving so turning it back on doesn't replay old mail
        return;
    }
    let new = match collect_new() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[graphite] notification check failed: {e}");
            return;
        }
    };
    let show = |title: &str, body: &str| {
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            eprintln!("[graphite] notification failed: {e}");
        }
    };
    match new.len() {
        0 => {}
        1..=3 => new.iter().rev().for_each(|m| show(&m.sender, &m.subject)),
        n => show(&format!("{n} new messages"), &format!("Latest from {}: {}", new[0].sender, new[0].subject)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_is_silent_then_reports_only_new_unread() {
        let _guard = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("gm-notify-{}", uuid::Uuid::new_v4()));
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg")) };
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('in','a','rin','Inbox','inbox');
                 INSERT INTO folders(id,account_id,remote_id,display_name) VALUES('p','a','rp','Projects');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at) VALUES('old','a','in','r1','Old','Ann',100);",
            )?;
            Ok(())
        })
        .unwrap();
        assert!(collect_new().unwrap().is_empty()); // first sync: silent
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at,is_read) VALUES('n1','a','in','r2','Hello','Bob',200,0);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at,is_read) VALUES('n2','a','in','r3','Seen','Cy',210,1);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at,is_read) VALUES('n3','a','p','r4','Other folder','Di',220,0);",
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(collect_new().unwrap(), vec![NewMail { sender: "Bob".into(), subject: "Hello".into() }]);
        assert!(collect_new().unwrap().is_empty()); // nothing announced twice
    }
}
