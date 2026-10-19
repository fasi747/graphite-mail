//! Gmail + Google Calendar provider (filled in during the Gmail stage).

use anyhow::{Result, anyhow};

pub async fn sign_in(_open: impl FnOnce(&str) -> Result<()>) -> Result<crate::auth::Account> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
pub async fn access_token(_email: &str) -> Result<String> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
pub async fn sync_account(_account_id: &str) -> Result<crate::sync::SyncReport> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
pub async fn create_label(_name: &str, _parent: Option<&str>, _account_id: &str) -> Result<String> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
pub async fn rename_label(_id: &str, _name: &str) -> Result<()> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
pub async fn delete_label(_id: &str) -> Result<()> {
    Err(anyhow!("Gmail support is coming in the next version."))
}
