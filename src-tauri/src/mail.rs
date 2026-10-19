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
    account_email: String,
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
    pub(crate) body_html: Option<String>,
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
            "SELECT f.id, f.remote_id, f.parent_remote_id, f.display_name, f.well_known_name, f.unread_count, f.total_count, a.email
             FROM folders f JOIN accounts a ON a.id=f.account_id
             ORDER BY a.created_at, CASE well_known_name WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sentitems' THEN 2
               WHEN 'archive' THEN 3 WHEN 'deleteditems' THEN 4 WHEN 'junkemail' THEN 5 ELSE 9 END, f.display_name",
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
                account_email: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Messages in one folder, newest first. The special id "all-inboxes" combines every account's Inbox.
pub fn messages(folder_id: &str, offset: i64, limit: i64) -> Result<Vec<MessageRow>> {
    storage::with_db(|db| {
        let filter = if folder_id == "all-inboxes" {
            "m.folder_id IN (SELECT id FROM folders WHERE well_known_name='inbox' AND ?1 IS NOT NULL)"
        } else {
            "m.folder_id=?1"
        };
        let mut st = db.prepare(&format!(
            "SELECT {ROW_COLS} FROM messages m WHERE {filter} ORDER BY m.received_at DESC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = st.query_map(params![folder_id, limit, offset], map_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn search(q: &str, limit: i64) -> Result<Vec<MessageRow>> {
    // Quote each term so user input can't break FTS5 syntax; prefix-match the last term.
    let terms: Vec<String> = q
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
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
    let (row, to, cc, hash): (MessageRow, Option<String>, Option<String>, Option<String>) =
        storage::with_db(|db| {
            Ok(db.query_row(
            &format!("SELECT {ROW_COLS}, m.to_recipients_json, m.cc_recipients_json, m.body_blob_hash FROM messages m WHERE m.id=?1"),
            [id],
            |r| Ok((map_row(r)?, r.get(9)?, r.get(10)?, r.get(11)?)),
        )?)
        })?;
    let body_html = hash
        .and_then(|h| sync::read_blob(&h, "html").ok())
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let body_html = body_html.map(|html| {
        if !sync::has_cid(&html) {
            return html;
        }
        let pics: Vec<(String, String, Vec<u8>)> = storage::with_db(|db| {
            let mut st = db.prepare("SELECT content_id, mime_type, blob_hash FROM attachments WHERE message_id=?1 AND is_inline=1 AND blob_hash IS NOT NULL")?;
            let rows = st.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(cid, mime, hash)| Some((cid, mime, sync::read_blob(&hash, "bin").ok()?)))
        .collect();
        inline_cids(&html, &pics)
    });
    Ok(MessageDetail {
        row,
        to_json: to,
        cc_json: cc,
        body_html,
    })
}

/// Replaces `cid:` picture references with the downloaded pictures, embedded as data: URLs.
fn inline_cids(html: &str, pics: &[(String, String, Vec<u8>)]) -> String {
    use base64::Engine;
    let mut out = html.to_string();
    for (cid, mime, bytes) in pics {
        let cid = cid.trim_matches(|c| c == '<' || c == '>');
        let data = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        // Mail apps write the id as-is or percent-encoded, in any letter case.
        for key in [
            cid.to_string(),
            url::form_urlencoded::byte_serialize(cid.as_bytes()).collect::<String>(),
        ] {
            let needle = format!("cid:{key}").to_ascii_lowercase();
            let mut start = 0;
            while let Some(pos) = out.to_ascii_lowercase()[start..].find(&needle) {
                let at = start + pos;
                out.replace_range(at..at + needle.len(), &data);
                start = at + data.len();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_cid_pictures() {
        let html = r#"<img src="cid:logo@01D"><img src="CID:Logo@01D"><img src="cid:other">"#;
        let out = inline_cids(
            html,
            &[("<logo@01D>".into(), "image/png".into(), vec![1, 2, 3])],
        );
        assert_eq!(out.matches("data:image/png;base64,AQID").count(), 2);
        assert!(out.contains("cid:other")); // unknown pictures are left alone
    }

    #[test]
    fn local_queries_and_search() {
        let _guard = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    #[test]
    fn two_accounts_all_inboxes_and_queue_ownership() {
        let _guard = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("gm-multi-{}", uuid::Uuid::new_v4()));
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg")) };
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a1','work@x.com','Work',1),('a2','home@y.com','Home',2);
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('i1','a1','r1','Inbox','inbox'),('i2','a2','r2','Inbox','inbox'),('p2','a2','r3','Projects',NULL);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at) VALUES
                   ('w','a1','i1','m1','Work mail',100),('h','a2','i2','m2','Home mail',200),('x','a2','p2','m3','Elsewhere',300);",
            )?;
            Ok(())
        })
        .unwrap();
        let all: Vec<String> = messages("all-inboxes", 0, 50).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(all, vec!["h", "w"]); // both inboxes, newest first, other folders excluded
        let f = folders().unwrap();
        assert_eq!(f.iter().filter(|f| f.account_email == "home@y.com").count(), 2);
        // A change to the home account's message is queued under the home account.
        crate::outbox::set_flag("h", true).unwrap();
        let owner: String = storage::with_db(|db| Ok(db.query_row("SELECT account_id FROM offline_mutation_queue", [], |r| r.get(0))?)).unwrap();
        assert_eq!(owner, "a2");
    }
}
