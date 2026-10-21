//! Mail rules. They live on the server (Outlook inbox rules / Gmail filters) so they also work when the app is closed;
//! a local copy is kept for listing and for "run on existing mail". IMAP accounts have no server rules: theirs are
//! kept here only and run on new Inbox mail each time the account syncs.

use crate::{auth, google, outbox, storage};
use anyhow::{Result, anyhow, bail};
use reqwest::Method;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

fn graph() -> String {
    #[cfg(test)]
    if let Ok(u) = std::env::var("GM_TEST_GRAPH") {
        return u;
    }
    GRAPH.to_string()
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Rule {
    pub id: Option<String>,
    /// Email of the account the rule belongs to.
    pub account: String,
    pub name: String,
    pub enabled: bool,
    pub from: String,
    pub subject: String,
    pub to: String,
    pub body: String,
    /// Local id of the folder to move matching mail into.
    pub move_to: Option<String>,
    pub mark_read: bool,
    /// Gmail: star. Outlook: mark as important (Outlook rules cannot set a flag).
    pub flag: bool,
    pub delete: bool,
}

#[derive(Serialize)]
pub struct RuleView {
    #[serde(flatten)]
    pub rule: Rule,
    pub folder_name: Option<String>,
    pub provider: String,
}

const NEEDS_PERMISSION: &str = "Rules need one extra permission for this account. Use “Allow rules” first.";

fn clean(mut r: Rule) -> Result<Rule> {
    for s in [&mut r.from, &mut r.subject, &mut r.to, &mut r.body, &mut r.name] {
        *s = s.trim().to_string();
    }
    if r.from.is_empty() && r.subject.is_empty() && r.to.is_empty() && r.body.is_empty() {
        bail!("Add at least one condition (who it's from, a word in the subject, …).");
    }
    if r.move_to.is_none() && !r.mark_read && !r.flag && !r.delete {
        bail!("Choose what the rule should do.");
    }
    if r.delete && (r.move_to.is_some() || r.mark_read || r.flag) {
        bail!("“Delete” can't be combined with other actions.");
    }
    if r.name.is_empty() {
        r.name = [&r.from, &r.subject, &r.to, &r.body].iter().find(|s| !s.is_empty()).map(|s| format!("Rule: {s}")).unwrap_or_default();
    }
    Ok(r)
}

fn is_address(s: &str) -> bool {
    s.contains('@') && !s.contains(char::is_whitespace)
}

/// The Outlook (Microsoft Graph) form of a rule.
pub fn outlook_rule(r: &Rule, folder_remote: Option<&str>, sequence: i64) -> Value {
    let mut cond = serde_json::Map::new();
    if is_address(&r.from) {
        cond.insert("fromAddresses".into(), json!([{ "emailAddress": { "address": r.from } }]));
    } else if !r.from.is_empty() {
        cond.insert("senderContains".into(), json!([r.from]));
    }
    if !r.subject.is_empty() {
        cond.insert("subjectContains".into(), json!([r.subject]));
    }
    if !r.to.is_empty() {
        cond.insert("recipientContains".into(), json!([r.to]));
    }
    if !r.body.is_empty() {
        cond.insert("bodyContains".into(), json!([r.body]));
    }
    let mut act = serde_json::Map::new();
    if let Some(f) = folder_remote {
        act.insert("moveToFolder".into(), json!(f));
    }
    if r.mark_read {
        act.insert("markAsRead".into(), json!(true));
    }
    if r.flag {
        act.insert("markImportance".into(), json!("high"));
    }
    if r.delete {
        act.insert("delete".into(), json!(true));
    }
    json!({ "displayName": r.name, "sequence": sequence, "isEnabled": r.enabled, "conditions": cond, "actions": act })
}

/// The Gmail filter form of a rule. `folder_remote` is the label id (INBOX, ALL_MAIL for Archive, TRASH, SPAM or a label).
pub fn gmail_filter(r: &Rule, folder_remote: Option<&str>) -> Value {
    let mut criteria = serde_json::Map::new();
    for (k, v) in [("from", &r.from), ("to", &r.to), ("subject", &r.subject)] {
        if !v.is_empty() {
            criteria.insert(k.into(), json!(v));
        }
    }
    if !r.body.is_empty() {
        criteria.insert("query".into(), json!(format!("\"{}\"", r.body.replace('"', ""))));
    }
    let (mut add, mut remove): (Vec<&str>, Vec<&str>) = (vec![], vec![]);
    match folder_remote {
        None | Some("INBOX") => {}
        Some("ALL_MAIL") => remove.push("INBOX"),
        Some(label) => {
            add.push(label);
            remove.push("INBOX");
        }
    }
    if r.mark_read {
        remove.push("UNREAD");
    }
    if r.flag {
        add.push("STARRED");
    }
    if r.delete {
        add.push("TRASH");
        remove.push("INBOX");
    }
    json!({ "criteria": criteria, "action": { "addLabelIds": add, "removeLabelIds": remove } })
}

// ---------------------------------------------------------------- storage

struct Row {
    rule: Rule,
    remote_id: Option<String>,
    account_id: String,
    provider: String,
}

fn load(id: &str) -> Result<Row> {
    storage::with_db(|db| {
        db.query_row(
            "SELECT r.id, a.email, r.name, r.enabled, r.from_text, r.subject_text, r.to_text, r.body_text, r.move_to_folder,
                    r.mark_read, r.flag, r.del, r.remote_id, a.id, COALESCE(a.provider,'microsoft')
             FROM rules r JOIN accounts a ON a.id=r.account_id WHERE r.id=?1",
            [id],
            |r| {
                Ok(Row {
                    rule: Rule {
                        id: Some(r.get(0)?),
                        account: r.get(1)?,
                        name: r.get(2)?,
                        enabled: r.get(3)?,
                        from: r.get(4)?,
                        subject: r.get(5)?,
                        to: r.get(6)?,
                        body: r.get(7)?,
                        move_to: r.get(8)?,
                        mark_read: r.get(9)?,
                        flag: r.get(10)?,
                        delete: r.get(11)?,
                    },
                    remote_id: r.get(12)?,
                    account_id: r.get(13)?,
                    provider: r.get(14)?,
                })
            },
        )
        .optional()?
        .ok_or(anyhow!("That rule no longer exists."))
    })
}

pub fn list() -> Result<Vec<RuleView>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT r.id FROM rules r JOIN accounts a ON a.id=r.account_id ORDER BY a.created_at, r.created_at")?;
        let ids: Vec<String> = st.query_map([], |r| r.get(0))?.filter_map(|r| r.ok()).collect();
        Ok(ids)
    })?
    .into_iter()
    .map(|id| {
        let row = load(&id)?;
        let folder_name = match &row.rule.move_to {
            Some(f) => storage::with_db(|db| Ok(db.query_row("SELECT display_name FROM folders WHERE id=?1", [f], |r| r.get::<_, String>(0)).optional()?))?,
            None => None,
        };
        Ok(RuleView { rule: row.rule, folder_name, provider: row.provider })
    })
    .collect()
}

/// (account row id, provider) of an account, and whether it may manage rules.
fn account_info(email: &str) -> Result<(String, String)> {
    let r: Option<(String, String)> = storage::with_db(|db| {
        Ok(db
            .query_row("SELECT id, COALESCE(provider,'microsoft') FROM accounts WHERE lower(email)=lower(?1)", [email], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    })?;
    let (id, provider) = r.ok_or_else(|| anyhow!("Unknown account {email}"))?;
    if !cfg!(test) && !auth::accounts().iter().any(|a| a.email.eq_ignore_ascii_case(email) && a.rules) {
        bail!("{NEEDS_PERMISSION}");
    }
    Ok((id, provider))
}

/// Remote id of the target folder, checking it belongs to the rule's account.
fn folder_remote(folder: &Option<String>, account_id: &str) -> Result<Option<String>> {
    let Some(f) = folder else { return Ok(None) };
    let r: Option<String> = storage::with_db(|db| {
        Ok(db.query_row("SELECT remote_id FROM folders WHERE id=?1 AND account_id=?2", params![f, account_id], |r| r.get(0)).optional()?)
    })?;
    r.map(Some).ok_or_else(|| anyhow!("That folder belongs to a different account."))
}

fn server_error(status: u16, v: &Value, what: &str) -> anyhow::Error {
    if status == 401 || status == 403 {
        return anyhow!("{NEEDS_PERMISSION}");
    }
    anyhow!("Couldn't {what}: {}", v["error"]["message"].as_str().unwrap_or("the server refused"))
}

// ---------------------------------------------------------------- server calls

async fn push_new(row_account_email: &str, provider: &str, r: &Rule, folder_remote: Option<&str>, sequence: i64) -> Result<Option<String>> {
    if provider == crate::imap::PROVIDER {
        return Ok(None); // kept on this computer only
    }
    auth::in_account(Some(row_account_email.to_string()), async {
        let (s, v) = if provider == "google" {
            if !r.enabled {
                return Ok(None); // Gmail filters have no "off" switch: a disabled rule simply isn't on the server
            }
            google::call(Method::POST, &google::url(&google::gmail(), &["settings", "filters"], &[]), Some(gmail_filter(r, folder_remote))).await?
        } else {
            google::call(Method::POST, &format!("{}/me/mailFolders/inbox/messageRules", graph()), Some(outlook_rule(r, folder_remote, sequence))).await?
        };
        if !(200..300).contains(&s) {
            return Err(server_error(s, &v, "save the rule"));
        }
        Ok(v["id"].as_str().map(String::from))
    })
    .await
}

async fn remove_remote(email: &str, provider: &str, remote: &str) -> Result<()> {
    if provider == crate::imap::PROVIDER {
        return Ok(());
    }
    auth::in_account(Some(email.to_string()), async {
        let u = if provider == "google" {
            google::url(&google::gmail(), &["settings", "filters", remote], &[])
        } else {
            format!("{}/me/mailFolders/inbox/messageRules/{remote}", graph())
        };
        let (s, v) = google::call(Method::DELETE, &u, None).await?;
        if (200..300).contains(&s) || s == 404 {
            Ok(())
        } else {
            Err(server_error(s, &v, "remove the rule"))
        }
    })
    .await
}

// ---------------------------------------------------------------- public operations

pub async fn create(rule: Rule) -> Result<String> {
    let r = clean(rule)?;
    let (account_id, provider) = account_info(&r.account)?;
    let folder = folder_remote(&r.move_to, &account_id)?;
    let n: i64 = storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM rules WHERE account_id=?1", [&account_id], |x| x.get(0))?))?;
    let remote = push_new(&r.account, &provider, &r, folder.as_deref(), n + 1).await?;
    let id = uuid::Uuid::new_v4().to_string();
    storage::with_db(|db| {
        db.execute(
            "INSERT INTO rules(id,account_id,name,enabled,from_text,subject_text,to_text,body_text,move_to_folder,mark_read,flag,del,remote_id,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![id, account_id, r.name, r.enabled, r.from, r.subject, r.to, r.body, r.move_to, r.mark_read, r.flag, r.delete, remote, chrono::Utc::now().timestamp()],
        )?;
        Ok(())
    })?;
    Ok(id)
}

/// Blocks the sender of a message: a server rule that sends their future mail to Junk (Gmail: a filter to Spam).
/// Returns the blocked address.
pub async fn block_sender(message_id: &str) -> Result<String> {
    let (account, sender) = outbox::sender_of(message_id)?;
    let junk = outbox::junk_folder_of(message_id)?;
    create(Rule { account, name: format!("Block {sender}"), enabled: true, from: sender.clone(), move_to: Some(junk), ..Default::default() }).await?;
    Ok(sender)
}

/// Blocks an address for one account (e.g. from the Contacts view): the same rule as Block sender.
pub async fn block_address(account: &str, address: &str) -> Result<String> {
    let sender = address.trim().to_string();
    if !is_address(&sender) {
        bail!("“{sender}” is not an email address.");
    }
    let junk: String = storage::with_db(|db| {
        db.query_row(
            "SELECT f.id FROM folders f JOIN accounts a ON a.id=f.account_id WHERE lower(a.email)=lower(?1) AND f.well_known_name='junkemail'",
            [account],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(anyhow!("This account has no Junk folder yet. Try again after the next sync."))
    })?;
    create(Rule { account: account.to_string(), name: format!("Block {sender}"), enabled: true, from: sender.clone(), move_to: Some(junk), ..Default::default() }).await?;
    Ok(sender)
}

pub async fn update(rule: Rule) -> Result<()> {
    let id = rule.id.clone().ok_or(anyhow!("Missing rule id"))?;
    let old = load(&id)?;
    let mut r = clean(rule)?;
    r.account = old.rule.account.clone(); // a rule can't move between accounts
    let (account_id, provider) = account_info(&r.account)?;
    let folder = folder_remote(&r.move_to, &account_id)?;
    let remote = if provider == "google" {
        // Gmail filters can't be edited: replace the old one.
        if let Some(old_remote) = &old.remote_id {
            remove_remote(&r.account, &provider, old_remote).await?;
        }
        push_new(&r.account, &provider, &r, folder.as_deref(), 0).await?
    } else {
        match &old.remote_id {
            Some(rid) => {
                let body = outlook_rule(&r, folder.as_deref(), 1);
                let (s, v) = auth::in_account(Some(r.account.clone()), google::call(Method::PATCH, &format!("{}/me/mailFolders/inbox/messageRules/{rid}", graph()), Some(body))).await?;
                if !(200..300).contains(&s) {
                    return Err(server_error(s, &v, "save the rule"));
                }
                Some(rid.clone())
            }
            None => push_new(&r.account, &provider, &r, folder.as_deref(), 1).await?,
        }
    };
    storage::with_db(|db| {
        db.execute(
            "UPDATE rules SET name=?1,enabled=?2,from_text=?3,subject_text=?4,to_text=?5,body_text=?6,move_to_folder=?7,mark_read=?8,flag=?9,del=?10,remote_id=?11 WHERE id=?12",
            params![r.name, r.enabled, r.from, r.subject, r.to, r.body, r.move_to, r.mark_read, r.flag, r.delete, remote, id],
        )?;
        Ok(())
    })
}

pub async fn set_enabled(id: &str, on: bool) -> Result<()> {
    let mut r = load(id)?.rule;
    r.enabled = on;
    update(r).await
}

pub async fn delete(id: &str) -> Result<()> {
    let row = load(id)?;
    account_info(&row.rule.account)?;
    if let Some(remote) = &row.remote_id {
        remove_remote(&row.rule.account, &row.provider, remote).await?;
    }
    storage::with_db(|db| Ok(db.execute("DELETE FROM rules WHERE id=?1", [id]).map(|_| ())?))
}

// ---------------------------------------------------------------- run on existing mail

fn has(hay: &str, needle: &str) -> bool {
    needle.is_empty() || hay.to_lowercase().contains(&needle.to_lowercase())
}

/// Applies a rule to the mail already in `folder_id` (local). Returns how many messages matched.
/// Changes go through the normal offline queue, so they reach the server like any other action.
pub fn run_now(rule_id: &str, folder_id: &str) -> Result<usize> {
    let row = load(rule_id)?;
    let r = &row.rule;
    let in_account: bool = storage::with_db(|db| Ok(db.query_row("SELECT COUNT(*) FROM folders WHERE id=?1 AND account_id=?2", params![folder_id, row.account_id], |x| x.get::<_, i64>(0))? > 0))?;
    if !in_account {
        bail!("Choose a folder of the same account as the rule.");
    }
    let candidates = storage::with_db(|db| {
        let mut st = db.prepare(&format!("{CANDIDATES} WHERE (m.folder_id=?1 OR m.id IN (SELECT message_id FROM message_labels WHERE folder_id=?1))"))?;
        let rows = st.query_map([folder_id], candidate)?;
        Ok(rows.filter_map(|x| x.ok()).collect::<Vec<_>>())
    })?;
    apply(r, &row.account_id, candidates)
}

/// What a rule looks at in a message: (id, sender, subject, recipients, text, folder, read, flagged).
type Candidate = (String, String, String, String, String, String, bool, bool);

const CANDIDATES: &str = "SELECT m.id, COALESCE(m.sender_name,'') || ' ' || COALESCE(m.sender_address,''), COALESCE(m.subject,''),
        COALESCE(m.to_recipients_json,'') || ' ' || COALESCE(m.cc_recipients_json,''),
        COALESCE((SELECT body_plaintext FROM messages_fts WHERE message_id=m.id),''), m.folder_id, m.is_read, m.is_flagged
     FROM messages m";

fn candidate(r: &rusqlite::Row) -> rusqlite::Result<Candidate> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
}

/// Applies one rule to some messages through the offline queue. Returns how many matched.
fn apply(r: &Rule, account_id: &str, candidates: Vec<Candidate>) -> Result<usize> {
    let trash: Option<String> = storage::with_db(|db| {
        Ok(db.query_row("SELECT id FROM folders WHERE account_id=?1 AND well_known_name='deleteditems'", [account_id], |x| x.get(0)).optional()?)
    })?;
    let mut matched = 0;
    for (id, sender, subject, to, body, current, read, flagged) in candidates {
        // Mail already in Trash is left alone by a "delete" rule.
        if !(has(&sender, &r.from) && has(&subject, &r.subject) && has(&to, &r.to) && has(&body, &r.body)) {
            continue;
        }
        if r.delete && trash.as_deref() == Some(current.as_str()) {
            continue;
        }
        matched += 1;
        if r.delete {
            outbox::delete(&id)?;
            continue;
        }
        if r.mark_read && !read {
            outbox::set_read(&id, true)?;
        }
        if r.flag && !flagged {
            outbox::set_flag(&id, true)?;
        }
        if let Some(dest) = &r.move_to {
            if dest != &current {
                outbox::move_to(&id, dest)?;
            }
        }
    }
    Ok(matched)
}

/// IMAP accounts: runs the account's switched-on rules on newly arrived messages. Returns how many changed.
pub fn apply_local(account_id: &str, message_ids: &[String]) -> Result<usize> {
    let rules: Vec<String> = storage::with_db(|db| {
        let mut st = db.prepare("SELECT id FROM rules WHERE account_id=?1 AND enabled=1 ORDER BY created_at")?;
        let rows = st.query_map([account_id], |r| r.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })?;
    let mut n = 0;
    for rid in rules {
        let row = load(&rid)?;
        // Read the messages again for each rule: an earlier rule may have moved or deleted some.
        let candidates: Vec<Candidate> = storage::with_db(|db| {
            let mut st = db.prepare(&format!("{CANDIDATES} WHERE m.id=?1 AND m.account_id=?2"))?;
            let mut out = vec![];
            for id in message_ids {
                out.extend(st.query_map(params![id, account_id], candidate)?.filter_map(|x| x.ok()));
            }
            Ok(out)
        })?;
        n += apply(&row.rule, account_id, candidates)?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::tests::fake_google;
    use std::sync::{Arc, Mutex};

    fn rule(account: &str) -> Rule {
        Rule { id: None, account: account.into(), name: String::new(), enabled: true, from: "boss@x.com".into(), subject: "invoice".into(), mark_read: true, flag: true, ..Default::default() }
    }

    #[test]
    fn builds_outlook_rules() {
        let v = outlook_rule(&Rule { move_to: Some("x".into()), ..rule("a") }, Some("FOLDERID"), 3);
        assert_eq!(v["conditions"]["fromAddresses"][0]["emailAddress"]["address"], "boss@x.com");
        assert_eq!(v["conditions"]["subjectContains"][0], "invoice");
        assert_eq!(v["actions"]["moveToFolder"], "FOLDERID");
        assert_eq!(v["actions"]["markAsRead"], true);
        assert_eq!(v["actions"]["markImportance"], "high");
        assert_eq!(v["sequence"], 3);
        // a plain name rather than an address becomes a "sender contains" test
        let v = outlook_rule(&Rule { from: "Acme Corp".into(), subject: String::new(), body: "refund".into(), to: "me".into(), delete: true, ..rule("a") }, None, 1);
        assert_eq!(v["conditions"]["senderContains"][0], "Acme Corp");
        assert_eq!(v["conditions"]["bodyContains"][0], "refund");
        assert_eq!(v["conditions"]["recipientContains"][0], "me");
        assert_eq!(v["actions"]["delete"], true);
    }

    #[test]
    fn builds_gmail_filters() {
        let f = gmail_filter(&rule("a"), Some("Label_7"));
        assert_eq!(f["criteria"]["from"], "boss@x.com");
        assert_eq!(f["criteria"]["subject"], "invoice");
        assert_eq!(f["action"]["addLabelIds"], json!(["Label_7", "STARRED"]));
        assert_eq!(f["action"]["removeLabelIds"], json!(["INBOX", "UNREAD"]));
        let f = gmail_filter(&Rule { body: "re\"fund".into(), mark_read: false, flag: false, delete: true, ..rule("a") }, Some("ALL_MAIL"));
        assert_eq!(f["criteria"]["query"], "\"refund\"");
        assert_eq!(f["action"]["addLabelIds"], json!(["TRASH"]));
        let archive = gmail_filter(&Rule { mark_read: false, flag: false, ..rule("a") }, Some("ALL_MAIL"));
        assert_eq!(archive["action"]["removeLabelIds"], json!(["INBOX"]));
    }

    #[test]
    fn checks_rules_before_sending_them() {
        assert!(clean(Rule { from: String::new(), subject: String::new(), ..rule("a") }).unwrap_err().to_string().contains("condition"));
        assert!(clean(Rule { mark_read: false, flag: false, ..rule("a") }).unwrap_err().to_string().contains("what the rule should do"));
        assert!(clean(Rule { delete: true, ..rule("a") }).unwrap_err().to_string().contains("can't be combined"));
        assert_eq!(clean(rule("a")).unwrap().name, "Rule: boss@x.com");
    }

    #[tokio::test]
    async fn rules_on_both_providers_and_run_now() {
        let _guard = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("gm-rules-{}", uuid::Uuid::new_v4()));
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg"));
            std::env::set_var("GM_TEST_TOKEN", "t");
        }
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO accounts(id,email,display_name,created_at,provider) VALUES('m','me@x.com','Me',1,'microsoft'),('g','me@gmail.com','Gm',2,'google');
                 INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES
                   ('mi','m','MS-INBOX','Inbox','inbox'),('mt','m','MS-TRASH','Deleted','deleteditems'),('mf','m','MS-BILLS','Bills',NULL),
                   ('gi','g','INBOX','Inbox','inbox'),('gt','g','TRASH','Trash','deleteditems'),('gl','g','Label_9','Bills',NULL);
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_name,sender_address,to_recipients_json,received_at,is_read) VALUES
                   ('m1','m','mi','r1','April invoice','Boss','boss@x.com','[]',3,0),
                   ('m2','m','mi','r2','Lunch?','Pal','pal@x.com','[]',2,0),
                   ('m3','m','mi','r3','Invoice again','Boss','boss@x.com','[]',1,1);
                 INSERT INTO messages_fts(message_id,subject,sender_name,sender_address,body_plaintext) VALUES('m3','Invoice again','Boss','boss@x.com','please pay the refund');",
            )?;
            Ok(())
        })
        .unwrap();
        let seen: Arc<Mutex<Vec<(String, String, Value)>>> = Default::default();
        let (base, log) = fake_google(|m, p, b| match (m, p.split('?').next().unwrap()) {
            ("POST", "/me/mailFolders/inbox/messageRules") => (201, json!({ "id": "OUT-1", "echo": b["displayName"] })),
            ("PATCH", "/me/mailFolders/inbox/messageRules/OUT-1") => (200, json!({})),
            ("DELETE", "/me/mailFolders/inbox/messageRules/OUT-1") => (204, Value::Null),
            ("POST", "/gmail/v1/users/me/settings/filters") => (200, json!({ "id": "FLT-1" })),
            ("DELETE", "/gmail/v1/users/me/settings/filters/FLT-1") => (204, Value::Null),
            ("DELETE", "/gmail/v1/users/me/settings/filters/FLT-0") => (404, json!({ "error": { "message": "gone" } })),
            ("POST", "/me/mailFolders/inbox/messageRules/forbidden") => (403, json!({})),
            _ => (404, json!({ "error": { "message": "no route" } })),
        })
        .await;
        unsafe {
            std::env::set_var("GM_TEST_GRAPH", &base);
            std::env::set_var("GM_TEST_GOOGLE", &base);
        }
        let _ = &seen;

        // Outlook: create, edit, switch off, delete
        let id = create(Rule { move_to: Some("mf".into()), ..rule("me@x.com") }).await.unwrap();
        let sent = log.lock().unwrap().last().unwrap().clone();
        assert_eq!(sent.2["actions"]["moveToFolder"], "MS-BILLS");
        let listed = list().unwrap();
        assert_eq!((listed.len(), listed[0].folder_name.as_deref(), listed[0].provider.as_str()), (1, Some("Bills"), "microsoft"));
        update(Rule { id: Some(id.clone()), name: "Bills".into(), subject: "bill".into(), move_to: Some("mf".into()), ..rule("me@x.com") }).await.unwrap();
        assert_eq!(log.lock().unwrap().last().unwrap().0, "PATCH");
        set_enabled(&id, false).await.unwrap();
        assert_eq!(log.lock().unwrap().last().unwrap().2["isEnabled"], false);
        // a folder from another account is refused
        assert!(create(Rule { move_to: Some("gl".into()), ..rule("me@x.com") }).await.unwrap_err().to_string().contains("different account"));

        // Gmail: create, edit = replace, switch off removes the filter, delete
        let gid = create(Rule { account: "me@gmail.com".into(), move_to: Some("gl".into()), ..rule("me@gmail.com") }).await.unwrap();
        assert_eq!(log.lock().unwrap().last().unwrap().2["action"]["addLabelIds"], json!(["Label_9", "STARRED"]));
        log.lock().unwrap().clear();
        set_enabled(&gid, false).await.unwrap();
        let calls: Vec<String> = log.lock().unwrap().iter().map(|(m, _, _)| m.clone()).collect();
        assert_eq!(calls, vec!["DELETE"], "a disabled Gmail rule isn't on the server");
        let remote: Option<String> = storage::with_db(|db| Ok(db.query_row("SELECT remote_id FROM rules WHERE id=?1", [&gid], |r| r.get(0))?)).unwrap();
        assert_eq!(remote, None);
        set_enabled(&gid, true).await.unwrap();
        assert_eq!(log.lock().unwrap().last().unwrap().0, "POST");

        // Run on existing mail (Outlook inbox): matches m1 (and m3, which is already read) by sender+subject
        let rid = list().unwrap().into_iter().find(|r| r.provider == "microsoft").unwrap().rule.id.unwrap();
        storage::with_db(|db| Ok(db.execute("UPDATE rules SET subject_text='invoice', enabled=1 WHERE id=?1", [&rid]).map(|_| ())?)).unwrap();
        let n = run_now(&rid, "mi").unwrap();
        assert_eq!(n, 2);
        let state: Vec<(String, bool, bool, String)> = storage::with_db(|db| {
            let mut st = db.prepare("SELECT id, is_read, is_flagged, folder_id FROM messages WHERE account_id='m' ORDER BY id")?;
            Ok(st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.filter_map(|r| r.ok()).collect())
        })
        .unwrap();
        assert_eq!(state[0], ("m1".into(), true, true, "mf".into()));
        assert_eq!(state[1], ("m2".into(), false, false, "mi".into()), "non-matching mail untouched");
        assert_eq!(state[2].3, "mf");
        assert_eq!(outbox::pending_count().unwrap() > 0, true, "the changes are queued for the server");
        // body words
        storage::with_db(|db| Ok(db.execute("UPDATE rules SET from_text='', subject_text='', body_text='REFUND' WHERE id=?1", [&rid]).map(|_| ())?)).unwrap();
        assert_eq!(run_now(&rid, "mf").unwrap(), 1);
        assert!(run_now(&rid, "gi").unwrap_err().to_string().contains("same account"));

        // Block sender: a rule from that address into Junk (Outlook) / Spam (Gmail)
        storage::with_db(|db| {
            db.execute_batch(
                "INSERT INTO folders(id,account_id,remote_id,display_name,well_known_name) VALUES('mj','m','MS-JUNK','Junk Email','junkemail'),('gs','g','SPAM','Spam','junkemail');
                 INSERT INTO messages(id,account_id,folder_id,remote_id,subject,sender_address,received_at) VALUES('g1','g','gi','x1','Win','spam@bad.biz',4);",
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(block_sender("m2").await.unwrap(), "pal@x.com");
        let sent = log.lock().unwrap().last().unwrap().clone();
        assert_eq!(sent.2["conditions"]["fromAddresses"][0]["emailAddress"]["address"], "pal@x.com");
        assert_eq!(sent.2["actions"]["moveToFolder"], "MS-JUNK");
        assert_eq!(sent.2["displayName"], "Block pal@x.com");
        block_sender("g1").await.unwrap();
        let sent = log.lock().unwrap().last().unwrap().clone();
        assert_eq!((sent.2["criteria"]["from"].clone(), sent.2["action"]["addLabelIds"].clone(), sent.2["action"]["removeLabelIds"].clone()), (json!("spam@bad.biz"), json!(["SPAM"]), json!(["INBOX"])));
        for r in list().unwrap().into_iter().filter(|r| r.rule.name.starts_with("Block")) {
            storage::with_db(|db| Ok(db.execute("DELETE FROM rules WHERE id=?1", [r.rule.id.unwrap()]).map(|_| ())?)).unwrap();
        }

        // delete removes it on the server and here
        delete(&id).await.unwrap();
        assert_eq!(log.lock().unwrap().last().unwrap().0, "DELETE");
        delete(&gid).await.unwrap();
        assert!(list().unwrap().is_empty());
    }
}
