//! Contacts sync against a pretend Microsoft Graph: paging, throttling, delta, permissions, removal.

use super::*;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// (status, extra headers, JSON body)
type Reply = (u16, Vec<(&'static str, String)>, Value);

#[derive(Default)]
struct Fake {
    /// URL prefix (path + query) → replies in turn; the last one keeps being given.
    routes: Vec<(String, VecDeque<Reply>)>,
    seen: Vec<String>,
}

impl Fake {
    fn on(&mut self, prefix: &str, replies: Vec<Reply>) {
        self.routes.retain(|(p, _)| p != prefix);
        self.routes.push((prefix.to_string(), replies.into()));
    }
    fn answer(&mut self, target: &str) -> Reply {
        self.seen.push(target.to_string());
        let best = self.routes.iter_mut().filter(|(p, _)| target.starts_with(p.as_str())).max_by_key(|(p, _)| p.len());
        match best {
            Some((_, q)) if q.len() > 1 => q.pop_front().unwrap(),
            Some((_, q)) => q.front().cloned().unwrap(),
            None => (404, vec![], json!({ "error": { "code": "NotFound" } })),
        }
    }
    fn count(&self, prefix: &str) -> usize {
        self.seen.iter().filter(|s| s.starts_with(prefix)).count()
    }
}

fn ok(v: Value) -> Reply {
    (200, vec![], v)
}

async fn fake_graph() -> (String, Arc<Mutex<Fake>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fake = Arc::new(Mutex::new(Fake::default()));
    let f = fake.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let f = f.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16384];
                let mut got = 0;
                while !buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut buf[got..]).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => got += n,
                    }
                }
                let head = String::from_utf8_lossy(&buf[..got]).to_string();
                let target = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                assert!(head.to_ascii_lowercase().contains("authorization: bearer t"), "every call carries the token");
                let (status, headers, body) = f.lock().unwrap().answer(&target);
                let body = body.to_string();
                let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
                let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, fake)
}

/// A fresh database with one Microsoft account ("a", me@corp.com) and a Gmail one ("g").
fn setup(base: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("gm-contacts-{}", uuid::Uuid::new_v4()));
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg"));
        std::env::set_var("GM_TEST_TOKEN", "t");
        std::env::set_var("GM_TEST_GRAPH", base);
    }
    storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
    storage::with_db(|db| {
        db.execute_batch(
            "INSERT INTO accounts(id,email,display_name,created_at,provider) VALUES('a','me@corp.com','Me',1,'microsoft'),('g','me@gmail.com','Gm',2,'imap');",
        )?;
        Ok(())
    })
    .unwrap();
    let _ = auth::sign_out();
    for (email, provider) in [("me@corp.com", "microsoft"), ("me@gmail.com", "imap")] {
        auth::remember_account(&auth::Account { email: email.into(), display_name: email.into(), provider: provider.into(), rules: false, contacts: false }).unwrap();
    }
    tmp
}

fn names(source: &str) -> Vec<String> {
    group(source, "", None, 100).unwrap().rows.into_iter().map(|r| r.name).collect()
}

fn user(id: &str, name: &str, mail: Option<&str>) -> Value {
    json!({ "id": id, "displayName": name, "mail": mail, "jobTitle": null, "businessPhones": [], "accountEnabled": true })
}

#[test]
fn retry_after_is_read_and_capped() {
    assert_eq!(retry_after(Some("7"), 0), 7);
    assert_eq!(retry_after(Some(" 3 "), 4), 3);
    assert_eq!(retry_after(Some("3600"), 0), MAX_WAIT_SECS, "never waits more than a minute");
    assert_eq!(retry_after(None, 0), 1);
    assert_eq!(retry_after(None, 3), 8, "doubles when Graph doesn't say");
    assert_eq!(retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT"), 1), 2, "a date falls back to doubling");
}

#[test]
fn graph_records_become_contacts() {
    let u = json!({ "id": "1", "displayName": "Ana Diaz", "mail": "ana@corp.com", "jobTitle": "CFO", "department": "Finance",
        "officeLocation": "HQ 4", "businessPhones": ["+1 555 0100"], "mobilePhone": "+1 555 0199", "accountEnabled": true });
    let e = from_user(&u).unwrap();
    assert_eq!((e.name.as_str(), e.emails[0].as_str(), e.job_title.as_deref(), e.office.as_deref()), ("Ana Diaz", "ana@corp.com", Some("CFO"), Some("HQ 4")));
    assert_eq!(e.phones, vec![("work".to_string(), "+1 555 0100".to_string()), ("mobile".to_string(), "+1 555 0199".to_string())]);
    assert!(from_user(&json!({ "id": "2", "displayName": "Room 4.01", "mail": "room401@corp.com", "accountEnabled": false })).is_none(), "disabled accounts are skipped");
    assert!(from_user(&json!({ "id": "3", "displayName": "Service account", "mail": null })).is_none(), "entries without a mailbox are skipped");
    let c = from_contact(&json!({ "id": "c", "givenName": "Bo", "surname": "Li", "emailAddresses": [{ "address": "bo@x.com" }, { "address": "bo@home.com" }], "homePhones": ["123"] }));
    assert_eq!((c.name.as_str(), c.emails.len(), c.phones[0].0.as_str()), ("Bo Li", 2, "home"));
    assert_eq!(from_contact(&json!({ "id": "d" })).name, "(no name)");
}

#[tokio::test]
async fn directory_is_paged_throttled_swept_and_enriched() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (base, fake) = fake_graph().await;
    setup(&base);
    {
        let mut f = fake.lock().unwrap();
        f.on("/users?$select=id,displayName,mail,jobTitle", vec![ok(json!({
            "value": [
                { "id": "u1", "displayName": "Ana Diaz", "mail": "ana@corp.com", "jobTitle": "CFO", "department": "Finance", "businessPhones": ["+1 555 0100"], "accountEnabled": true },
                { "id": "u2", "displayName": "Ben Ode", "mail": "ben@corp.com", "accountEnabled": true },
                { "id": "u3", "displayName": "Room 4.01", "mail": "room@corp.com", "accountEnabled": false },
                { "id": "u4", "displayName": "Sync service", "mail": null, "accountEnabled": true }
            ],
            "@odata.nextLink": format!("{base}/users?page=2")
        }))]);
        // Page 2 is throttled twice before it comes.
        f.on("/users?page=2", vec![
            (429, vec![("Retry-After", "2".into())], json!({ "error": { "code": "TooManyRequests" } })),
            (429, vec![("Retry-After", "1".into())], json!({ "error": { "code": "TooManyRequests" } })),
            ok(json!({ "value": [user("u5", "Cleo Park", Some("cleo@corp.com"))] })),
        ]);
        f.on("/me/people", vec![ok(json!({ "value": [{ "scoredEmailAddresses": [{ "address": "Ben@Corp.com" }], "jobTitle": "Engineer", "department": "R&D",
            "phones": [{ "type": "business", "number": "+1 555 0123" }] }] }))]);
    }
    assert_eq!(sync_directory("a").await.unwrap(), 3);
    assert_eq!(names(DIRECTORY), vec!["Ana Diaz", "Ben Ode", "Cleo Park"], "disabled and mailbox-less entries skipped");
    assert_eq!(fake.lock().unwrap().count("/users?page=2"), 3, "429 answers were retried after Retry-After");
    let ben = group(DIRECTORY, "ben", None, 10).unwrap().rows.remove(0);
    let d = detail(&ben.id).unwrap();
    assert_eq!((d.job_title.as_deref(), d.department.as_deref(), d.phones[0].number.as_str()), (Some("Engineer"), Some("R&D"), "+1 555 0123"), "People fills the gaps");
    assert_eq!(group(DIRECTORY, "engineer", None, 10).unwrap().total, 1, "the added title is searchable");

    // Someone left: the next full listing removes them.
    fake.lock().unwrap().on("/users?page=2", vec![ok(json!({ "value": [] }))]);
    sync_directory("a").await.unwrap();
    assert_eq!(names(DIRECTORY), vec!["Ana Diaz", "Ben Ode"]);

    // Basic directory access only: the full field list is refused, the basic one works (and is remembered).
    {
        let mut f = fake.lock().unwrap();
        f.on("/users?$select=id,displayName,mail,jobTitle", vec![(403, vec![], json!({ "error": { "code": "Authorization_RequestDenied" } }))]);
        f.on("/users?$select=id,displayName,mail&", vec![ok(json!({ "value": [user("u1", "Ana Diaz", Some("ana@corp.com"))] }))]);
    }
    assert_eq!(sync_directory("a").await.unwrap(), 1);
    assert!(state_get("a", "contacts:dir_basic").is_some());
    let before = fake.lock().unwrap().count("/users?$select=id,displayName,mail,jobTitle");
    sync_directory("a").await.unwrap();
    assert_eq!(fake.lock().unwrap().count("/users?$select=id,displayName,mail,jobTitle"), before, "the refused field list isn't asked for again");
}

#[tokio::test]
async fn throttling_that_never_ends_is_an_error_not_a_hang() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (base, fake) = fake_graph().await;
    setup(&base);
    fake.lock().unwrap().on("/users", vec![(429, vec![("Retry-After", "1".into())], json!({}))]);
    let e = sync_directory("a").await.unwrap_err();
    assert!(e.to_string().contains("429"), "{e}");
    assert_eq!(fake.lock().unwrap().count("/users"), MAX_TRIES as usize);
}

fn contact(id: &str, name: &str, email: &str) -> Value {
    json!({ "id": id, "displayName": name, "emailAddresses": [{ "name": name, "address": email }], "mobilePhone": "+44 7700 900123" })
}

#[tokio::test]
async fn personal_contacts_use_delta_and_recover_from_expired_links() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (base, fake) = fake_graph().await;
    setup(&base);
    {
        let mut f = fake.lock().unwrap();
        f.on("/me/contacts?$top=1", vec![ok(json!({ "value": [{ "parentFolderId": "F0" }] }))]);
        f.on("/me/contactFolders?", vec![ok(json!({ "value": [{ "id": "F1", "displayName": "Family" }, { "id": "F2", "displayName": "Old" }] }))]);
        f.on("/me/contactFolders/F1/childFolders", vec![ok(json!({ "value": [] }))]);
        f.on("/me/contactFolders/F2/childFolders", vec![ok(json!({ "value": [] }))]);
        f.on("/me/contactFolders/F0/contacts/delta?$select", vec![ok(json!({
            "value": [contact("c1", "Dee Ross", "dee@x.com"), contact("c2", "Eli Gray", "eli@x.com")],
            "@odata.nextLink": format!("{base}/me/contactFolders/F0/contacts/delta?$skiptoken=2")
        }))]);
        f.on("/me/contactFolders/F0/contacts/delta?$skiptoken=2", vec![ok(json!({
            "value": [contact("c3", "Fay Moss", "fay@x.com")],
            "@odata.deltaLink": format!("{base}/me/contactFolders/F0/contacts/delta?$deltatoken=A")
        }))]);
        f.on("/me/contactFolders/F1/contacts/delta?$select", vec![ok(json!({
            "value": [contact("c4", "Mum", "mum@home.com")], "@odata.deltaLink": format!("{base}/me/contactFolders/F1/contacts/delta?$deltatoken=B")
        }))]);
        // A folder without delta support is listed in full instead.
        f.on("/me/contactFolders/F2/contacts/delta", vec![(400, vec![], json!({ "error": { "code": "ErrorInvalidRequest" } }))]);
        f.on("/me/contactFolders/F2/contacts?$top=100", vec![ok(json!({ "value": [contact("c5", "Gus Old", "gus@old.com")] }))]);
    }
    sync_personal("a").await.unwrap();
    assert_eq!(names(PERSONAL), vec!["Dee Ross", "Eli Gray", "Fay Moss", "Gus Old", "Mum"]);
    let mum = detail(&group(PERSONAL, "mum", None, 5).unwrap().rows[0].id).unwrap();
    assert_eq!((mum.folder.as_deref(), mum.phones[0].kind.as_str()), (Some("Family"), "mobile"));
    assert_eq!(state_get("a", "contacts:delta:F0").as_deref(), Some(format!("{base}/me/contactFolders/F0/contacts/delta?$deltatoken=A").as_str()));

    // Next time only the changes come: one removed, one renamed. Family's sync point expired (410): it starts over.
    {
        let mut f = fake.lock().unwrap();
        f.on("/me/contactFolders/F0/contacts/delta?$deltatoken=A", vec![ok(json!({
            "value": [{ "id": "c2", "@removed": { "reason": "deleted" } }, contact("c3", "Fay Moss-Lee", "fay@x.com")],
            "@odata.deltaLink": format!("{base}/me/contactFolders/F0/contacts/delta?$deltatoken=A2")
        }))]);
        f.on("/me/contactFolders/F1/contacts/delta?$deltatoken=B", vec![(410, vec![], json!({ "error": { "code": "SyncStateNotFound" } }))]);
        f.on("/me/contactFolders/F1/contacts/delta?$select", vec![ok(json!({
            "value": [contact("c6", "Dad", "dad@home.com")], "@odata.deltaLink": format!("{base}/me/contactFolders/F1/contacts/delta?$deltatoken=B2")
        }))]);
        // The "Old" folder was deleted on the server.
        f.on("/me/contactFolders?", vec![ok(json!({ "value": [{ "id": "F1", "displayName": "Family" }] }))]);
    }
    let fresh_f0 = fake.lock().unwrap().count("/me/contactFolders/F0/contacts/delta?$select");
    sync_personal("a").await.unwrap();
    assert_eq!(names(PERSONAL), vec!["Dad", "Dee Ross", "Fay Moss-Lee"], "removed, renamed, restarted folder, deleted folder");
    assert_eq!(fake.lock().unwrap().count("/me/contactFolders/F0/contacts/delta?$select"), fresh_f0, "the delta link was used, not a full listing");
    assert_eq!(state_get("a", "contacts:delta:F2"), None);
}

#[tokio::test]
async fn missing_permission_is_a_status_with_a_way_out() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (base, fake) = fake_graph().await;
    setup(&base);
    fake.lock().unwrap().on("/", vec![(403, vec![], json!({ "error": { "code": "ErrorAccessDenied" } }))]);
    assert_eq!(statuses(None).unwrap()[0].directory, "never");
    sync_account("a", true).await.expect("a refusal never fails the sync pass");
    let s = &statuses(None).unwrap()[0];
    assert_eq!((s.email.as_str(), s.granted, s.directory.as_str(), s.personal.as_str()), ("me@corp.com", false, "needs_consent", "needs_consent"));
    assert_eq!(statuses(None).unwrap().len(), 1, "app-password (IMAP) accounts have no contacts");
    // Not asked again on every background pass.
    let n = fake.lock().unwrap().seen.len();
    sync_account("a", false).await.unwrap();
    assert_eq!(fake.lock().unwrap().seen.len(), n);
    // Granted, but the organisation still refuses: "blocked" (the view explains it and offers Grant access again).
    auth::remember_account(&auth::Account { email: "me@corp.com".into(), display_name: "Me".into(), provider: "microsoft".into(), rules: false, contacts: true }).unwrap();
    sync_account("a", true).await.unwrap();
    assert_eq!(statuses(None).unwrap()[0].directory, "blocked");
    // Offline is an error to retry, not a permission problem.
    unsafe { std::env::set_var("GM_TEST_GRAPH", "http://127.0.0.1:1") };
    sync_account("a", true).await.unwrap();
    assert_eq!(statuses(None).unwrap()[0].personal, "error");
}

/// Two directory entries, one personal contact.
fn seed() {
    storage::with_db(|db| {
        let a = Entry { name: "Ana Diaz".into(), emails: vec!["ana@corp.com".into()], job_title: Some("CFO".into()), department: Some("Finance".into()), ..Default::default() };
        let b = Entry { name: "Ben Ode".into(), emails: vec!["ben@corp.com".into()], ..Default::default() };
        let c = Entry { name: "Anna Bell".into(), emails: vec!["anna@home.com".into(), "anna@work.com".into()], ..Default::default() };
        upsert(db, "a", DIRECTORY, "u1", None, &a, 1)?;
        upsert(db, "a", DIRECTORY, "u2", None, &b, 1)?;
        upsert(db, "a", PERSONAL, "c1", Some(("F0", "Contacts")), &c, 1)?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn search_suggestions_removal_and_account_deletion() {
    let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (base, fake) = fake_graph().await;
    setup(&base);
    seed();
    let o = overview("", None).unwrap();
    assert_eq!((o.directory.total, o.personal.total), (2, 1));
    assert_eq!(o.directory.rows[0].subtitle.as_deref(), Some("CFO"));
    let o = overview("an", None).unwrap();
    assert_eq!((o.directory.total, o.personal.total), (1, 1), "one search box across both groups");
    assert_eq!(overview("finance diaz", None).unwrap().directory.total, 1, "every word must match");
    assert_eq!(overview("", Some("me@gmail.com")).unwrap().directory.total, 0, "account filter");

    let s = suggest("an", "me@corp.com", 8).unwrap();
    assert_eq!(s.iter().map(|x| x.email.as_str()).collect::<Vec<_>>(), vec!["anna@home.com", "anna@work.com", "ana@corp.com"], "own contacts first");
    assert!(suggest("a", "me@corp.com", 8).unwrap().is_empty(), "two letters at least");
    assert!(suggest("an", "me@gmail.com", 8).unwrap().is_empty(), "no suggestions for Gmail / IMAP accounts");

    // "Remove cached contacts": gone, and not downloaded again until "Refresh contacts".
    remove_cached_for("me@corp.com").unwrap();
    assert_eq!(overview("", None).unwrap().directory.total, 0);
    assert_eq!(statuses(None).unwrap()[0].directory, "removed");
    sync_account("a", false).await.unwrap();
    assert!(fake.lock().unwrap().seen.is_empty(), "a paused account isn't synced in the background");
    fake.lock().unwrap().on("/users", vec![ok(json!({ "value": [user("u1", "Ana Diaz", Some("ana@corp.com"))] }))]);
    fake.lock().unwrap().on("/me/contacts", vec![ok(json!({ "value": [] }))]);
    fake.lock().unwrap().on("/me/contactFolders", vec![ok(json!({ "value": [] }))]);
    fake.lock().unwrap().on("/me/people", vec![ok(json!({ "value": [] }))]);
    let after = refresh(Some("me@corp.com")).await.unwrap();
    assert_eq!((after[0].directory.as_str(), after[0].directory_count), ("ok", 1), "Refresh brings them back");
    assert!(remove_cached_for("me@gmail.com").is_err());

    // Removing the account deletes its cached contacts.
    seed();
    crate::sync::forget_account_data("me@corp.com").unwrap();
    assert_eq!(storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM contacts", [], |r| r.get::<_, i64>(0))?)).unwrap(), 0);
    unsafe { std::env::remove_var("GM_TEST_GRAPH") };
}
