//! Folder management on the server (needs to be online): create, rename, delete.
//! The local folder table is updated from the server's reply so the sidebar changes at once.

use crate::{auth, storage};
use anyhow::{anyhow, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

async fn call(method: reqwest::Method, url: &str, body: Option<Value>) -> Result<Value> {
    let token = auth::access_token().await.map_err(|_| anyhow!("You need to be signed in and online to change folders."))?;
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .request(method, url)
        .bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|_| anyhow!("You're offline. Folder changes need an internet connection."))?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("Microsoft refused the change");
        return Err(anyhow!("{msg}"));
    }
    Ok(v)
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(anyhow!("Enter a folder name."));
    }
    if n.len() > 255 {
        return Err(anyhow!("That name is too long."));
    }
    Ok(n.to_string())
}

/// Look up a local folder: (remote id, well-known name).
fn lookup(id: &str) -> Result<(String, Option<String>)> {
    storage::with_db(|db| {
        Ok(db
            .query_row("SELECT remote_id, well_known_name FROM folders WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?
            .ok_or(anyhow!("That folder no longer exists."))?)
    })
}

/// Creates a folder at the top level, or inside `parent_id` when given. Returns the new local id.
pub async fn create(name: &str, parent_id: Option<&str>) -> Result<String> {
    let name = clean_name(name)?;
    let url = match parent_id {
        Some(p) => format!("{GRAPH}/me/mailFolders/{}/childFolders", lookup(p)?.0),
        None => format!("{GRAPH}/me/mailFolders"),
    };
    let v = call(reqwest::Method::POST, &url, Some(json!({ "displayName": name }))).await?;
    let rid = v["id"].as_str().ok_or(anyhow!("Unexpected reply from Microsoft"))?.to_string();
    let id = uuid::Uuid::new_v4().to_string();
    storage::with_db(|db| {
        db.execute(
            "INSERT INTO folders(id,account_id,remote_id,parent_remote_id,display_name,unread_count,total_count)
             VALUES(?1,(SELECT id FROM accounts ORDER BY created_at LIMIT 1),?2,?3,?4,0,0)
             ON CONFLICT(account_id,remote_id) DO NOTHING",
            params![id, rid, v["parentFolderId"].as_str(), name],
        )?;
        Ok(())
    })?;
    Ok(id)
}

pub async fn rename(id: &str, name: &str) -> Result<()> {
    let name = clean_name(name)?;
    let (rid, wk) = lookup(id)?;
    if wk.is_some() {
        return Err(anyhow!("Built-in folders like Inbox can't be renamed."));
    }
    call(reqwest::Method::PATCH, &format!("{GRAPH}/me/mailFolders/{rid}"), Some(json!({ "displayName": name }))).await?;
    storage::with_db(|db| Ok(db.execute("UPDATE folders SET display_name=?1 WHERE id=?2", params![name, id]).map(|_| ())?))
}

/// Deletes on the server (Microsoft moves it to Deleted Items) and removes it and its subfolders locally.
pub async fn delete(id: &str) -> Result<()> {
    let (rid, wk) = lookup(id)?;
    if wk.is_some() {
        return Err(anyhow!("Built-in folders like Inbox can't be deleted."));
    }
    call(reqwest::Method::DELETE, &format!("{GRAPH}/me/mailFolders/{rid}"), None).await?;
    storage::with_db(|db| {
        // Remove the folder and every folder below it; their messages go via ON DELETE CASCADE.
        let mut doomed = vec![rid.clone()];
        let mut i = 0;
        while i < doomed.len() {
            let mut st = db.prepare("SELECT remote_id FROM folders WHERE parent_remote_id=?1")?;
            let kids: Vec<String> = st.query_map([&doomed[i]], |r| r.get(0))?.filter_map(|r| r.ok()).collect();
            doomed.extend(kids);
            i += 1;
        }
        for r in &doomed {
            db.execute(
                "DELETE FROM messages_fts WHERE message_id IN (SELECT m.id FROM messages m JOIN folders f ON f.id=m.folder_id WHERE f.remote_id=?1)",
                [r],
            )?;
            db.execute("DELETE FROM folders WHERE remote_id=?1", [r])?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn validates_names() {
        assert!(super::clean_name("   ").is_err());
        assert_eq!(super::clean_name("  Projects ").unwrap(), "Projects");
        assert!(super::clean_name(&"x".repeat(300)).is_err());
    }
}
