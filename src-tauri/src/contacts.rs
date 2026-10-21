//! Microsoft 365 contacts: the organisation's directory (Global Address List) and the user's own contacts, kept in
//! the local database for offline use, fast search and To/Cc suggestions. Nothing leaves this computer, and no
//! contact data is ever logged.
//!
//! - Personal contacts: every contact folder, through Graph delta queries (a full listing where delta isn't offered).
//! - Directory: /users, paged and throttled. Graph only offers /users/delta with User.Read.All, which this app doesn't
//!   ask for, so the directory is listed in full (at most daily, or on "Refresh contacts") and entries that are gone
//!   are swept out afterwards. /me/people fills in job titles and phone numbers the basic directory access can't read.
//! - 429/503 answers are retried after Retry-After; missing permission is recorded as a status, never as a sync error.

use crate::{auth, storage};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
pub const DIRECTORY: &str = "directory";
pub const PERSONAL: &str = "personal";

/// How often the background sync looks again (the "Refresh contacts" button always does).
const PERSONAL_EVERY: i64 = 30 * 60;
const DIRECTORY_EVERY: i64 = 24 * 60 * 60;
/// Throttling: a short pause between pages, and patience when Graph says "too many requests".
const PAGE_PAUSE_MS: u64 = 150;
const MAX_TRIES: u32 = 6;
const MAX_WAIT_SECS: u64 = 60;
/// The UI shows at most this many rows per group; searching narrows the list.
pub const LIST_LIMIT: i64 = 500;

const USER_FULL: &str = "id,displayName,mail,jobTitle,department,officeLocation,companyName,businessPhones,mobilePhone,accountEnabled";
const USER_BASIC: &str = "id,displayName,mail";
const CONTACT_FIELDS: &str = "id,displayName,givenName,surname,emailAddresses,businessPhones,homePhones,mobilePhone,jobTitle,department,officeLocation,companyName";

fn graph() -> String {
    #[cfg(test)]
    if let Ok(u) = std::env::var("GM_TEST_GRAPH") {
        return u;
    }
    GRAPH.to_string()
}

/// Waiting is counted in seconds in the app and in milliseconds in tests.
fn wait(units: u64) -> Duration {
    if cfg!(test) { Duration::from_millis(units) } else { Duration::from_secs(units) }
}

fn page_pause() -> Duration {
    if cfg!(test) { Duration::ZERO } else { Duration::from_millis(PAGE_PAUSE_MS) }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------- Graph requests

/// Graph refused for lack of permission (401/403): the user hasn't granted it, or the organisation blocks it.
#[derive(Debug)]
pub struct Denied;

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Microsoft 365 didn't allow reading contacts")
    }
}
impl std::error::Error for Denied {}

/// A delta link Graph no longer accepts (410): start that folder over.
#[derive(Debug)]
struct Expired;
impl std::fmt::Display for Expired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the contacts sync point expired")
    }
}
impl std::error::Error for Expired {}

/// Graph answered with an error that isn't about permission. Only the status is kept (no response text, which could
/// quote contact data).
#[derive(Debug)]
struct Failed(u16);
impl std::fmt::Display for Failed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Microsoft 365 answered {}", self.0)
    }
}
impl std::error::Error for Failed {}

/// Seconds to wait from a Retry-After header (seconds form); doubling back-off when it is missing.
fn retry_after(header: Option<&str>, attempt: u32) -> u64 {
    header.and_then(|h| h.trim().parse::<u64>().ok()).unwrap_or(1 << attempt.min(5)).clamp(1, MAX_WAIT_SECS)
}

async fn get(url: &str) -> Result<Value> {
    for attempt in 0..MAX_TRIES {
        let token = auth::access_token().await?;
        let resp = crate::http::client().get(url).bearer_auth(token).send().await?;
        let status = resp.status().as_u16();
        if (status == 429 || status == 503) && attempt + 1 < MAX_TRIES {
            let secs = retry_after(resp.headers().get("retry-after").and_then(|v| v.to_str().ok()), attempt);
            tokio::time::sleep(wait(secs)).await;
            continue;
        }
        return match status {
            200..=299 => Ok(resp.json().await?),
            401 | 403 => Err(Denied.into()),
            410 => Err(Expired.into()),
            s => Err(Failed(s).into()),
        };
    }
    Err(Failed(429).into())
}

fn is<E: std::error::Error + Send + Sync + 'static>(e: &anyhow::Error) -> bool {
    e.downcast_ref::<E>().is_some()
}

// ---------------------------------------------------------------- state

fn state_get(account_id: &str, key: &str) -> Option<String> {
    storage::with_db(|db| Ok(db.query_row("SELECT value FROM account_state WHERE account_id=?1 AND key=?2", params![account_id, key], |r| r.get(0)).optional()?))
        .ok()
        .flatten()
}

fn state_set(account_id: &str, key: &str, value: &str) -> Result<()> {
    storage::with_db(|db| {
        db.execute(
            "INSERT INTO account_state(account_id,key,value) VALUES(?1,?2,?3) ON CONFLICT(account_id,key) DO UPDATE SET value=excluded.value",
            params![account_id, key, value],
        )?;
        Ok(())
    })
}

fn state_del(account_id: &str, key: &str) -> Result<()> {
    storage::with_db(|db| Ok(db.execute("DELETE FROM account_state WHERE account_id=?1 AND key=?2", params![account_id, key]).map(|_| ())?))
}

// ---------------------------------------------------------------- storing

#[derive(Default, Debug, PartialEq)]
struct Entry {
    name: String,
    emails: Vec<String>,
    /// (kind, number): "work", "mobile", "home"
    phones: Vec<(String, String)>,
    job_title: Option<String>,
    department: Option<String>,
    office: Option<String>,
    company: Option<String>,
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn phones(v: &Value, kind: &str, out: &mut Vec<(String, String)>) {
    let mut add = |n: &Value| {
        if let Some(n) = text(n) {
            if !out.iter().any(|(_, x)| *x == n) {
                out.push((kind.to_string(), n));
            }
        }
    };
    match v.as_array() {
        Some(a) => a.iter().for_each(&mut add),
        None => add(v),
    }
}

/// A directory user worth listing: enabled (when Graph says), with a mailbox address.
fn from_user(u: &Value) -> Option<Entry> {
    if u["accountEnabled"].as_bool() == Some(false) {
        return None; // disabled accounts (room and equipment mailboxes are usually disabled accounts too)
    }
    let mail = text(&u["mail"])?;
    let mut ph = vec![];
    phones(&u["businessPhones"], "work", &mut ph);
    phones(&u["mobilePhone"], "mobile", &mut ph);
    Some(Entry {
        name: text(&u["displayName"]).unwrap_or_else(|| mail.clone()),
        emails: vec![mail],
        phones: ph,
        job_title: text(&u["jobTitle"]),
        department: text(&u["department"]),
        office: text(&u["officeLocation"]),
        company: text(&u["companyName"]),
    })
}

fn from_contact(c: &Value) -> Entry {
    let emails: Vec<String> = c["emailAddresses"].as_array().into_iter().flatten().filter_map(|e| text(&e["address"])).collect();
    let mut ph = vec![];
    phones(&c["businessPhones"], "work", &mut ph);
    phones(&c["mobilePhone"], "mobile", &mut ph);
    phones(&c["homePhones"], "home", &mut ph);
    let joined = [text(&c["givenName"]), text(&c["surname"])].into_iter().flatten().collect::<Vec<_>>().join(" ");
    let name = text(&c["displayName"]).or(Some(joined).filter(|s| !s.is_empty())).or_else(|| emails.first().cloned()).unwrap_or_else(|| "(no name)".into());
    Entry {
        name,
        emails,
        phones: ph,
        job_title: text(&c["jobTitle"]),
        department: text(&c["department"]),
        office: text(&c["officeLocation"]),
        company: text(&c["companyName"]),
    }
}

fn search_text(e: &Entry) -> String {
    let mut parts = vec![e.name.clone()];
    parts.extend(e.emails.iter().cloned());
    parts.extend([&e.job_title, &e.department, &e.company].into_iter().flatten().cloned());
    parts.join(" ").to_lowercase()
}

fn upsert(db: &rusqlite::Connection, account_id: &str, source: &str, remote_id: &str, folder: Option<(&str, &str)>, e: &Entry, generation: i64) -> Result<()> {
    let phones: Vec<Value> = e.phones.iter().map(|(k, n)| serde_json::json!({ "kind": k, "number": n })).collect();
    db.execute(
        "INSERT INTO contacts(id,account_id,source,remote_id,folder_id,folder_name,display_name,emails_json,phones_json,job_title,department,office,company,search_text,sync_gen)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
         ON CONFLICT(account_id,source,remote_id) DO UPDATE SET folder_id=excluded.folder_id, folder_name=excluded.folder_name,
           display_name=excluded.display_name, emails_json=excluded.emails_json, phones_json=excluded.phones_json, job_title=excluded.job_title,
           department=excluded.department, office=excluded.office, company=excluded.company, search_text=excluded.search_text, sync_gen=excluded.sync_gen",
        params![
            uuid::Uuid::new_v4().to_string(), account_id, source, remote_id, folder.map(|f| f.0), folder.map(|f| f.1), e.name,
            serde_json::to_string(&e.emails)?, Value::Array(phones).to_string(), e.job_title, e.department, e.office, e.company, search_text(e), generation
        ],
    )?;
    Ok(())
}

fn next_link(v: &Value) -> Option<String> {
    v["@odata.nextLink"].as_str().map(String::from)
}

// ---------------------------------------------------------------- directory

/// Lists the whole directory. Rows not seen in this listing (people who left) are removed at the end.
async fn sync_directory(account_id: &str) -> Result<usize> {
    let generation = chrono::Utc::now().timestamp_millis();
    let basic = state_get(account_id, "contacts:dir_basic").is_some();
    let first = |fields: &str| format!("{}/users?$select={fields}&$top=999", graph());
    let mut page = match get(&first(if basic { USER_BASIC } else { USER_FULL })).await {
        // Basic directory access (User.ReadBasic.All) can't read titles, phones or the enabled flag: ask for less.
        Err(e) if is::<Denied>(&e) && !basic => {
            let p = get(&first(USER_BASIC)).await?;
            state_set(account_id, "contacts:dir_basic", "1")?;
            p
        }
        r => r?,
    };
    let mut n = 0;
    loop {
        let users = page["value"].as_array().cloned().unwrap_or_default();
        storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            for u in &users {
                if let (Some(id), Some(e)) = (u["id"].as_str(), from_user(u)) {
                    upsert(&tx, account_id, DIRECTORY, id, None, &e, generation)?;
                    n += 1;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        let Some(next) = next_link(&page) else { break };
        tokio::time::sleep(page_pause()).await;
        page = get(&next).await?;
    }
    storage::with_db(|db| {
        Ok(db.execute("DELETE FROM contacts WHERE account_id=?1 AND source=?2 AND sync_gen<>?3", params![account_id, DIRECTORY, generation]).map(|_| ())?)
    })?;
    // The people this user works with carry the details basic directory access leaves out.
    if let Err(e) = enrich_from_people(account_id).await {
        if !is::<Denied>(&e) {
            eprintln!("[graphite] contacts: couldn't read People details ({e})");
        }
    }
    Ok(n)
}

/// Fills in job title, department, office, company and phones of directory entries from /me/people.
async fn enrich_from_people(account_id: &str) -> Result<()> {
    let url = format!("{}/me/people?$top=500&$select=scoredEmailAddresses,phones,jobTitle,department,officeLocation,companyName", graph());
    let v = get(&url).await?;
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        for p in v["value"].as_array().into_iter().flatten() {
            for addr in p["scoredEmailAddresses"].as_array().into_iter().flatten().filter_map(|a| text(&a["address"])) {
                let row: Option<(String, String)> = tx
                    .query_row(
                        "SELECT id, phones_json FROM contacts WHERE account_id=?1 AND source=?2 AND lower(emails_json) LIKE '%\"' || lower(?3) || '\"%'",
                        params![account_id, DIRECTORY, addr],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                let Some((id, phones_json)) = row else { continue };
                let mut ph: Vec<Value> = serde_json::from_str(&phones_json).unwrap_or_default();
                if ph.is_empty() {
                    for x in p["phones"].as_array().into_iter().flatten() {
                        if let Some(n) = text(&x["number"]) {
                            let kind = match x["type"].as_str() { Some("mobile") => "mobile", Some("home") => "home", _ => "work" };
                            ph.push(serde_json::json!({ "kind": kind, "number": n }));
                        }
                    }
                }
                tx.execute(
                    "UPDATE contacts SET job_title=COALESCE(job_title,?2), department=COALESCE(department,?3), office=COALESCE(office,?4),
                       company=COALESCE(company,?5), phones_json=?6,
                       search_text=search_text || ' ' || lower(COALESCE(?2,'') || ' ' || COALESCE(?3,'') || ' ' || COALESCE(?5,''))
                     WHERE id=?1",
                    params![id, text(&p["jobTitle"]), text(&p["department"]), text(&p["officeLocation"]), text(&p["companyName"]), Value::Array(ph).to_string()],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    })
}

// ---------------------------------------------------------------- personal contacts

/// The user's contact folders: the default "Contacts" folder and every folder (and sub-folder) they made.
async fn contact_folders(account_id: &str) -> Result<Vec<(String, String)>> {
    let g = graph();
    let mut out: Vec<(String, String)> = vec![];
    // Graph lists only the folders the user made; the default folder's id comes from one of its contacts.
    let probe = get(&format!("{g}/me/contacts?$top=1&$select=parentFolderId")).await?;
    let default = probe["value"][0]["parentFolderId"].as_str().map(String::from).or_else(|| state_get(account_id, "contacts:default_folder"));
    if let Some(d) = &default {
        state_set(account_id, "contacts:default_folder", d)?;
        out.push((d.clone(), "Contacts".into()));
    }
    let mut todo = vec![format!("{g}/me/contactFolders?$top=100&$select=id,displayName")];
    while let Some(url) = todo.pop() {
        let page = get(&url).await?;
        for f in page["value"].as_array().into_iter().flatten() {
            if let Some(id) = f["id"].as_str() {
                if !out.iter().any(|(x, _)| x == id) {
                    out.push((id.to_string(), text(&f["displayName"]).unwrap_or_else(|| "Contacts".into())));
                    todo.push(format!("{g}/me/contactFolders/{id}/childFolders?$top=100&$select=id,displayName"));
                }
            }
        }
        if let Some(next) = next_link(&page) {
            todo.push(next);
        }
    }
    Ok(out)
}

/// One contact folder: changes since the last delta link, or everything the first time.
async fn sync_folder(account_id: &str, folder_id: &str, folder_name: &str) -> Result<usize> {
    let key = format!("contacts:delta:{folder_id}");
    let fresh_url = format!("{}/me/contactFolders/{folder_id}/contacts/delta?$select={CONTACT_FIELDS}", graph());
    let saved = state_get(account_id, &key);
    let (mut url, mut fresh) = match saved {
        Some(u) => (u, false),
        None => (fresh_url.clone(), true),
    };
    let generation = chrono::Utc::now().timestamp_millis();
    let mut n = 0;
    loop {
        let page = match get(&url).await {
            Ok(p) => p,
            Err(e) if is::<Expired>(&e) && !fresh => {
                // Graph forgot this sync point: start the folder over (the full answer replaces what is here).
                state_del(account_id, &key)?;
                url = fresh_url.clone();
                fresh = true;
                continue;
            }
            Err(e) if fresh && matches!(e.downcast_ref::<Failed>(), Some(Failed(400 | 404 | 501))) => {
                return list_folder(account_id, folder_id, folder_name).await; // no delta for this folder
            }
            Err(e) => return Err(e),
        };
        storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            for c in page["value"].as_array().into_iter().flatten() {
                let Some(id) = c["id"].as_str() else { continue };
                if c.get("@removed").is_some() {
                    tx.execute("DELETE FROM contacts WHERE account_id=?1 AND source=?2 AND remote_id=?3", params![account_id, PERSONAL, id])?;
                } else {
                    upsert(&tx, account_id, PERSONAL, id, Some((folder_id, folder_name)), &from_contact(c), generation)?;
                }
                n += 1;
            }
            tx.commit()?;
            Ok(())
        })?;
        if let Some(next) = next_link(&page) {
            tokio::time::sleep(page_pause()).await;
            url = next;
            continue;
        }
        if let Some(delta) = page["@odata.deltaLink"].as_str() {
            state_set(account_id, &key, delta)?;
        }
        break;
    }
    if fresh {
        sweep_folder(account_id, folder_id, generation)?;
    }
    Ok(n)
}

/// Folders without delta support: list everything and drop what wasn't listed.
async fn list_folder(account_id: &str, folder_id: &str, folder_name: &str) -> Result<usize> {
    let generation = chrono::Utc::now().timestamp_millis();
    let mut url = format!("{}/me/contactFolders/{folder_id}/contacts?$top=100&$select={CONTACT_FIELDS}", graph());
    let mut n = 0;
    loop {
        let page = get(&url).await?;
        storage::with_db(|db| {
            let tx = db.unchecked_transaction()?;
            for c in page["value"].as_array().into_iter().flatten() {
                if let Some(id) = c["id"].as_str() {
                    upsert(&tx, account_id, PERSONAL, id, Some((folder_id, folder_name)), &from_contact(c), generation)?;
                    n += 1;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        match next_link(&page) {
            Some(next) => {
                tokio::time::sleep(page_pause()).await;
                url = next;
            }
            None => break,
        }
    }
    sweep_folder(account_id, folder_id, generation)?;
    Ok(n)
}

fn sweep_folder(account_id: &str, folder_id: &str, generation: i64) -> Result<()> {
    storage::with_db(|db| {
        Ok(db
            .execute("DELETE FROM contacts WHERE account_id=?1 AND source=?2 AND folder_id=?3 AND sync_gen<>?4", params![account_id, PERSONAL, folder_id, generation])
            .map(|_| ())?)
    })
}

async fn sync_personal(account_id: &str) -> Result<usize> {
    let folders = contact_folders(account_id).await?;
    let mut n = 0;
    for (id, name) in &folders {
        n += sync_folder(account_id, id, name).await?;
    }
    // Folders deleted on the server take their contacts with them.
    storage::with_db(|db| {
        let ids: Vec<&str> = folders.iter().map(|(id, _)| id.as_str()).collect();
        let mut st = db.prepare("SELECT DISTINCT folder_id FROM contacts WHERE account_id=?1 AND source=?2")?;
        let here: Vec<Option<String>> = st.query_map(params![account_id, PERSONAL], |r| r.get(0))?.filter_map(|r| r.ok()).collect();
        for f in here.into_iter().flatten().filter(|f| !ids.contains(&f.as_str())) {
            db.execute("DELETE FROM contacts WHERE account_id=?1 AND source=?2 AND folder_id=?3", params![account_id, PERSONAL, f])?;
            db.execute("DELETE FROM account_state WHERE account_id=?1 AND key=?2", params![account_id, format!("contacts:delta:{f}")])?;
        }
        Ok(())
    })?;
    Ok(n)
}

// ---------------------------------------------------------------- sync passes

/// Only one contacts sync at a time (background pass, Refresh button, Grant access).
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn granted(email: &str) -> bool {
    auth::accounts().iter().any(|a| a.email.eq_ignore_ascii_case(email) && a.contacts)
}

/// Records how one part went: "ok", "needs_consent" (not granted yet), "blocked" (granted, but the organisation
/// refuses), or "error" (try again later).
fn record(account_id: &str, email: &str, part: &str, r: &Result<usize>) -> Result<()> {
    let status = match r {
        Ok(_) => "ok",
        Err(e) if is::<Denied>(e) => if granted(email) { "blocked" } else { "needs_consent" },
        Err(_) => "error",
    };
    state_set(account_id, &format!("contacts:{part}:status"), status)?;
    if r.is_ok() || status != "error" {
        // A refusal is not retried every pass either; the user can grant access and refresh.
        state_set(account_id, &format!("contacts:{part}:last"), &now().to_string())?;
    }
    if let Err(e) = r {
        if status == "error" {
            eprintln!("[graphite] contacts sync ({part}) for {email} failed: {e}");
        }
    }
    Ok(())
}

fn due(account_id: &str, part: &str, every: i64) -> bool {
    state_get(account_id, &format!("contacts:{part}:last")).and_then(|t| t.parse::<i64>().ok()).is_none_or(|t| now() - t >= every)
}

/// Brings one Microsoft account's contacts up to date (inside auth::in_account for it). `force` = the user asked.
/// Never fails because of contacts: problems are recorded for the Contacts view to show.
pub async fn sync_account(account_id: &str, force: bool) -> Result<()> {
    let _one = LOCK.lock().await;
    let email: String = storage::with_db(|db| Ok(db.query_row("SELECT email FROM accounts WHERE id=?1", [account_id], |r| r.get(0))?))?;
    if force {
        state_del(account_id, "contacts:paused")?;
    } else if state_get(account_id, "contacts:paused").is_some() {
        return Ok(()); // the user removed the cached contacts; they come back with "Refresh contacts"
    }
    if force || due(account_id, PERSONAL, PERSONAL_EVERY) {
        let r = sync_personal(account_id).await;
        record(account_id, &email, PERSONAL, &r)?;
    }
    if force || due(account_id, DIRECTORY, DIRECTORY_EVERY) {
        let r = sync_directory(account_id).await;
        record(account_id, &email, DIRECTORY, &r)?;
    }
    Ok(())
}

fn account_id_of(email: &str) -> Result<String> {
    microsoft_accounts(Some(email))?.into_iter().next().map(|(id, _)| id).ok_or_else(|| anyhow!("Contacts are available for Microsoft 365 accounts."))
}

/// "Refresh contacts": syncs every Microsoft account now (or just `only`), and returns how each one stands.
pub async fn refresh(only: Option<&str>) -> Result<Vec<AccountStatus>> {
    for acc in auth::accounts().into_iter().filter(|a| a.provider == "microsoft" && only.is_none_or(|o| a.email.eq_ignore_ascii_case(o))) {
        let id = crate::sync::ensure_account_pub(&acc)?;
        auth::in_account(Some(acc.email.clone()), sync_account(&id, true)).await?;
    }
    statuses(only)
}

/// "Remove cached contacts" for one account (by email).
pub fn remove_cached_for(email: &str) -> Result<()> {
    remove_cached(&account_id_of(email)?)
}

/// "Remove cached contacts": forgets one account's contacts and stops syncing them until "Refresh contacts".
pub fn remove_cached(account_id: &str) -> Result<()> {
    storage::with_db(|db| {
        let tx = db.unchecked_transaction()?;
        tx.execute("DELETE FROM contacts WHERE account_id=?1", [account_id])?;
        tx.execute("DELETE FROM account_state WHERE account_id=?1 AND key LIKE 'contacts:%'", [account_id])?;
        tx.execute("INSERT INTO account_state(account_id,key,value) VALUES(?1,'contacts:paused','1')", [account_id])?;
        tx.commit()?;
        Ok(())
    })
}

// ---------------------------------------------------------------- reading (for the UI)

#[derive(Serialize, Debug)]
pub struct Row {
    pub id: String,
    pub source: String,
    pub account_email: String,
    pub name: String,
    pub email: Option<String>,
    /// Job title, or the department / company when there is no title.
    pub subtitle: Option<String>,
}

#[derive(Serialize, Debug, Default)]
pub struct Group {
    pub total: i64,
    pub rows: Vec<Row>,
}

#[derive(Serialize, Debug)]
pub struct AccountStatus {
    pub email: String,
    pub granted: bool,
    /// "ok", "never" (not synced yet), "needs_consent", "blocked", "error" or "removed"
    pub directory: String,
    pub personal: String,
    pub directory_count: i64,
    pub personal_count: i64,
    pub last_sync: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct Overview {
    pub directory: Group,
    pub personal: Group,
    pub accounts: Vec<AccountStatus>,
}

#[derive(Serialize, Debug)]
pub struct Phone {
    pub kind: String,
    pub number: String,
}

#[derive(Serialize, Debug)]
pub struct Detail {
    pub id: String,
    pub source: String,
    pub account_email: String,
    pub folder: Option<String>,
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<Phone>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub office: Option<String>,
    pub company: Option<String>,
}

/// Microsoft accounts (rows of the accounts table): (id, email).
fn microsoft_accounts(only: Option<&str>) -> Result<Vec<(String, String)>> {
    storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT id, email FROM accounts WHERE COALESCE(provider,'microsoft')='microsoft' AND (?1 IS NULL OR lower(email)=lower(?1)) ORDER BY created_at",
        )?;
        let rows = st.query_map([only], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

/// The search words, each of which must appear (in any order).
fn terms(query: &str) -> Vec<String> {
    query.to_lowercase().split_whitespace().map(String::from).collect()
}

fn group(source: &str, query: &str, account: Option<&str>, limit: i64) -> Result<Group> {
    let t = terms(query);
    let mut filter = String::from("c.source=?1 AND COALESCE(a.provider,'microsoft')='microsoft' AND (?2 IS NULL OR lower(a.email)=lower(?2))");
    for i in 0..t.len() {
        filter.push_str(&format!(" AND instr(c.search_text, ?{}) > 0", i + 3));
    }
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(source.to_string()), Box::new(account.map(String::from))];
    for w in &t {
        args.push(Box::new(w.clone()));
    }
    storage::with_db(|db| {
        let from = "contacts c JOIN accounts a ON a.id=c.account_id";
        let total: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {from} WHERE {filter}"), rusqlite::params_from_iter(args.iter()), |r| r.get(0))?;
        let mut st = db.prepare(&format!(
            "SELECT c.id, c.source, a.email, c.display_name, c.emails_json, COALESCE(c.job_title, c.department, c.company)
             FROM {from} WHERE {filter} ORDER BY c.display_name COLLATE NOCASE, c.id LIMIT {limit}"
        ))?;
        let rows = st
            .query_map(rusqlite::params_from_iter(args.iter()), |r| {
                let emails: Vec<String> = serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default();
                Ok(Row { id: r.get(0)?, source: r.get(1)?, account_email: r.get(2)?, name: r.get(3)?, email: emails.into_iter().next(), subtitle: r.get(5)? })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(Group { total, rows })
    })
}

pub fn statuses(account: Option<&str>) -> Result<Vec<AccountStatus>> {
    microsoft_accounts(account)?
        .into_iter()
        .map(|(id, email)| {
            let paused = state_get(&id, "contacts:paused").is_some();
            let status = |part: &str| if paused { "removed".to_string() } else { state_get(&id, &format!("contacts:{part}:status")).unwrap_or_else(|| "never".into()) };
            let count = |source: &str| -> Result<i64> {
                storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM contacts WHERE account_id=?1 AND source=?2", params![id, source], |r| r.get(0))?))
            };
            let last = [DIRECTORY, PERSONAL].iter().filter_map(|p| state_get(&id, &format!("contacts:{p}:last")).and_then(|t| t.parse().ok())).max();
            Ok(AccountStatus {
                granted: granted(&email),
                directory: status(DIRECTORY),
                personal: status(PERSONAL),
                directory_count: count(DIRECTORY)?,
                personal_count: count(PERSONAL)?,
                last_sync: last,
                email,
            })
        })
        .collect()
}

/// What the Contacts view shows: both groups (filtered by the search and the account), and each account's state.
pub fn overview(query: &str, account: Option<&str>) -> Result<Overview> {
    Ok(Overview { directory: group(DIRECTORY, query, account, LIST_LIMIT)?, personal: group(PERSONAL, query, account, LIST_LIMIT)?, accounts: statuses(account)? })
}

pub fn detail(id: &str) -> Result<Detail> {
    storage::with_db(|db| {
        db.query_row(
            "SELECT c.id, c.source, a.email, c.folder_name, c.display_name, c.emails_json, c.phones_json, c.job_title, c.department, c.office, c.company
             FROM contacts c JOIN accounts a ON a.id=c.account_id WHERE c.id=?1",
            [id],
            |r| {
                let phones: Vec<Value> = serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default();
                Ok(Detail {
                    id: r.get(0)?,
                    source: r.get(1)?,
                    account_email: r.get(2)?,
                    folder: r.get(3)?,
                    name: r.get(4)?,
                    emails: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                    phones: phones.iter().map(|p| Phone { kind: p["kind"].as_str().unwrap_or("work").into(), number: p["number"].as_str().unwrap_or_default().into() }).collect(),
                    job_title: r.get(7)?,
                    department: r.get(8)?,
                    office: r.get(9)?,
                    company: r.get(10)?,
                })
            },
        )
        .optional()?
        .ok_or(anyhow!("That contact is no longer here."))
    })
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub email: String,
}

/// To/Cc suggestions while writing from a Microsoft account: its own contacts first, then the directory; names and
/// addresses that start with what was typed come before ones that merely contain it.
pub fn suggest(query: &str, account: &str, limit: usize) -> Result<Vec<Suggestion>> {
    let q = query.trim().to_lowercase();
    if q.chars().count() < 2 || microsoft_accounts(Some(account))?.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<(String, String, String)> = storage::with_db(|db| {
        let mut st = db.prepare(
            "SELECT c.display_name, c.emails_json, c.source FROM contacts c JOIN accounts a ON a.id=c.account_id
             WHERE lower(a.email)=lower(?1) AND instr(c.search_text, ?2) > 0
             ORDER BY c.source='personal' DESC, (instr(lower(c.display_name), ?2)=1 OR instr(c.emails_json, '\"' || ?2)>0) DESC, c.display_name COLLATE NOCASE
             LIMIT 200",
        )?;
        let r = st.query_map(params![account, q], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(r.filter_map(|x| x.ok()).collect())
    })?;
    let mut out: Vec<Suggestion> = vec![];
    for (name, emails, _) in rows {
        for e in serde_json::from_str::<Vec<String>>(&emails).unwrap_or_default() {
            if out.len() < limit && !out.iter().any(|s| s.email.eq_ignore_ascii_case(&e)) {
                out.push(Suggestion { name: name.clone(), email: e });
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "contacts_tests.rs"]
mod tests;
