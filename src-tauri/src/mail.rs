//! Read-side queries for the UI, served entirely from the local database (works offline).

use crate::{storage, sync};
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

#[derive(Serialize)]
pub struct Folder {
    id: String,
    remote_id: String,
    parent_remote_id: Option<String>,
    name: String,
    well_known: Option<String>,
    unread: i64,
    total: i64,
}

#[derive(Serialize)]
pub struct MessageRow {
    id: String,
    subject: Option<String>,
    sender_name: Option<String>,
    sender_address: Option<String>,
    snippet: Option<String>,
    received_at: i64,
    is_read: bool,
    is_flagged: bool,
    has_attachments: bool,
}

#[derive(Serialize)]
pub struct MessageDetail {
    #[serde(flatten)]
    row: MessageRow,
    to_json: Option<String>,
    cc_json: Option<String>,
    body_html: Option<String>,
}

const ROW_COLS: &str = "m.id, m.subject, m.sender_name, m.sender_address, m.snippet, m.received_at, m.is_read, m.is_flagged, m.has_attachments";

fn map_row(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: r.get(0)?,
        subject: r.get(1)?,
        sender_name: r.get(2)?,
        sender_address: r.get(3)?,
        snippet: r.get(4)?,
        received_at: r.get(5)?,
        is_read: r.get(6)?,
        is_flagged: r.get(7)?,
        has_attachments: r.get(8)?,
    })
}

pub fn folders() -> Result<Vec<Folder>> {
    storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT id, remote_id, parent_remote_id, display_name, well_known_name, unread_count, total_count FROM folders
             ORDER BY CASE well_known_name WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sentitems' THEN 2
               WHEN 'archive' THEN 3 WHEN 'deleteditems' THEN 4 WHEN 'junkemail' THEN 5 ELSE 9 END, display_name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Folder {
                id: r.get(0)?,
                remote_id: r.get(1)?,
                parent_remote_id: r.get(2)?,
                name: r.get(3)?,
                well_known: r.get(4)?,
                unread: r.get(5)?,
                total: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn messages(folder_id: &str, offset: i64, limit: i64) -> Result<Vec<MessageRow>> {
    storage::with_db(|db| {
        let mut st = db.prepare(&format!(
            "SELECT {ROW_COLS} FROM messages m WHERE m.folder_id=?1 ORDER BY m.received_at DESC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = st.query_map(params![folder_id, limit, offset], map_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn search(q: &str, limit: i64) -> Result<Vec<MessageRow>> {
    // Quote each term so user input can't break FTS5 syntax; prefix-match the last term.
    let terms: Vec<String> = q.split_whitespace().map(|t| format!("\"{}\"", t.replace('"', ""))).collect();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let fts = format!("{}*", terms.join(" "));
    storage::with_db(|db| {
        let mut st = db.prepare(&format!(
            "SELECT {ROW_COLS} FROM messages_fts f JOIN messages m ON m.id=f.message_id
             WHERE messages_fts MATCH ?1 ORDER BY m.received_at DESC LIMIT ?2"
        ))?;
        let rows = st.query_map(params![fts, limit], map_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn message(id: &str) -> Result<MessageDetail> {
    let (row, to, cc, hash): (MessageRow, Option<String>, Option<String>, Option<String>) = storage::with_db(|db| {
        Ok(db.query_row(
            &format!("SELECT {ROW_COLS}, m.to_recipients_json, m.cc_recipients_json, m.body_blob_hash FROM messages m WHERE m.id=?1"),
            [id],
            |r| Ok((map_row(r)?, r.get(9)?, r.get(10)?, r.get(11)?)),
        )?)
    })?;
    let body_html = hash.and_then(|h| sync::read_blob(&h, "html").ok()).map(|b| String::from_utf8_lossy(&b).into_owned());
    Ok(MessageDetail { row, to_json: to, cc_json: cc, body_html })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_queries_and_search() {
        let tmp = std::env::temp_dir().join(format!("gm-test-{}", uuid::Uuid::new_v4()));
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg")) };
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('f','a','rf','Inbox','inbox');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at) VALUES('m1','a','f','r1','Budget review','Asim',100);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,received_at) VALUES('m2','a','f','r2','Lunch','Bob',200);
                 INSERT INTO messages_fts(message_id,subject,sender_name,sender_address,body_plaintext) VALUES('m1','Budget review','Asim','','capex numbers for 2027');",
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(folders().unwrap().len(), 1);
        let ms = messages("f", 0, 50).unwrap();
        assert_eq!(ms[0].id, "m2"); // newest first
        assert_eq!(search("capex", 10).unwrap()[0].id, "m1");
        assert_eq!(search("bud", 10).unwrap().len(), 1); // prefix match
        assert!(search("\"broken( query", 10).is_ok()); // hostile input is safe
    }
}
