//! Offline-first mutations: apply locally at once, queue for Graph, replay (with coalescing) before pulling deltas.

use crate::{auth, storage};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn account_id() -> Result<String> {
    storage::with_db(|db| {
        Ok(db
            .query_row("SELECT id FROM accounts ORDER BY created_at LIMIT 1", [], |r| r.get(0))
            .optional()?
            .ok_or(anyhow!("No account"))?)
    })
}

fn enqueue(db: &rusqlite::Connection, entity: &str, op: &str, payload: Value) -> Result<()> {
    db.execute(
        "INSERT INTO offline_mutation_queue(account_id,entity_type,local_entity_id,operation,payload_json,created_at)
         VALUES((SELECT id FROM accounts ORDER BY created_at LIMIT 1),'MESSAGE',?1,?2,?3,?4)",
        params![entity, op, payload.to_string(), now()],
    )?;
    Ok(())
}

pub fn set_read(id: &str, read: bool) -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE messages SET is_read=?1, sync_status='DIRTY_LOCAL' WHERE id=?2 AND is_read<>?1",
            params![read, id],
        )?;
        if changed > 0 {
            tx.execute(
                "UPDATE folders SET unread_count=MAX(0, unread_count + ?1) WHERE id=(SELECT folder_id FROM messages WHERE id=?2)",
                params![if read { -1 } else { 1 }, id],
            )?;
            enqueue(&tx, id, "SET_READ", json!({ "read": read }))?;
        }
        tx.commit()?;
        Ok(())
    })
}

pub fn set_flag(id: &str, flagged: bool) -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        tx.execute("UPDATE messages SET is_flagged=?1, sync_status='DIRTY_LOCAL' WHERE id=?2", params![flagged, id])?;
        enqueue(&tx, id, "SET_FLAG", json!({ "flagged": flagged }))?;
        tx.commit()?;
        Ok(())
    })
}

/// Move to a local folder id. Deleting = moving to Deleted Items (or hard delete when already there).
pub fn move_to(id: &str, folder_id: &str) -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        let (remote_folder, unread): (String, bool) = tx.query_row(
            "SELECT f.remote_id, m.is_read=0 FROM folders f, messages m WHERE f.id=?1 AND m.id=?2",
            params![folder_id, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if unread {
            tx.execute("UPDATE folders SET unread_count=MAX(0,unread_count-1) WHERE id=(SELECT folder_id FROM messages WHERE id=?1)", [id])?;
            tx.execute("UPDATE folders SET unread_count=unread_count+1 WHERE id=?1", [folder_id])?;
        }
        tx.execute("UPDATE messages SET folder_id=?1, sync_status='DIRTY_LOCAL' WHERE id=?2", params![folder_id, id])?;
        enqueue(&tx, id, "MOVE", json!({ "destination": remote_folder }))?;
        tx.commit()?;
        Ok(())
    })
}

pub fn delete(id: &str) -> Result<()> {
    let (in_trash, trash): (bool, Option<String>) = storage::with_db(|db| {
        Ok(db.query_row(
            "SELECT COALESCE(f.well_known_name='deleteditems',0),
                    (SELECT id FROM folders WHERE well_known_name='deleteditems' AND account_id=m.account_id)
             FROM messages m JOIN folders f ON f.id=m.folder_id WHERE m.id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    match (in_trash, trash) {
        (false, Some(t)) => move_to(id, &t),
        _ => storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            let rid: Option<String> = tx.query_row("SELECT remote_id FROM messages WHERE id=?1", [id], |r| r.get(0))?;
            tx.execute("DELETE FROM messages_fts WHERE message_id=?1", [id])?;
            tx.execute("DELETE FROM messages WHERE id=?1", [id])?;
            if let Some(rid) = rid {
                enqueue(&tx, id, "DELETE_HARD", json!({ "remote_id": rid }))?;
            }
            tx.commit()?;
            Ok(())
        }),
    }
}

#[derive(Deserialize)]
pub struct Draft {
    pub mode: String, // "new" | "reply" | "replyAll" | "forward"
    pub reply_to: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub html: String,
}

/// Queue an outgoing message; it is sent immediately when online, otherwise on reconnect.
pub fn send(d: Draft) -> Result<()> {
    let _ = account_id()?;
    let entity = d.reply_to.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    storage::with_db(|db| {
        enqueue(
            db,
            &entity,
            "SEND",
            json!({ "mode": d.mode, "to": d.to, "cc": d.cc, "subject": d.subject, "html": d.html }),
        )
    })
}

pub fn pending_count() -> Result<i64> {
    storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM offline_mutation_queue", [], |r| r.get(0))?))
}

struct Item {
    seq: i64,
    entity: String,
    op: String,
    payload: Value,
}

/// Drop operations made redundant by a later one on the same message (read/unread toggles, flag toggles).
fn coalesce(db: &rusqlite::Connection) -> Result<()> {
    db.execute_batch(
        "DELETE FROM offline_mutation_queue WHERE operation IN ('SET_READ','SET_FLAG') AND seq_id NOT IN
           (SELECT MAX(seq_id) FROM offline_mutation_queue WHERE operation IN ('SET_READ','SET_FLAG') GROUP BY local_entity_id, operation);",
    )?;
    Ok(())
}

fn recipients(list: &Value) -> Value {
    Value::Array(
        list.as_array()
            .map(|a| a.iter().filter_map(|e| e.as_str()).map(|e| json!({ "emailAddress": { "address": e } })).collect())
            .unwrap_or_default(),
    )
}

async fn call(method: reqwest::Method, url: &str, body: Option<Value>) -> Result<(u16, Value)> {
    let token = auth::access_token().await?;
    let mut req = reqwest::Client::new().request(method, url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    let v = resp.json().await.unwrap_or(Value::Null);
    Ok((status, v))
}

/// Replays the queue in order. Stops at the first network-level failure so ordering is preserved.
pub async fn replay() -> Result<usize> {
    storage::with_db(coalesce)?;
    let items: Vec<Item> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT seq_id, local_entity_id, operation, payload_json FROM offline_mutation_queue
             WHERE next_retry_at <= ?1 ORDER BY seq_id",
        )?;
        let rows = st.query_map([now()], |r| {
            Ok(Item {
                seq: r.get(0)?,
                entity: r.get(1)?,
                op: r.get(2)?,
                payload: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or(Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;

    let mut done = 0;
    for it in items {
        let rid: Option<String> = storage::with_db(|db| {
            Ok(db
                .query_row("SELECT remote_id FROM messages WHERE id=?1", [&it.entity], |r| r.get(0))
                .optional()?
                .flatten())
        })?;
        let rid = rid.or_else(|| it.payload["remote_id"].as_str().map(String::from));
        let p = &it.payload;
        use reqwest::Method;
        let result = match (it.op.as_str(), &rid) {
            ("SET_READ", Some(r)) => call(Method::PATCH, &format!("{GRAPH}/me/messages/{r}"), Some(json!({ "isRead": p["read"] }))).await,
            ("SET_FLAG", Some(r)) => {
                let s = if p["flagged"].as_bool() == Some(true) { "flagged" } else { "notFlagged" };
                call(Method::PATCH, &format!("{GRAPH}/me/messages/{r}"), Some(json!({ "flag": { "flagStatus": s } }))).await
            }
            ("MOVE", Some(r)) => call(Method::POST, &format!("{GRAPH}/me/messages/{r}/move"), Some(json!({ "destinationId": p["destination"] }))).await,
            ("DELETE_HARD", Some(r)) => call(Method::DELETE, &format!("{GRAPH}/me/messages/{r}"), None).await,
            ("SEND", _) => {
                let msg = json!({
                    "subject": p["subject"], "body": { "contentType": "HTML", "content": p["html"] },
                    "toRecipients": recipients(&p["to"]), "ccRecipients": recipients(&p["cc"]),
                });
                match (p["mode"].as_str().unwrap_or("new"), &rid) {
                    ("new", _) | (_, None) => call(Method::POST, &format!("{GRAPH}/me/sendMail"), Some(json!({ "message": msg, "saveToSentItems": true }))).await,
                    (mode, Some(r)) => {
                        // Graph threads replies/forwards correctly when using these endpoints.
                        call(Method::POST, &format!("{GRAPH}/me/messages/{r}/{mode}"), Some(json!({ "message": msg }))).await
                    }
                }
            }
            (_, None) => Ok((404, Value::Null)), // message no longer exists remotely
            _ => Ok((400, Value::Null)),
        };

        match result {
            Ok((s, v)) if (200..300).contains(&s) || s == 404 => {
                storage::with_db(|db| {
                    db.execute("DELETE FROM offline_mutation_queue WHERE seq_id=?1", [it.seq])?;
                    if it.op == "MOVE" {
                        // Graph gives moved messages a new id; keep our local id stable.
                        if let Some(new_id) = v["id"].as_str() {
                            db.execute("UPDATE messages SET remote_id=?1 WHERE id=?2", params![new_id, it.entity])?;
                        }
                    }
                    let still_dirty: i64 = db.query_row(
                        "SELECT COUNT(*) FROM offline_mutation_queue WHERE local_entity_id=?1",
                        [&it.entity],
                        |r| r.get(0),
                    )?;
                    if still_dirty == 0 {
                        db.execute("UPDATE messages SET sync_status='SYNCED' WHERE id=?1", [&it.entity])?;
                    }
                    Ok(())
                })?;
                done += 1;
            }
            Ok((s, v)) => {
                // Server rejected it: back off, keep the error for the UI, give up after 8 tries.
                let msg = format!("{s}: {}", v["error"]["message"].as_str().unwrap_or(""));
                storage::with_db(|db| {
                    db.execute(
                        "UPDATE offline_mutation_queue SET retry_count=retry_count+1, last_error=?1,
                           next_retry_at=?2 + (60 * (1 << MIN(retry_count, 6))) WHERE seq_id=?3",
                        params![msg, now(), it.seq],
                    )?;
                    db.execute("DELETE FROM offline_mutation_queue WHERE seq_id=?1 AND retry_count>=8", [it.seq])?;
                    Ok(())
                })?;
            }
            Err(e) => return if done > 0 { Ok(done) } else { Err(e) }, // offline: try again later
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    #[test]
    fn coalesces_toggles() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE offline_mutation_queue(seq_id INTEGER PRIMARY KEY AUTOINCREMENT, account_id TEXT, entity_type TEXT,
               local_entity_id TEXT, operation TEXT, payload_json TEXT, retry_count INT DEFAULT 0, next_retry_at INT DEFAULT 0,
               last_error TEXT, created_at INT);
             INSERT INTO offline_mutation_queue(local_entity_id,operation,payload_json) VALUES
               ('m1','SET_READ','{\"read\":true}'),('m1','SET_READ','{\"read\":false}'),('m1','SET_READ','{\"read\":true}'),
               ('m1','MOVE','{}'),('m2','SET_READ','{\"read\":true}');",
        )
        .unwrap();
        super::coalesce(&db).unwrap();
        let n: i64 = db.query_row("SELECT COUNT(*) FROM offline_mutation_queue", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3); // one read for m1, the move, one read for m2
        let last: String = db
            .query_row("SELECT payload_json FROM offline_mutation_queue WHERE local_entity_id='m1' AND operation='SET_READ'", [], |r| r.get(0))
            .unwrap();
        assert!(last.contains("true"));
    }
}
