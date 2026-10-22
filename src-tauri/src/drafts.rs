//! Drafts (half-written messages) and per-account signatures. Both live on this computer.

use crate::{notify, storage};
use anyhow::{Result, anyhow};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DraftRow {
    pub id: Option<String>,
    pub mode: String,
    pub reply_to: Option<String>,
    /// Account email the message is written from.
    pub from: Option<String>,
    pub to: String,
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    pub subject: String,
    /// Everything in the editor, as HTML (signature and quoted text included).
    pub html: String,
    /// Paths of files to attach.
    pub attachments: Vec<String>,
    #[serde(default)]
    pub updated_at: i64,
}

pub fn save(d: DraftRow) -> Result<String> {
    let id = d.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now().timestamp();
    storage::with_db(|db| {
        db.execute(
            "INSERT INTO drafts(id,mode,reply_to,from_email,to_text,cc_text,bcc_text,subject,html,attachments_json,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(id) DO UPDATE SET mode=excluded.mode, reply_to=excluded.reply_to, from_email=excluded.from_email, to_text=excluded.to_text,
               cc_text=excluded.cc_text, bcc_text=excluded.bcc_text, subject=excluded.subject, html=excluded.html, attachments_json=excluded.attachments_json, updated_at=excluded.updated_at",
            params![id, d.mode, d.reply_to, d.from, d.to, d.cc, d.bcc, d.subject, d.html, serde_json::to_string(&d.attachments)?, now],
        )?;
        Ok(())
    })?;
    Ok(id)
}

pub fn list() -> Result<Vec<DraftRow>> {
    storage::with_db(|db| {
        let mut st = db.prepare("SELECT id,mode,reply_to,from_email,to_text,cc_text,bcc_text,subject,html,attachments_json,updated_at FROM drafts ORDER BY updated_at DESC")?;
        let rows = st.query_map([], |r| {
            Ok(DraftRow {
                id: r.get(0)?,
                mode: r.get(1)?,
                reply_to: r.get(2)?,
                from: r.get(3)?,
                to: r.get(4)?,
                cc: r.get(5)?,
                bcc: r.get(6)?,
                subject: r.get(7)?,
                html: r.get(8)?,
                attachments: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
                updated_at: r.get(10)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    })
}

pub fn delete(id: &str) -> Result<()> {
    storage::with_db(|db| Ok(db.execute("DELETE FROM drafts WHERE id=?1", [id]).map(|_| ())?))
}

fn sig_key(email: &str) -> String {
    format!("signature:{}", email.to_lowercase())
}

pub fn get_signature(email: &str) -> String {
    notify::get_setting(&sig_key(email)).unwrap_or_default()
}

pub fn set_signature(email: &str, text: &str) -> Result<()> {
    if text.len() > 5000 {
        return Err(anyhow!("A signature can be at most 5000 characters."));
    }
    notify::set_setting(&sig_key(email), text)
}

#[allow(dead_code)]
fn exists(id: &str) -> bool {
    storage::with_db(|db| Ok(db.query_row("SELECT 1 FROM drafts WHERE id=?1", [id], |_| Ok(())).optional()?.is_some())).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_and_signatures_are_kept() {
        let _g = storage::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("gm-draft-{}", uuid::Uuid::new_v4()));
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.join("cfg")) };
        storage::initialize(tmp.join("vault").to_str().unwrap()).unwrap();
        let id = save(DraftRow { mode: "new".into(), from: Some("a@x.com".into()), to: "b@y.com".into(), bcc: "secret@y.com".into(), subject: "Hello".into(), html: "<p>half written</p>".into(), attachments: vec!["/tmp/a.pdf".into()], ..Default::default() }).unwrap();
        assert!(exists(&id));
        assert_eq!(list().unwrap()[0].bcc, "secret@y.com", "blind copies are kept with the draft");
        // saving again updates the same draft
        save(DraftRow { id: Some(id.clone()), mode: "new".into(), to: "b@y.com, c@z.com".into(), subject: "Hello again".into(), html: "<p>more</p>".into(), ..Default::default() }).unwrap();
        let l = list().unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].subject.as_str(), l[0].to.as_str(), l[0].html.as_str()), ("Hello again", "b@y.com, c@z.com", "<p>more</p>"));
        let second = save(DraftRow { mode: "reply".into(), reply_to: Some("m1".into()), subject: "Re".into(), ..Default::default() }).unwrap();
        assert_eq!(list().unwrap().len(), 2);
        delete(&id).unwrap();
        assert_eq!(list().unwrap()[0].id.as_deref(), Some(second.as_str()));

        assert_eq!(get_signature("A@x.com"), "");
        set_signature("A@x.com", "Best regards,\nAnn").unwrap();
        assert_eq!(get_signature("a@X.com"), "Best regards,\nAnn", "case doesn't matter");
        assert!(set_signature("a@x.com", &"x".repeat(5001)).is_err());
    }
}
