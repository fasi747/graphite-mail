//! Attachments: list a message's files, fetch them when asked (and keep them for offline), open or save them,
//! and prepare files chosen in the editor for sending.

use crate::{auth, google, storage, sync};
use anyhow::{Result, anyhow, bail};
use base64::Engine;
use reqwest::Method;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// All files of one outgoing message together (Gmail's limit is 25 MB including encoding overhead).
pub const MAX_OUTGOING: u64 = 15 * 1024 * 1024;

fn graph() -> String {
    #[cfg(test)]
    if let Ok(u) = std::env::var("GM_TEST_GRAPH") {
        return u;
    }
    GRAPH.to_string()
}

#[derive(Serialize)]
pub struct AttachmentRow {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
    pub downloaded: bool,
}

struct Owner {
    email: String,
    provider: String,
    remote_id: Option<String>,
    has_attachments: bool,
}

fn owner_of_message(message_id: &str) -> Result<Owner> {
    storage::with_db(|db| {
        db.query_row(
            "SELECT a.email, COALESCE(a.provider,'microsoft'), m.remote_id, m.has_attachments FROM messages m JOIN accounts a ON a.id=m.account_id WHERE m.id=?1",
            [message_id],
            |r| Ok(Owner { email: r.get(0)?, provider: r.get(1)?, remote_id: r.get(2)?, has_attachments: r.get(3)? }),
        )
        .optional()?
        .ok_or(anyhow!("That message no longer exists."))
    })
}

fn rows(message_id: &str) -> Result<Vec<AttachmentRow>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT id, filename, mime_type, size_bytes, is_downloaded FROM attachments WHERE message_id=?1 AND is_inline=0 ORDER BY rowid")?;
        let r = st.query_map([message_id], |r| Ok(AttachmentRow { id: r.get(0)?, name: r.get(1)?, mime: r.get(2)?, size: r.get(3)?, downloaded: r.get(4)? }))?;
        Ok(r.filter_map(|x| x.ok()).collect())
    })
}

/// Outlook: records the names and sizes of a message's real attachments (the bytes come when opened).
pub async fn store_file_attachments(message_id: &str, remote_id: &str) -> Result<usize> {
    let (s, v) = google::call(Method::GET, &format!("{}/me/messages/{remote_id}/attachments?$top=50&$select=id,name,contentType,size,isInline", graph()), None).await?;
    if !(200..300).contains(&s) {
        bail!("Couldn't read the attachment list ({s}).");
    }
    let list = v["value"].as_array().cloned().unwrap_or_default();
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        tx.execute("DELETE FROM attachments WHERE message_id=?1 AND is_inline=0", [message_id])?;
        let mut n = 0;
        for a in &list {
            if a["isInline"].as_bool() == Some(true) || a["@odata.type"].as_str().is_some_and(|t| t != "#microsoft.graph.fileAttachment") {
                continue;
            }
            tx.execute(
                "INSERT INTO attachments(id,message_id,remote_attachment_id,filename,mime_type,size_bytes,is_inline,is_downloaded) VALUES(?1,?2,?3,?4,?5,?6,0,0)",
                params![uuid::Uuid::new_v4().to_string(), message_id, a["id"].as_str(), a["name"].as_str().unwrap_or("attachment"), a["contentType"].as_str().unwrap_or("application/octet-stream"), a["size"].as_i64().unwrap_or(0)],
            )?;
            n += 1;
        }
        tx.commit()?;
        Ok(n)
    })
}

/// The attachments of a message. Older Outlook messages get their list from the server the first time.
pub async fn list(message_id: &str) -> Result<Vec<AttachmentRow>> {
    let found = rows(message_id)?;
    if !found.is_empty() {
        return Ok(found);
    }
    let o = owner_of_message(message_id)?;
    if o.has_attachments && o.provider == "microsoft" {
        if let Some(rid) = &o.remote_id {
            let _ = auth::in_account(Some(o.email.clone()), store_file_attachments(message_id, rid)).await; // offline: just no list yet
            return rows(message_id);
        }
    }
    Ok(found)
}

struct Att {
    name: String,
    size: i64,
    remote: Option<String>,
    blob: Option<String>,
    message_id: String,
}

fn load(att_id: &str) -> Result<Att> {
    storage::with_db(|db| {
        db.query_row("SELECT filename, size_bytes, remote_attachment_id, blob_hash, message_id FROM attachments WHERE id=?1", [att_id], |r| {
            Ok(Att { name: r.get(0)?, size: r.get(1)?, remote: r.get(2)?, blob: r.get(3)?, message_id: r.get(4)? })
        })
        .optional()?
        .ok_or(anyhow!("That attachment is no longer available."))
    })
}

/// The file's bytes: from the local copy, else downloaded now (and kept for offline).
pub async fn bytes(att_id: &str) -> Result<(String, Vec<u8>)> {
    let a = load(att_id)?;
    if let Some(h) = &a.blob {
        if let Ok(b) = sync::read_blob(h, "bin") {
            return Ok((a.name, b));
        }
    }
    let o = owner_of_message(&a.message_id)?;
    let rid = o.remote_id.clone().ok_or(anyhow!("This message hasn't been uploaded yet."))?;
    let data = auth::in_account(Some(o.email.clone()), async {
        if o.provider == "google" {
            google::download_attachment(&rid, &a.name, a.size).await
        } else if o.provider == crate::imap::PROVIDER {
            crate::imap::download_attachment(&a.message_id, &a.name, a.size).await
        } else {
            let aid = a.remote.clone().ok_or(anyhow!("Attachment id missing"))?;
            let (s, v) = google::call(Method::GET, &format!("{}/me/messages/{rid}/attachments/{aid}", graph()), None).await?;
            if !(200..300).contains(&s) {
                bail!("The server couldn't give us this file ({s}).");
            }
            Ok(base64::engine::general_purpose::STANDARD.decode(v["contentBytes"].as_str().unwrap_or_default())?)
        }
    })
    .await
    .map_err(|e| if e.to_string().contains("error sending request") { anyhow!("You're offline, and this file hasn't been downloaded yet.") } else { e })?;
    let hash = sync::write_blob(&data, "bin")?;
    storage::with_db(|db| Ok(db.execute("UPDATE attachments SET blob_hash=?1, is_downloaded=1 WHERE id=?2", params![hash, att_id]).map(|_| ())?))?;
    Ok((a.name, data))
}

/// File names that could run code when opened; these are saved, never opened straight away.
pub fn risky(name: &str) -> bool {
    let n = name.to_lowercase();
    [".exe", ".sh", ".bat", ".cmd", ".com", ".scr", ".msi", ".appimage", ".desktop", ".jar", ".js", ".vbs", ".ps1", ".run", ".bin", ".pif", ".lnk", ".py", ".pl"]
        .iter()
        .any(|e| n.ends_with(e))
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c == '\0' || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "attachment".into() } else { cleaned }
}

/// Where opened attachments are put; emptied at every start.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("graphite-mail-attachments")
}

pub fn clear_temp() {
    let _ = std::fs::remove_dir_all(temp_root());
}

/// Writes the file to a private temp folder and returns its path (the caller opens it).
pub async fn prepare_open(att_id: &str) -> Result<PathBuf> {
    let a = load(att_id)?;
    if risky(&a.name) {
        bail!("For your safety “{}” can't be opened straight from an email. Use Save, and open it yourself if you trust it.", a.name);
    }
    let (name, data) = bytes(att_id).await?;
    let dir = temp_root().join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(safe_name(&name));
    std::fs::write(&path, data)?;
    Ok(path)
}

pub async fn save(att_id: &str, dest: &str) -> Result<()> {
    let (_, data) = bytes(att_id).await?;
    std::fs::write(Path::new(dest), data).map_err(|e| anyhow!("Couldn't save the file: {e}"))
}

/// Files chosen in the editor → blobs + the list stored in the queued message. Enforces the size limit.
pub fn prepare_outgoing(paths: &[String]) -> Result<Vec<Value>> {
    let mut total = 0u64;
    let mut out = vec![];
    for p in paths {
        let path = Path::new(p);
        let meta = std::fs::metadata(path).map_err(|_| anyhow!("Couldn't read {p}."))?;
        if !meta.is_file() {
            bail!("{p} is not a file.");
        }
        total += meta.len();
        if total > MAX_OUTGOING {
            bail!("Attachments are limited to {} MB in total.", MAX_OUTGOING >> 20);
        }
        let bytes = std::fs::read(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "attachment".into());
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        out.push(json!({ "name": name, "mime": mime, "size": bytes.len(), "blob": sync::write_blob(&bytes, "bin")? }));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::tests::fake_google;

    fn pdf_b64() -> String {
        base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.4 hello")
    }

    fn setup() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!("gm-att-{}", uuid::Uuid::new_v4()));
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg"));
            std::env::set_var("GM_TEST_TOKEN", "t");
        }
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts(id,email,display_name,created_at,provider) VALUES('m','me@x.com','Me',1,'microsoft'),('g','me@gmail.com','Gm',2,'google');
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('mi','m','MI','Inbox','inbox'),('gi','g','INBOX','Inbox','inbox');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,received_at,has_attachments,body_cached) VALUES
                   ('m1','m','mi','r1','With file',1,1,1),('g1','g','gi','gm1','Gmail file',2,1,1);",
            )?;
            Ok(())
        })
        .unwrap();
        tmp
    }

    #[test]
    fn dangerous_files_are_not_opened() {
        for n in ["setup.EXE", "run.sh", "x.AppImage", "evil.desktop", "a.ps1"] {
            assert!(risky(n), "{n}");
        }
        for n in ["report.pdf", "photo.jpg", "notes.txt", "sheet.xlsx"] {
            assert!(!risky(n), "{n}");
        }
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd"); // no folder tricks
        assert_eq!(safe_name("..."), "attachment");
        assert_eq!(safe_name("a/b\\c.txt"), "a_b_c.txt");
    }

    #[tokio::test]
    async fn attachments_list_download_open_and_save() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = setup();
        let mut pdf = tmp.clone();
        pdf.push("unused");
        let gmail_raw = crate::google::tests::raw_mail("Gmail file", "<p>x</p>", None, true);
        let (base, log) = fake_google(move |m, p, _| match (m, p.split('?').next().unwrap()) {
            ("GET", "/me/messages/r1/attachments") => (200, json!({ "value": [
                { "@odata.type": "#microsoft.graph.fileAttachment", "id": "A1", "name": "contract.pdf", "contentType": "application/pdf", "size": 14, "isInline": false },
                { "@odata.type": "#microsoft.graph.fileAttachment", "id": "A2", "name": "logo.png", "contentType": "image/png", "size": 3, "isInline": true },
                { "@odata.type": "#microsoft.graph.itemAttachment", "id": "A3", "name": "forwarded mail", "size": 5 } ] })),
            ("GET", "/me/messages/r1/attachments/A1") => (200, json!({ "contentBytes": pdf_b64() })),
            ("GET", "/gmail/v1/users/me/messages/gm1") => (200, json!({ "id": "gm1", "raw": gmail_raw })),
            _ => (404, json!({ "error": { "message": "no route" } })),
        })
        .await;
        unsafe {
            std::env::set_var("GM_TEST_GRAPH", &base);
            std::env::set_var("GM_TEST_GOOGLE", &base);
        }

        // Outlook: names come from the server once, only real files are listed
        let list = list("m1").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].size, list[0].downloaded), ("contract.pdf", 14, false));
        let calls_before = log.lock().unwrap().len();
        assert_eq!(crate::attachments::list("m1").await.unwrap().len(), 1);
        assert_eq!(log.lock().unwrap().len(), calls_before, "the list is kept locally");

        // downloading keeps a local copy
        let (name, data) = bytes(&list[0].id).await.unwrap();
        assert_eq!((name.as_str(), data.as_slice()), ("contract.pdf", b"%PDF-1.4 hello".as_slice()));
        let n = log.lock().unwrap().len();
        bytes(&list[0].id).await.unwrap();
        assert_eq!(log.lock().unwrap().len(), n, "second time it comes from the local copy");
        assert!(list_downloaded("m1"));

        // opening writes a private temp file; saving copies it
        let path = prepare_open(&list[0].id).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.4 hello");
        assert!(path.starts_with(temp_root()));
        let dest = tmp.join("saved.pdf");
        save(&list[0].id, dest.to_str().unwrap()).await.unwrap();
        assert_eq!(std::fs::read(dest).unwrap(), b"%PDF-1.4 hello");
        clear_temp();
        assert!(!path.exists());

        // risky types refuse to open
        storage::with_db(|db| {
            db.execute("INSERT INTO attachments(id,message_id,remote_attachment_id,filename,mime_type,size_bytes,is_inline,is_downloaded) VALUES('bad','m1','A9','install.sh','text/x-sh',3,0,0)", [])?;
            Ok(())
        })
        .unwrap();
        assert!(prepare_open("bad").await.unwrap_err().to_string().contains("For your safety"));

        // Gmail: attachment metadata was stored at sync; bytes are found again in the full message
        storage::with_db(|db| {
            db.execute("INSERT INTO attachments(id,message_id,filename,mime_type,size_bytes,is_inline,is_downloaded) VALUES('gatt','g1','bill.pdf','application/pdf',13,0,0)", [])?;
            Ok(())
        })
        .unwrap();
        let (_, gdata) = bytes("gatt").await.unwrap();
        assert_eq!(gdata, b"%PDF-1.4 fake");
    }

    fn list_downloaded(message_id: &str) -> bool {
        rows(message_id).unwrap().iter().all(|r| r.downloaded)
    }

    #[test]
    fn outgoing_files_are_checked_and_stored() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = setup();
        let f = tmp.join("report.pdf");
        std::fs::write(&f, b"data").unwrap();
        let out = prepare_outgoing(&[f.to_string_lossy().to_string()]).unwrap();
        assert_eq!((out[0]["name"].as_str(), out[0]["mime"].as_str(), out[0]["size"].as_i64()), (Some("report.pdf"), Some("application/pdf"), Some(4)));
        assert_eq!(sync::read_blob(out[0]["blob"].as_str().unwrap(), "bin").unwrap(), b"data");
        let big = tmp.join("big.bin");
        std::fs::write(&big, vec![0u8; (MAX_OUTGOING + 1) as usize]).unwrap();
        assert!(prepare_outgoing(&[big.to_string_lossy().to_string()]).unwrap_err().to_string().contains("limited to 15 MB"));
        assert!(prepare_outgoing(&["/no/such/file".into()]).is_err());
        assert!(prepare_outgoing(&[tmp.to_string_lossy().to_string()]).unwrap_err().to_string().contains("not a file"));
    }

    #[tokio::test]
    async fn sending_with_attachments_on_both_providers() {
        use crate::outbox::{Draft, pending_count, replay, send};
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = setup();
        let small = tmp.join("small.txt");
        let large = tmp.join("large.bin");
        std::fs::write(&small, b"small file").unwrap();
        std::fs::write(&large, vec![7u8; 3_500_000]).unwrap();
        let (paths, ) = (vec![small.to_string_lossy().to_string(), large.to_string_lossy().to_string()],);
        let gmail_orig = crate::google::tests::raw_mail("Original", "<p>orig</p>", None, true);
        let (base, log) = fake_google(move |m, p, _| match (m, p.split('?').next().unwrap()) {
            ("POST", "/me/messages") => (201, json!({ "id": "D1" })),
            ("POST", "/me/messages/r1/createReply") => (201, json!({ "id": "D2" })),
            ("PATCH", "/me/messages/D2") => (200, json!({})),
            ("POST", x) if x.ends_with("/attachments") => (201, json!({})),
            ("POST", x) if x.ends_with("/createUploadSession") => (201, json!({ "uploadUrl": format!("{}/upload/1", std::env::var("GM_TEST_GRAPH").unwrap()) })),
            ("PUT", "/upload/1") => (200, json!({})),
            ("POST", x) if x.ends_with("/send") => (202, Value::Null),
            ("GET", "/gmail/v1/users/me/messages/gm1") => (200, json!({ "id": "gm1", "raw": gmail_orig })),
            ("POST", "/gmail/v1/users/me/messages/send") => (200, json!({ "id": "s" })),
            _ => (404, json!({ "error": { "message": "no route" } })),
        })
        .await;
        unsafe {
            std::env::set_var("GM_TEST_GRAPH", &base);
            std::env::set_var("GM_TEST_GOOGLE", &base);
        }

        // Outlook, new message: draft -> small file -> big file by upload session in 2 pieces -> send
        send(Draft { mode: "new".into(), reply_to: None, to: vec!["a@b.c".into()], cc: vec![], bcc: vec!["hidden@b.c".into()], subject: "Files".into(), html: "<p>hi</p>".into(), from: Some("me@x.com".into()), attachments: paths.clone() }).unwrap();
        assert_eq!(replay().await.unwrap(), 1);
        let seq: Vec<String> = log.lock().unwrap().iter().map(|(m, p, _)| format!("{m} {}", p.split('?').next().unwrap())).collect();
        assert_eq!(seq, vec!["POST /me/messages", "POST /me/messages/D1/attachments", "POST /me/messages/D1/attachments/createUploadSession", "PUT /upload/1", "PUT /upload/1", "POST /me/messages/D1/send"]);
        let first = log.lock().unwrap()[0].2.clone();
        assert_eq!(first["subject"], "Files");
        // The draft-with-files path keeps blind copies out of To/Cc too.
        assert_eq!(first["bccRecipients"][0]["emailAddress"]["address"], "hidden@b.c");
        assert_eq!((first["toRecipients"].as_array().unwrap().len(), first["ccRecipients"].as_array().unwrap().len()), (1, 0));
        assert!(!first["toRecipients"].to_string().contains("hidden"));
        assert_eq!(log.lock().unwrap()[1].2["name"], "small.txt");
        assert_eq!(pending_count().unwrap(), 0);

        // Outlook reply: a reply draft is made, filled in, then sent with the file
        log.lock().unwrap().clear();
        send(Draft { mode: "reply".into(), reply_to: Some("m1".into()), to: vec!["a@b.c".into()], cc: vec![], bcc: vec![], subject: "Re".into(), html: "<p>r</p>".into(), from: None, attachments: vec![small.to_string_lossy().to_string()] }).unwrap();
        replay().await.unwrap();
        let seq: Vec<String> = log.lock().unwrap().iter().map(|(m, p, _)| format!("{m} {}", p.split('?').next().unwrap())).collect();
        assert_eq!(seq, vec!["POST /me/messages/r1/createReply", "PATCH /me/messages/D2", "POST /me/messages/D2/attachments", "POST /me/messages/D2/send"]);

        // Gmail: the file is inside the message; a forward also carries the original's file
        log.lock().unwrap().clear();
        send(Draft { mode: "forward".into(), reply_to: Some("g1".into()), to: vec!["z@z.com".into()], cc: vec![], bcc: vec![], subject: "Fw: Original".into(), html: "<p>fyi</p>".into(), from: None, attachments: vec![small.to_string_lossy().to_string()] }).unwrap();
        replay().await.unwrap();
        let sent = log.lock().unwrap().iter().find(|(_, p, _)| p.ends_with("/messages/send")).cloned().unwrap();
        let raw = crate::google::tests::unb64_for_test(sent.2["raw"].as_str().unwrap());
        let parsed = crate::google::parse_raw_full(&raw, true).unwrap();
        let names: Vec<&str> = parsed.files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, vec!["small.txt", "bill.pdf"]);
        assert_eq!(parsed.file_bytes[0], b"small file");
        assert_eq!(pending_count().unwrap(), 0);
    }
}
