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

/// Changes still waiting to go to the server (changes the server turned down are counted separately).
pub fn pending_count() -> Result<i64> {
    storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM offline_mutation_queue WHERE failed=0", [], |r| r.get(0))?))
}

#[derive(serde::Serialize)]
pub struct QueueItem {
    pub seq: i64,
    pub what: String,
    pub failed: bool,
    pub tries: i64,
    pub error: Option<String>,
}

fn describe(op: &str) -> &'static str {
    match op {
        "SET_READ" => "Mark as read/unread",
        "SET_FLAG" => "Flag or unflag",
        "MOVE" => "Move to folder",
        "DELETE_HARD" => "Delete permanently",
        "SEND" => "Send message",
        "RSVP" => "Reply to invitation",
        "CREATE_EVENT" => "Create event",
        "DELETE_EVENT" => "Delete event",
        _ => "Change",
    }
}

/// Everything still in the queue, for the "waiting to sync" details.
pub fn overview() -> Result<Vec<QueueItem>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT seq_id, operation, failed, retry_count, last_error FROM offline_mutation_queue ORDER BY seq_id")?;
        let rows = st.query_map([], |r| {
            Ok(QueueItem { seq: r.get(0)?, what: describe(&r.get::<_, String>(1)?).into(), failed: r.get(2)?, tries: r.get(3)?, error: r.get(4)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Puts rejected changes back in line for another attempt.
pub fn retry_failed() -> Result<()> {
    storage::with_db(|db| {
        db.execute("UPDATE offline_mutation_queue SET failed=0, retry_count=0, next_retry_at=0", [])?;
        Ok(())
    })
}

/// Gives up on rejected changes (and lets the server's version win next sync).
pub fn discard_failed() -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        for table in ["messages", "calendar_events"] {
            tx.execute(
                &format!("UPDATE {table} SET sync_status='SYNCED' WHERE id IN (SELECT local_entity_id FROM offline_mutation_queue WHERE failed=1)"),
                [],
            )?;
        }
        tx.execute("DELETE FROM offline_mutation_queue WHERE failed=1", [])?;
        tx.commit()?;
        Ok(())
    })
}

enum Verdict {
    Done,
    /// Temporary trouble (offline server, throttling, expired sign-in): try again later.
    Retry,
    /// The server said no and will keep saying no.
    Rejected,
}

fn verdict(op: &str, status: u16) -> Verdict {
    match status {
        // Not found = already gone, nothing left to do; but never drop an unsent message silently.
        404 if op != "SEND" => Verdict::Done,
        200..=299 => Verdict::Done,
        401 | 408 | 429 | 500..=599 => Verdict::Retry,
        _ => Verdict::Rejected,
    }
}

const MAX_TRIES: i64 = 10;

fn graph_base() -> String {
    #[cfg(test)]
    if let Ok(u) = std::env::var("GM_TEST_GRAPH") {
        return u;
    }
    GRAPH.to_string()
}

/// Only one replay at a time, so the same message can never be sent twice.
static REPLAY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let mut req = crate::http::client().request(method, url).bearer_auth(token);
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
    let _one_at_a_time = REPLAY_LOCK.lock().await;
    let base = graph_base();
    storage::with_db(coalesce)?;
    let items: Vec<Item> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT seq_id, local_entity_id, operation, payload_json FROM offline_mutation_queue
             WHERE failed=0 AND next_retry_at <= ?1 ORDER BY seq_id",
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
                        // A forward must name its recipients at the top level, not only inside the message.
                        let mut body = json!({ "message": msg, "comment": "" });
                        if mode == "forward" {
                            body["toRecipients"] = recipients(&p["to"]);
                        }
                        call(Method::POST, &format!("{base}/me/messages/{r}/{mode}"), Some(body)).await
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
            Ok((s, v)) if matches!(verdict(&it.op, s), Verdict::Done) => {
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
                let msg = format!("{s}: {}", v["error"]["message"].as_str().unwrap_or("The server did not accept this change."));
                eprintln!("[graphite] change {} not accepted: {msg}", it.op);
                let rejected = matches!(verdict(&it.op, s), Verdict::Rejected);
                storage::with_db(|db| {
                    // Rejected for good, or tried too often: keep it visible but stop retrying. Otherwise back off (30s … 16min).
                    db.execute(
                        "UPDATE offline_mutation_queue SET retry_count=retry_count+1, last_error=?1,
                           next_retry_at=?2 + (30 * (1 << MIN(retry_count, 5))),
                           failed=CASE WHEN ?4 OR retry_count+1>=?5 THEN 1 ELSE 0 END WHERE seq_id=?3",
                        params![msg, now(), it.seq, rejected, MAX_TRIES],
                    )?;
                    Ok(())
                })?;
            }
            Err(e) => {
                // Offline or timed out: remember why, try again next pass, and stop here to keep order.
                let msg = e.to_string();
                let _ = storage::with_db(|db| Ok(db.execute("UPDATE offline_mutation_queue SET last_error=?1 WHERE seq_id=?2", params![msg, it.seq])?));
                return if done > 0 { Ok(done) } else { Err(e) };
            }
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

    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A pretend Microsoft Graph: answers with the given status codes in turn, and records every request.
    async fn fake_graph(statuses: Vec<u16>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (seen2, queue) = (seen.clone(), Arc::new(Mutex::new(VecDeque::from(statuses))));
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    data.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some(head_end) = text.find("\r\n\r\n") {
                        let want = text[..head_end].to_ascii_lowercase().split("content-length:").nth(1)
                            .and_then(|r| r.lines().next()?.trim().parse::<usize>().ok()).unwrap_or(0);
                        if data.len() >= head_end + 4 + want { break; }
                    }
                    if n == 0 { break; }
                }
                seen2.lock().unwrap().push(String::from_utf8_lossy(&data).to_string());
                let st = queue.lock().unwrap().pop_front().unwrap_or(200);
                let body = if st >= 400 { r#"{"error":{"message":"boom"}}"# } else { "{}" };
                let resp = format!("HTTP/1.1 {st} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (url, seen)
    }

    fn q(op: &str, entity: &str, payload: Value) {
        storage::with_db(|db| enqueue(db, entity, op, payload)).unwrap();
    }
    fn clear() {
        storage::with_db(|db| Ok(db.execute("DELETE FROM offline_mutation_queue", [])?)).unwrap();
    }

    #[tokio::test]
    async fn replay_copes_with_every_kind_of_answer() {
        let _guard = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("gm-replay-{}", uuid::Uuid::new_v4()));
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg"));
            std::env::set_var("GM_TEST_TOKEN", "t");
        }
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO folders(id,account_id,remote_id,display_name) VALUES('f','a','rf','Inbox');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,received_at) VALUES('m1','a','f','r1',1),('m2','a','f','r2',2);",
            )?;
            Ok(())
        })
        .unwrap();

        // 1. Accepted: leaves the queue.
        let (url, seen) = fake_graph(vec![200]).await;
        unsafe { std::env::set_var("GM_TEST_GRAPH", &url) };
        q("SET_READ", "m1", json!({ "read": true }));
        assert_eq!(replay().await.unwrap(), 1);
        assert_eq!(pending_count().unwrap(), 0);
        assert!(seen.lock().unwrap()[0].starts_with("PATCH /me/messages/r1 "));

        // 2. Refused for good (403): stops being "waiting", stays listed with the reason, is not retried.
        let (url, seen) = fake_graph(vec![403]).await;
        unsafe { std::env::set_var("GM_TEST_GRAPH", &url) };
        q("SET_FLAG", "m2", json!({ "flagged": true }));
        replay().await.unwrap();
        assert_eq!(pending_count().unwrap(), 0);
        let list = overview().unwrap();
        assert!(list[0].failed && list[0].error.as_deref().unwrap().contains("403"));
        replay().await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 1, "a refused change must not be retried");
        discard_failed().unwrap();
        assert!(overview().unwrap().is_empty());

        // 3. Temporary trouble (503): still waiting, backs off.
        let (url, _) = fake_graph(vec![503]).await;
        unsafe { std::env::set_var("GM_TEST_GRAPH", &url) };
        q("SET_READ", "m1", json!({ "read": false }));
        replay().await.unwrap();
        assert_eq!(pending_count().unwrap(), 1);
        let (tries, later): (i64, bool) = storage::with_db(|db| {
            Ok(db.query_row("SELECT retry_count, next_retry_at > ?1 FROM offline_mutation_queue", [now()], |r| Ok((r.get(0)?, r.get(1)?)))?)
        }).unwrap();
        assert_eq!((tries, later), (1, true));
        clear();

        // 4. A forward names its recipients at the top level.
        let (url, seen) = fake_graph(vec![202]).await;
        unsafe { std::env::set_var("GM_TEST_GRAPH", &url) };
        q("SEND", "m1", json!({ "mode": "forward", "to": ["a@x.com"], "cc": [], "subject": "Fw: hi", "html": "<p>hi</p>" }));
        assert_eq!(replay().await.unwrap(), 1);
        let req = seen.lock().unwrap()[0].clone();
        assert!(req.starts_with("POST /me/messages/r1/forward "));
        let body: Value = serde_json::from_str(req.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["toRecipients"][0]["emailAddress"]["address"], "a@x.com");

        // 5. A reply whose original message is gone (404) is never dropped silently.
        let (url, _) = fake_graph(vec![404]).await;
        unsafe { std::env::set_var("GM_TEST_GRAPH", &url) };
        q("SEND", "m1", json!({ "mode": "reply", "to": ["a@x.com"], "cc": [], "subject": "Re: hi", "html": "<p>x</p>" }));
        replay().await.unwrap();
        assert_eq!(pending_count().unwrap(), 0);
        assert!(overview().unwrap()[0].failed);
        retry_failed().unwrap();
        assert_eq!(pending_count().unwrap(), 1); // back in line
        clear();

        // 6. Offline: nothing is lost, the reason is remembered, and the error is reported.
        unsafe { std::env::set_var("GM_TEST_GRAPH", "http://127.0.0.1:1") };
        q("SET_READ", "m1", json!({ "read": true }));
        assert!(replay().await.is_err());
        assert_eq!(pending_count().unwrap(), 1);
        assert!(overview().unwrap()[0].error.is_some());
    }
}
