//! Microsoft Graph mail sync: folder hierarchy, per-folder message delta, and offline body hydration.

use crate::{auth, storage};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const MSG_FIELDS: &str = "id,conversationId,internetMessageId,subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,flag,isDraft,hasAttachments,bodyPreview,changeKey";

async fn get(url: &str) -> Result<Value> {
    let token = auth::access_token().await?;
    let resp = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .header("Prefer", "odata.maxpagesize=100, outlook.body-content-type=\"html\"")
        .send()
        .await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(anyhow!("Graph {status}: {}", body["error"]["message"].as_str().unwrap_or("")));
    }
    Ok(body)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn ensure_account(acc: &auth::Account) -> Result<String> {
    storage::with_db(|db| {
        if let Some(id) = db
            .query_row("SELECT id FROM accounts WHERE email=?1", [&acc.email], |r| r.get(0))
            .optional()?
        {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO accounts(id,email,display_name,created_at) VALUES(?1,?2,?3,?4)",
            params![id, acc.email, acc.display_name, now()],
        )?;
        Ok(id)
    })
}

#[derive(Serialize, Default)]
pub struct SyncReport {
    pub folders: usize,
    pub messages_changed: usize,
    pub bodies_downloaded: usize,
    pub inline_images: usize,
    pub replayed: usize,
    pub events: usize,
}

/// Full sync pass: folders → message deltas → bodies.
pub async fn sync_all() -> Result<SyncReport> {
    let acc = auth::current_account().ok_or(anyhow!("Not signed in"))?;
    let account_id = ensure_account(&acc)?;
    let mut rep = SyncReport::default();
    // Replay offline actions before pulling, so server state never overwrites local changes.
    rep.replayed = crate::outbox::replay().await.unwrap_or(0);
    rep.folders = sync_folders(&account_id).await?;

    let folders: Vec<(String, String, Option<String>)> = storage::with_db(|db| {
        let mut st = db.prepare("SELECT id, remote_id, delta_link FROM folders WHERE account_id=?1")?;
        let rows = st.query_map([&account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;
    for (fid, remote, delta) in folders {
        rep.messages_changed += sync_folder_messages(&account_id, &fid, &remote, delta).await?;
    }
    rep.bodies_downloaded = hydrate_bodies(200).await?;
    rep.inline_images = hydrate_inline(200).await.unwrap_or_else(|e| {
        eprintln!("[graphite] inline image download failed: {e}");
        0
    });
    rep.events = crate::calendar::sync().await.unwrap_or(0);
    Ok(rep)
}

async fn sync_folders(account_id: &str) -> Result<usize> {
    let mut stack = vec![format!("{GRAPH}/me/mailFolders?$top=100&includeHiddenFolders=false")];
    let well_known = ["inbox", "drafts", "sentitems", "deleteditems", "junkemail", "archive", "outbox"];
    let mut wk_ids = std::collections::HashMap::new();
    for w in well_known {
        if let Ok(v) = get(&format!("{GRAPH}/me/mailFolders/{w}?$select=id")).await {
            if let Some(id) = v["id"].as_str() {
                wk_ids.insert(id.to_string(), w);
            }
        }
    }
    let mut count = 0;
    while let Some(url) = stack.pop() {
        let page = get(&url).await?;
        for f in page["value"].as_array().cloned().unwrap_or_default() {
            let rid = f["id"].as_str().unwrap_or_default().to_string();
            storage::with_db(|db| {
                db.execute(
                    "INSERT INTO folders(id,account_id,remote_id,parent_remote_id,display_name,well_known_name,unread_count,total_count)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                     ON CONFLICT(account_id,remote_id) DO UPDATE SET display_name=excluded.display_name,
                       parent_remote_id=excluded.parent_remote_id, unread_count=excluded.unread_count, total_count=excluded.total_count",
                    params![
                        uuid::Uuid::new_v4().to_string(), account_id, rid,
                        f["parentFolderId"].as_str(), f["displayName"].as_str().unwrap_or("(folder)"),
                        wk_ids.get(&rid).copied(), f["unreadItemCount"].as_i64().unwrap_or(0), f["totalItemCount"].as_i64().unwrap_or(0)
                    ],
                )?;
                Ok(())
            })?;
            count += 1;
            if f["childFolderCount"].as_i64().unwrap_or(0) > 0 {
                stack.push(format!("{GRAPH}/me/mailFolders/{rid}/childFolders?$top=100"));
            }
        }
        if let Some(next) = page["@odata.nextLink"].as_str() {
            stack.push(next.to_string());
        }
    }
    Ok(count)
}

fn recipients_json(v: &Value) -> String {
    let list: Vec<Value> = v
        .as_array()
        .map(|a| a.iter().map(|r| r["emailAddress"].clone()).collect())
        .unwrap_or_default();
    Value::Array(list).to_string()
}

async fn sync_folder_messages(account_id: &str, folder_id: &str, remote: &str, delta: Option<String>) -> Result<usize> {
    let mut url = delta.unwrap_or_else(|| format!("{GRAPH}/me/mailFolders/{remote}/messages/delta?$select={MSG_FIELDS}"));
    let mut changed = 0;
    loop {
        let page = match get(&url).await {
            Ok(p) => p,
            Err(e) if e.to_string().contains("410") => {
                // Delta token expired: clear it so the next pass does a fresh full sync.
                storage::with_db(|db| Ok(db.execute("UPDATE folders SET delta_link=NULL WHERE id=?1", [folder_id])?))?;
                return Err(e);
            }
            Err(e) => return Err(e),
        };
        let items = page["value"].as_array().cloned().unwrap_or_default();
        storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            for m in &items {
                let rid = m["id"].as_str().unwrap_or_default();
                if m.get("@removed").is_some() {
                    tx.execute("DELETE FROM messages_fts WHERE message_id IN (SELECT id FROM messages WHERE remote_id=?1)", [rid])?;
                    tx.execute("DELETE FROM messages WHERE remote_id=?1", [rid])?;
                    continue;
                }
                let received = m["receivedDateTime"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|d| d.timestamp())
                    .unwrap_or(0);
                tx.execute(
                    "INSERT INTO messages(id,account_id,folder_id,remote_id,internet_message_id,conversation_id,subject,sender_name,sender_address,
                        to_recipients_json,cc_recipients_json,snippet,received_at,is_read,is_flagged,is_draft,has_attachments,change_key)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
                     ON CONFLICT(remote_id) DO UPDATE SET folder_id=excluded.folder_id, subject=excluded.subject,
                        is_read=excluded.is_read, is_flagged=excluded.is_flagged, snippet=excluded.snippet,
                        change_key=excluded.change_key,
                        body_cached=CASE WHEN messages.change_key=excluded.change_key THEN messages.body_cached ELSE 0 END
                     WHERE messages.sync_status='SYNCED'",
                    params![
                        uuid::Uuid::new_v4().to_string(), account_id, folder_id, rid,
                        m["internetMessageId"].as_str(), m["conversationId"].as_str(), m["subject"].as_str(),
                        m["from"]["emailAddress"]["name"].as_str(), m["from"]["emailAddress"]["address"].as_str(),
                        recipients_json(&m["toRecipients"]), recipients_json(&m["ccRecipients"]),
                        m["bodyPreview"].as_str(), received,
                        m["isRead"].as_bool().unwrap_or(false), m["flag"]["flagStatus"].as_str() == Some("flagged"),
                        m["isDraft"].as_bool().unwrap_or(false), m["hasAttachments"].as_bool().unwrap_or(false),
                        m["changeKey"].as_str()
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })?;
        changed += items.len();
        if let Some(next) = page["@odata.nextLink"].as_str() {
            url = next.to_string();
        } else {
            if let Some(dl) = page["@odata.deltaLink"].as_str() {
                storage::with_db(|db| Ok(db.execute("UPDATE folders SET delta_link=?1 WHERE id=?2", params![dl, folder_id])?))?;
            }
            break;
        }
    }
    Ok(changed)
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Content-addressable store: blobs/ab/abcdef….html
pub fn write_blob(bytes: &[u8], ext: &str) -> Result<String> {
    let hash: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    let dir = storage::blob_dir()?.join(&hash[..2]);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{hash}.{ext}"));
    if !path.exists() {
        let tmp = dir.join(format!("{hash}.tmp"));
        fs::write(&tmp, bytes)?;
        fs::rename(tmp, &path)?;
    }
    Ok(hash)
}

pub fn read_blob(hash: &str, ext: &str) -> Result<Vec<u8>> {
    Ok(fs::read(storage::blob_dir()?.join(&hash[..2]).join(format!("{hash}.{ext}")))?)
}

/// Downloads full HTML bodies for newest messages first so mail is readable offline.
pub async fn hydrate_bodies(limit: usize) -> Result<usize> {
    let pending: Vec<(String, String, String, String, String)> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT id, remote_id, COALESCE(subject,''), COALESCE(sender_name,''), COALESCE(sender_address,'')
             FROM messages WHERE body_cached=0 AND remote_id IS NOT NULL ORDER BY received_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;
    let mut n = 0;
    for (id, rid, subject, sname, saddr) in pending {
        let v = match get(&format!("{GRAPH}/me/messages/{rid}?$select=body")).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let html = v["body"]["content"].as_str().unwrap_or_default();
        let hash = write_blob(html.as_bytes(), "html")?;
        let text = strip_html(html);
        storage::with_db(|db| {
            db.execute("UPDATE messages SET body_blob_hash=?1, body_cached=1 WHERE id=?2", params![hash, id])?;
            // A new body may reference different images: have hydrate_inline look at it again.
            db.execute("DELETE FROM inline_scan WHERE message_id=?1", [&id])?;
            db.execute("DELETE FROM messages_fts WHERE message_id=?1", [&id])?;
            db.execute(
                "INSERT INTO messages_fts(message_id,subject,sender_name,sender_address,body_plaintext) VALUES(?1,?2,?3,?4,?5)",
                params![id, subject, sname, saddr, text],
            )?;
            Ok(())
        })?;
        n += 1;
    }
    Ok(n)
}

const MAX_INLINE_BYTES: i64 = 25 * 1024 * 1024;

async fn get_raw(url: &str) -> Result<Vec<u8>> {
    let resp = reqwest::Client::new().get(url).bearer_auth(auth::access_token().await?).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Graph {}", resp.status()));
    }
    Ok(resp.bytes().await?.to_vec())
}

fn mark_inline_scanned(message_id: &str) -> Result<()> {
    storage::with_db(|db| Ok(db.execute("INSERT OR IGNORE INTO inline_scan(message_id) VALUES(?1)", [message_id]).map(|_| ())?))
}

/// Stores downloaded inline images in the blob store and records them in `attachments`.
fn save_inline(message_id: &str, atts: &[crate::inline::Att]) -> Result<usize> {
    let mut stored = Vec::new();
    for a in atts {
        if let Some(bytes) = &a.bytes {
            stored.push((a, write_blob(bytes, "bin")?));
        }
    }
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        tx.execute("DELETE FROM attachments WHERE message_id=?1 AND is_inline=1", [message_id])?;
        for (a, hash) in &stored {
            tx.execute(
                "INSERT INTO attachments(id,message_id,remote_attachment_id,filename,mime_type,size_bytes,content_id,is_inline,blob_hash,is_downloaded)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,1,?8,1)",
                params![uuid::Uuid::new_v4().to_string(), message_id, a.remote_id, a.name, a.mime, a.size, a.content_id, hash],
            )?;
        }
        tx.commit()?;
        Ok(())
    })?;
    Ok(stored.len())
}

/// Downloads the inline images of one message. Returns true when every embeddable attachment was stored.
async fn fetch_inline(base: &str, message_id: &str, remote_id: &str) -> Result<bool> {
    let list = get(&format!("{base}/me/messages/{remote_id}/attachments?$top=100")).await?;
    let mut atts: Vec<_> = crate::inline::parse_attachments(&list).into_iter().filter(|a| a.is_embeddable()).collect();
    for a in atts.iter_mut().filter(|a| a.bytes.is_none() && a.size <= MAX_INLINE_BYTES) {
        // Large attachments are left out of the list response; fetch them individually.
        let one = format!("{base}/me/messages/{remote_id}/attachments/{}", a.remote_id);
        a.bytes = match get(&one).await {
            Ok(v) => crate::inline::parse_attachments(&serde_json::json!({ "value": [v] })).into_iter().next().and_then(|x| x.bytes),
            Err(_) => None,
        };
        if a.bytes.is_none() {
            a.bytes = get_raw(&format!("{one}/$value")).await.ok();
        }
    }
    atts.retain(|a| a.size <= MAX_INLINE_BYTES);
    let complete = atts.iter().all(|a| a.bytes.is_some());
    save_inline(message_id, &atts)?;
    Ok(complete)
}

/// Finds downloaded bodies that haven't been checked for cid: images yet and downloads those images.
/// Covers new messages and everything cached before inline support existed.
pub async fn hydrate_inline(limit: usize) -> Result<usize> {
    hydrate_inline_from(GRAPH, limit).await
}

async fn hydrate_inline_from(base: &str, limit: usize) -> Result<usize> {
    let pending: Vec<(String, String, String)> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT id, remote_id, body_blob_hash FROM messages
             WHERE body_cached=1 AND body_blob_hash IS NOT NULL AND remote_id IS NOT NULL
               AND id NOT IN (SELECT message_id FROM inline_scan)
             ORDER BY received_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;
    let mut n = 0;
    for (id, rid, hash) in pending {
        let Ok(bytes) = read_blob(&hash, "html") else { continue };
        if crate::inline::cid_refs(&String::from_utf8_lossy(&bytes)).is_empty() {
            mark_inline_scanned(&id)?;
            continue;
        }
        match fetch_inline(base, &id, &rid).await {
            Ok(complete) => {
                n += 1;
                if complete {
                    mark_inline_scanned(&id)?;
                }
            }
            Err(e) => {
                eprintln!("[graphite] inline images for message {id} failed: {e}");
                if e.to_string().contains("404") {
                    mark_inline_scanned(&id)?; // message is gone remotely; don't retry forever
                }
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::testutil::{json_body, mock_graph};

    #[test]
    fn strips_html() {
        assert_eq!(super::strip_html("<p>Hello <b>there</b></p>\n<br>world"), "Hello there world");
    }

    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    fn file_att(id: &str, cid: Option<&str>, inline: bool, mime: &str, with_bytes: bool) -> serde_json::Value {
        let mut v = json!({ "@odata.type": "#microsoft.graph.fileAttachment", "id": id, "name": format!("{id}.png"),
            "contentType": mime, "size": 68, "isInline": inline, "contentId": cid });
        if with_bytes {
            v["contentBytes"] = json!(PNG_B64);
        }
        v
    }

    /// Fresh vault with one cached message whose body is `html`.
    fn vault_with_message(html: &str) -> std::path::PathBuf {
        let tmp = storage::init_test_vault();
        let hash = write_blob(html.as_bytes(), "html").unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts VALUES('a','me@x.com','Me',0);
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('f','a','rf','Inbox','inbox');",
            )?;
            db.execute(
                "INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at,body_blob_hash,body_cached)
                 VALUES('m1','a','f','R1','Hi',100,?1,1)",
                [hash],
            )?;
            Ok(())
        })
        .unwrap();
        *auth::TEST_TOKEN.lock().unwrap() = Some("test-token".into());
        tmp
    }

    fn scanned(id: &str) -> bool {
        storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM inline_scan WHERE message_id=?1", [id], |r| r.get::<_, i64>(0))? > 0)).unwrap()
    }

    fn att_rows() -> Vec<(String, String, bool)> {
        storage::with_db(|db| {
            let mut st = db.prepare("SELECT content_id, mime_type, is_downloaded FROM attachments ORDER BY content_id")?;
            Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn downloads_inline_images_and_renders_them_offline() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<p>Hi</p><img src="cid:logo123@mail"><img src='cid:Other'>"#);
        let m = mock_graph(vec![(
            "/me/messages/R1/attachments",
            200,
            json_body(json!({ "value": [
                file_att("A1", Some("logo123@mail"), true, "image/png", true),
                file_att("A2", Some("<other>"), true, "image/png", true),
                file_att("A3", None, false, "application/pdf", true), // ordinary attachment: ignored
            ]})),
        )])
        .await;

        assert_eq!(hydrate_inline_from(&m.base, 50).await.unwrap(), 1);
        assert_eq!(att_rows(), vec![("logo123@mail".into(), "image/png".into(), true), ("other".into(), "image/png".into(), true)]);
        assert!(scanned("m1"));

        // Rendering: both cid: references became data: URIs of the real image bytes.
        let html = crate::mail::message("m1").unwrap().body_html.unwrap();
        assert!(!html.contains("cid:"), "{html}");
        assert_eq!(html.matches(&format!("data:image/png;base64,{PNG_B64}")).count(), 2);

        // Already scanned: a second pass makes no requests at all.
        let before = m.hits.lock().unwrap().len();
        assert_eq!(hydrate_inline_from(&m.base, 50).await.unwrap(), 0);
        assert_eq!(m.hits.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn bodies_without_images_cost_no_requests() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message("<p>No images, just mailto:cid@example.com</p>");
        let m = mock_graph(vec![]).await;
        assert_eq!(hydrate_inline_from(&m.base, 50).await.unwrap(), 0);
        assert!(m.hits.lock().unwrap().is_empty());
        assert!(scanned("m1"));
    }

    #[tokio::test]
    async fn large_attachments_are_fetched_individually() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:big">"#);
        let m = mock_graph(vec![
            ("/me/messages/R1/attachments", 200, json_body(json!({ "value": [file_att("BIG", Some("big"), true, "image/png", false)] }))),
            ("/me/messages/R1/attachments/BIG", 200, json_body(file_att("BIG", Some("big"), true, "image/png", true))),
        ])
        .await;
        hydrate_inline_from(&m.base, 50).await.unwrap();
        assert_eq!(att_rows().len(), 1);
        assert!(crate::mail::message("m1").unwrap().body_html.unwrap().contains("data:image/png;base64,"));
        assert!(scanned("m1"));
    }

    #[tokio::test]
    async fn falls_back_to_raw_value_download() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:raw">"#);
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, PNG_B64).unwrap();
        let m = mock_graph(vec![
            ("/me/messages/R1/attachments", 200, json_body(json!({ "value": [file_att("RAW", Some("raw"), true, "image/png", false)] }))),
            ("/me/messages/R1/attachments/RAW", 500, b"{}".to_vec()),
            ("/me/messages/R1/attachments/RAW/$value", 200, png),
        ])
        .await;
        hydrate_inline_from(&m.base, 50).await.unwrap();
        assert!(crate::mail::message("m1").unwrap().body_html.unwrap().contains(PNG_B64));
        assert!(scanned("m1"));
    }

    #[tokio::test]
    async fn partial_failure_keeps_what_worked_and_retries_later() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:ok"><img src="cid:bad">"#);
        let m = mock_graph(vec![
            ("/me/messages/R1/attachments", 200, json_body(json!({ "value": [
                file_att("OK", Some("ok"), true, "image/png", true),
                file_att("BAD", Some("bad"), true, "image/png", false),
            ]}))),
            ("/me/messages/R1/attachments/BAD", 500, b"{}".to_vec()),
        ])
        .await;
        hydrate_inline_from(&m.base, 50).await.unwrap();
        assert_eq!(att_rows(), vec![("ok".into(), "image/png".into(), true)]);
        assert!(!scanned("m1"), "incomplete message must be retried on the next sync");
        let html = crate::mail::message("m1").unwrap().body_html.unwrap();
        assert!(html.contains(PNG_B64) && html.contains("cid:bad"));
    }

    #[tokio::test]
    async fn vanished_message_is_not_retried_forever_but_other_errors_are() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:x">"#);
        let gone = mock_graph(vec![("/me/messages/R1/attachments", 404, b"{}".to_vec())]).await;
        hydrate_inline_from(&gone.base, 50).await.unwrap();
        assert!(scanned("m1"));

        storage::with_db(|db| Ok(db.execute("DELETE FROM inline_scan", [])?)).unwrap();
        let broken = mock_graph(vec![("/me/messages/R1/attachments", 503, b"{}".to_vec())]).await;
        hydrate_inline_from(&broken.base, 50).await.unwrap();
        assert!(!scanned("m1"));
    }

    #[tokio::test]
    async fn rescanning_replaces_rows_instead_of_duplicating() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:a">"#);
        let m = mock_graph(vec![("/me/messages/R1/attachments", 200, json_body(json!({ "value": [file_att("A", Some("a"), true, "image/png", true)] })))]).await;
        hydrate_inline_from(&m.base, 50).await.unwrap();
        storage::with_db(|db| Ok(db.execute("DELETE FROM inline_scan", [])?)).unwrap(); // what hydrate_bodies does on a changed body
        hydrate_inline_from(&m.base, 50).await.unwrap();
        assert_eq!(att_rows().len(), 1);
    }

    #[tokio::test]
    async fn respects_the_per_pass_limit_newest_first() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        vault_with_message(r#"<img src="cid:a">"#);
        let hash = write_blob(br#"<img src="cid:a">"#, "html").unwrap();
        storage::with_db(|db| {
            db.execute(
                "INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at,body_blob_hash,body_cached)
                 VALUES('m2','a','f','R2','Newer',200,?1,1)",
                [hash],
            )?;
            Ok(())
        })
        .unwrap();
        let m = mock_graph(vec![
            ("/me/messages/R2/attachments", 200, json_body(json!({ "value": [file_att("A", Some("a"), true, "image/png", true)] }))),
            ("/me/messages/R1/attachments", 200, json_body(json!({ "value": [file_att("A", Some("a"), true, "image/png", true)] }))),
        ])
        .await;
        hydrate_inline_from(&m.base, 1).await.unwrap();
        assert!(scanned("m2") && !scanned("m1"));
    }

    #[test]
    fn blob_store_roundtrip_is_content_addressed() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        storage::init_test_vault();
        let a = write_blob(b"same", "bin").unwrap();
        assert_eq!(a, write_blob(b"same", "bin").unwrap());
        assert_eq!(read_blob(&a, "bin").unwrap(), b"same");
    }
}
