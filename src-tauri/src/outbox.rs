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
    storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM offline_mutation_queue WHERE retry_count<?1", [FAILED], |r| r.get(0))?))
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

/// Queue rows with this retry_count have failed for good: they are kept (with last_error) but no longer retried.
const FAILED: i64 = 100;
/// Transient failures are retried this many times (with growing back-off) before being treated as failed.
const MAX_TRIES: i64 = 8;

/// Graph answers that will not change on retry (bad request, forbidden, conflict, ...). 401 is excluded because a
/// token refresh fixes it, and 408/429/5xx are transient.
fn is_permanent(status: u16) -> bool {
    matches!(status, 400 | 403 | 405 | 409 | 410 | 412 | 413 | 422)
}

/// Undo the optimistic local change behind a queue item that can never be applied: let the server's
/// version win again on the next sync.
fn revert_local(db: &rusqlite::Connection, entity: &str) -> Result<()> {
    db.execute("UPDATE messages SET sync_status='SYNCED' WHERE id=?1", [entity])?;
    db.execute("UPDATE calendar_events SET sync_status='SYNCED' WHERE id=?1", [entity])?;
    db.execute("UPDATE folders SET delta_link=NULL", [])?; // full re-pull so moved/read/flagged state is restored
    Ok(())
}

/// Replays the queue in order. Stops at the first network-level failure so ordering is preserved.
pub async fn replay() -> Result<usize> {
    replay_at(GRAPH).await
}

async fn replay_at(base: &str) -> Result<usize> {
    storage::with_db(coalesce)?;
    let items: Vec<Item> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT seq_id, local_entity_id, operation, payload_json FROM offline_mutation_queue
             WHERE next_retry_at <= ?1 AND retry_count < ?2 ORDER BY seq_id",
        )?;
        let rows = st.query_map(params![now(), FAILED], |r| {
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
                .query_row(
                    "SELECT remote_id FROM messages WHERE id=?1 UNION ALL SELECT remote_id FROM calendar_events WHERE id=?1 LIMIT 1",
                    [&it.entity],
                    |r| r.get(0),
                )
                .optional()?
                .flatten())
        })?;
        let rid = rid.or_else(|| it.payload["remote_id"].as_str().map(String::from));
        let p = &it.payload;
        use reqwest::Method;
        let result = match (it.op.as_str(), &rid) {
            ("SET_READ", Some(r)) => call(Method::PATCH, &format!("{base}/me/messages/{r}"), Some(json!({ "isRead": p["read"] }))).await,
            ("SET_FLAG", Some(r)) => {
                let s = if p["flagged"].as_bool() == Some(true) { "flagged" } else { "notFlagged" };
                call(Method::PATCH, &format!("{base}/me/messages/{r}"), Some(json!({ "flag": { "flagStatus": s } }))).await
            }
            ("MOVE", Some(r)) => call(Method::POST, &format!("{base}/me/messages/{r}/move"), Some(json!({ "destinationId": p["destination"] }))).await,
            ("DELETE_HARD", Some(r)) => call(Method::DELETE, &format!("{base}/me/messages/{r}"), None).await,
            ("SEND", _) => {
                let msg = json!({
                    "subject": p["subject"], "body": { "contentType": "HTML", "content": p["html"] },
                    "toRecipients": recipients(&p["to"]), "ccRecipients": recipients(&p["cc"]),
                });
                match (p["mode"].as_str().unwrap_or("new"), &rid) {
                    ("new", _) | (_, None) => call(Method::POST, &format!("{base}/me/sendMail"), Some(json!({ "message": msg, "saveToSentItems": true }))).await,
                    (mode, Some(r)) => {
                        // Graph threads replies/forwards correctly when using these endpoints.
                        call(Method::POST, &format!("{base}/me/messages/{r}/{mode}"), Some(json!({ "message": msg }))).await
                    }
                }
            }
            ("RSVP", Some(r)) => {
                let resp = p["response"].as_str().unwrap_or("accept");
                call(Method::POST, &format!("{base}/me/events/{r}/{resp}"), Some(json!({ "sendResponse": true }))).await
            }
            ("CREATE_EVENT", _) => {
                let cal = p["calendar"].as_str().unwrap_or_default();
                call(Method::POST, &format!("{base}/me/calendars/{cal}/events"), Some(p["event"].clone())).await
            }
            ("DELETE_EVENT", Some(r)) => call(Method::DELETE, &format!("{base}/me/events/{r}"), None).await,
            (_, None) => Ok((404, Value::Null)), // item no longer exists remotely
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
                    if it.op == "CREATE_EVENT" {
                        if let Some(new_id) = v["id"].as_str() {
                            db.execute(
                                "UPDATE calendar_events SET remote_id=?1, online_meeting_url=?2 WHERE id=?3",
                                params![new_id, v["onlineMeeting"]["joinUrl"].as_str(), it.entity],
                            )?;
                        }
                    }
                    let still_dirty: i64 = db.query_row(
                        "SELECT COUNT(*) FROM offline_mutation_queue WHERE local_entity_id=?1",
                        [&it.entity],
                        |r| r.get(0),
                    )?;
                    if still_dirty == 0 {
                        db.execute("UPDATE messages SET sync_status='SYNCED' WHERE id=?1", [&it.entity])?;
                        db.execute("UPDATE calendar_events SET sync_status='SYNCED' WHERE id=?1", [&it.entity])?;
                    }
                    Ok(())
                })?;
                done += 1;
            }
            Ok((s, v)) => {
                // Server rejected it. Keep the reason for the UI and the terminal.
                let code = v["error"]["code"].as_str().unwrap_or("");
                let msg = format!("{s} {code}: {}", v["error"]["message"].as_str().unwrap_or("")).trim().to_string();
                eprintln!("[graphite] queued {} for {} rejected: {msg}", it.op, it.entity);
                storage::with_db(|db| {
                    let tries: i64 = db.query_row("SELECT retry_count FROM offline_mutation_queue WHERE seq_id=?1", [it.seq], |r| r.get(0))?;
                    if is_permanent(s) || tries + 1 >= MAX_TRIES {
                        db.execute(
                            "UPDATE offline_mutation_queue SET retry_count=?1, last_error=?2, next_retry_at=?3 WHERE seq_id=?4",
                            params![FAILED, msg, i64::MAX, it.seq],
                        )?;
                        revert_local(db, &it.entity)?;
                    } else {
                        db.execute(
                            "UPDATE offline_mutation_queue SET retry_count=retry_count+1, last_error=?1, next_retry_at=?2 + (60 * (1 << MIN(retry_count, 6))) WHERE seq_id=?3",
                            params![msg, now(), it.seq],
                        )?;
                    }
                    Ok(())
                })?;
            }
            Err(e) => {
                // Offline / token problem: keep the item, remember why, and try again later.
                eprintln!("[graphite] queued {} for {} not sent: {e}", it.op, it.entity);
                let _ = storage::with_db(|db| Ok(db.execute("UPDATE offline_mutation_queue SET last_error=?1 WHERE seq_id=?2", params![e.to_string(), it.seq])?));
                return if done > 0 { Ok(done) } else { Err(e) };
            }
        }
    }
    Ok(done)
}

#[derive(serde::Serialize)]
pub struct FailedAction {
    pub seq: i64,
    pub operation: String,
    pub error: String,
}

/// Queue items that Graph rejected for good (they stay listed until the user discards them).
pub fn failed_actions() -> Result<Vec<FailedAction>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT seq_id, operation, COALESCE(last_error,'') FROM offline_mutation_queue WHERE retry_count>=?1 ORDER BY seq_id")?;
        let rows = st.query_map([FAILED], |r| Ok(FailedAction { seq: r.get(0)?, operation: r.get(1)?, error: r.get(2)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

pub fn discard_failed() -> Result<usize> {
    storage::with_db(|db| Ok(db.execute("DELETE FROM offline_mutation_queue WHERE retry_count>=?1", [FAILED])?))
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

    use super::*;
    use crate::testutil::{json_body, mock_graph};
    use serde_json::json;

    fn vault() {
        let _ = storage::init_test_vault();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name,delta_link) VALUES('f','a','RF','Inbox','inbox','https://delta/1');
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name,delta_link) VALUES('f2','a','RF2','Archive','archive','https://delta/2');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at,is_read) VALUES('m1','a','f','R1','One',1,0);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at,is_read) VALUES('m2','a','f','R2','Two',2,0);",
            )?;
            Ok(())
        })
        .unwrap();
        *auth::TEST_TOKEN.lock().unwrap() = Some("t".into());
    }

    fn row(seq_of: &str) -> (i64, i64, String) {
        storage::with_db(|db| {
            Ok(db.query_row(
                "SELECT retry_count, next_retry_at, COALESCE(last_error,'') FROM offline_mutation_queue WHERE local_entity_id=?1",
                [seq_of],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap()
    }

    fn sync_status(id: &str) -> String {
        storage::with_db(|db| Ok(db.query_row("SELECT sync_status FROM messages WHERE id=?1", [id], |r| r.get(0))?)).unwrap()
    }

    fn delta(id: &str) -> Option<String> {
        storage::with_db(|db| Ok(db.query_row("SELECT delta_link FROM folders WHERE id=?1", [id], |r| r.get(0))?)).unwrap()
    }

    fn err_body(code: &str, msg: &str) -> Vec<u8> {
        json_body(json!({ "error": { "code": code, "message": msg } }))
    }

    #[test]
    fn classifies_statuses() {
        for s in [400, 403, 405, 409, 410, 412, 413, 422] {
            assert!(is_permanent(s), "{s}");
        }
        for s in [401, 408, 429, 500, 502, 503, 504] {
            assert!(!is_permanent(s), "{s}");
        }
    }

    #[tokio::test]
    async fn success_clears_the_item() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        assert_eq!(pending_count().unwrap(), 1);
        let m = mock_graph(vec![("/me/messages/R1", 200, b"{}".to_vec())]).await;
        assert_eq!(replay_at(&m.base).await.unwrap(), 1);
        assert_eq!(pending_count().unwrap(), 0);
        assert_eq!(sync_status("m1"), "SYNCED");
        assert_eq!(m.hits.lock().unwrap().as_slice(), ["PATCH /me/messages/R1"]);
    }

    #[tokio::test]
    async fn already_gone_on_the_server_counts_as_done() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_flag("m1", true).unwrap();
        let m = mock_graph(vec![("/me/messages/R1", 404, err_body("ErrorItemNotFound", "gone"))]).await;
        replay_at(&m.base).await.unwrap();
        assert_eq!(pending_count().unwrap(), 0);
        assert!(failed_actions().unwrap().is_empty());
    }

    #[tokio::test]
    async fn permanent_rejection_is_kept_reported_reverted_and_not_retried() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        assert_eq!(sync_status("m1"), "DIRTY_LOCAL");
        let m = mock_graph(vec![("/me/messages/R1", 400, err_body("ErrorInvalidRequest", "That is not allowed"))]).await;

        assert_eq!(replay_at(&m.base).await.unwrap(), 0);
        // No longer "waiting", but not lost: listed with the reason.
        assert_eq!(pending_count().unwrap(), 0);
        let failed = failed_actions().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].operation, "SET_READ");
        assert_eq!(failed[0].error, "400 ErrorInvalidRequest: That is not allowed");
        // Local optimistic state is released so the server's version wins on the next pull.
        assert_eq!(sync_status("m1"), "SYNCED");
        assert_eq!(delta("f"), None);
        assert_eq!(delta("f2"), None);
        // Never retried.
        assert_eq!(replay_at(&m.base).await.unwrap(), 0);
        assert_eq!(m.hits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn one_rejected_item_does_not_block_the_rest() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        set_read("m2", true).unwrap();
        let m = mock_graph(vec![
            ("/me/messages/R1", 403, err_body("ErrorAccessDenied", "no")),
            ("/me/messages/R2", 200, b"{}".to_vec()),
        ])
        .await;
        assert_eq!(replay_at(&m.base).await.unwrap(), 1);
        assert_eq!(failed_actions().unwrap().len(), 1);
        assert_eq!(pending_count().unwrap(), 0);
        assert_eq!(m.hits.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn transient_errors_back_off_and_stay_pending() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        let m = mock_graph(vec![("/me/messages/R1", 503, err_body("ServiceNotAvailable", "try later"))]).await;
        replay_at(&m.base).await.unwrap();
        let (tries, next, err) = row("m1");
        assert_eq!(tries, 1);
        assert!(next > now() + 30, "backed off");
        assert_eq!(err, "503 ServiceNotAvailable: try later");
        assert_eq!(pending_count().unwrap(), 1);
        assert!(failed_actions().unwrap().is_empty());
        assert_eq!(sync_status("m1"), "DIRTY_LOCAL");
        replay_at(&m.base).await.unwrap(); // still backing off: no new request
        assert_eq!(m.hits.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn gives_up_visibly_after_too_many_transient_failures() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        storage::with_db(|db| Ok(db.execute("UPDATE offline_mutation_queue SET retry_count=?1", [MAX_TRIES - 1])?)).unwrap();
        let m = mock_graph(vec![("/me/messages/R1", 500, err_body("InternalServerError", "boom"))]).await;
        replay_at(&m.base).await.unwrap();
        assert_eq!(pending_count().unwrap(), 0);
        assert_eq!(failed_actions().unwrap().len(), 1, "kept, not silently deleted");
    }

    #[tokio::test]
    async fn offline_keeps_the_item_and_records_why() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        assert!(replay_at("http://127.0.0.1:1").await.is_err()); // nothing listens on port 1
        let (tries, next, err) = row("m1");
        assert_eq!((tries, next), (0, 0), "being offline must not use up retries");
        assert!(!err.is_empty());
        assert_eq!(pending_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn move_keeps_the_local_id_and_adopts_graphs_new_remote_id() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        move_to("m1", "f2").unwrap();
        let m = mock_graph(vec![("/me/messages/R1/move", 201, json_body(json!({ "id": "R1-NEW" })))]).await;
        replay_at(&m.base).await.unwrap();
        let rid: String = storage::with_db(|db| Ok(db.query_row("SELECT remote_id FROM messages WHERE id='m1'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(rid, "R1-NEW");
        assert_eq!(pending_count().unwrap(), 0);
    }

    #[test]
    fn discarding_removes_only_failed_items() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault();
        set_read("m1", true).unwrap();
        set_flag("m2", true).unwrap();
        storage::with_db(|db| Ok(db.execute("UPDATE offline_mutation_queue SET retry_count=?1 WHERE local_entity_id='m1'", [FAILED])?)).unwrap();
        assert_eq!(failed_actions().unwrap().len(), 1);
        assert_eq!(discard_failed().unwrap(), 1);
        assert!(failed_actions().unwrap().is_empty());
        assert_eq!(pending_count().unwrap(), 1);
    }
}
