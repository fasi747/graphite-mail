//! Microsoft identity: OAuth2 Authorization Code + PKCE via loopback redirect.
//! Refresh tokens live only in the OS keyring (GNOME Keyring / KWallet via Secret Service).

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const CLIENT_ID: &str = "4fd86bd0-7f26-4941-87a6-e69aa7e982d0";
const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";
const SCOPES: &str =
    "openid profile offline_access User.Read Mail.ReadWrite Mail.Send Calendars.ReadWrite";
/// Extra permission needed to create Outlook mail rules; asked for only when the user turns rules on.
const RULES_SCOPE: &str = "MailboxSettings.ReadWrite";

fn scopes(rules: bool) -> String {
    if rules { format!("{SCOPES} {RULES_SCOPE}") } else { SCOPES.to_string() }
}
#[cfg(not(test))]
const KEYRING_SERVICE: &str = "graphite-mail";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Account {
    pub email: String,
    pub display_name: String,
    /// "microsoft", "google" (Gmail through Google sign-in) or "imap" (Gmail through IMAP/SMTP with an app password)
    #[serde(default = "default_provider")]
    pub provider: String,
    /// The account has granted the extra permission that mail rules need.
    #[serde(default)]
    pub rules: bool,
}

fn default_provider() -> String {
    "microsoft".into()
}

tokio::task_local! {
    /// The account whose mailbox the current task is working on.
    static ACTIVE: String;
}

/// Runs `fut` on behalf of `email`: every access_token() call inside uses that account.
pub async fn in_account<F: std::future::Future>(email: Option<String>, fut: F) -> F::Output {
    match email {
        Some(e) => ACTIVE.scope(e, fut).await,
        None => fut.await,
    }
}

/// Email of the account the current task works for (falls back to the first account).
pub fn active_email() -> Option<String> {
    ACTIVE.try_with(|e| e.clone()).ok().or_else(|| accounts().into_iter().next().map(|a| a.email))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

struct Cached {
    token: String,
    expires: Instant,
}

static CACHE: Mutex<Option<std::collections::HashMap<String, Cached>>> = Mutex::new(None);

fn cache_put(email: &str, token: &str, expires_in: u64) {
    CACHE.lock().unwrap().get_or_insert_with(Default::default).insert(
        email.to_lowercase(),
        Cached { token: token.to_string(), expires: Instant::now() + Duration::from_secs(expires_in.saturating_sub(120)) },
    );
}

fn cache_get(email: &str) -> Option<String> {
    let g = CACHE.lock().unwrap();
    let c = g.as_ref()?.get(&email.to_lowercase())?;
    (Instant::now() < c.expires).then(|| c.token.clone())
}

fn rt_key(email: &str) -> String {
    format!("rt:{}", email.to_lowercase())
}

/// All signed-in accounts, oldest first. Migrates the single-account format of earlier versions.
pub fn accounts() -> Vec<Account> {
    if let Some(list) = kr_get("accounts").ok().and_then(|s| serde_json::from_str::<Vec<Account>>(&s).ok()) {
        return list;
    }
    // Earlier versions kept one account under "account" + "refresh_token".
    let legacy: Option<Account> = kr_get("account").ok().and_then(|s| serde_json::from_str(&s).ok());
    let rt = kr_get("refresh_token").ok();
    if let (Some(acc), Some(rt)) = (legacy, rt) {
        if kr_set(&rt_key(&acc.email), &rt).is_ok() && save_accounts(&[acc.clone()]).is_ok() {
            kr_delete("account");
            kr_delete("refresh_token");
        }
        return vec![acc];
    }
    vec![]
}

fn save_accounts(list: &[Account]) -> Result<()> {
    kr_set("accounts", &serde_json::to_string(list)?)
}

/// Adds or updates an account after a successful sign-in.
pub fn remember(acc: &Account, refresh_token: &str) -> Result<()> {
    kr_set(&rt_key(&acc.email), refresh_token)?;
    remember_account(acc)
}

/// Adds or updates an account in the list, without touching its sign-in secret.
pub fn remember_account(acc: &Account) -> Result<()> {
    let mut list = accounts();
    match list.iter_mut().find(|a| a.email.eq_ignore_ascii_case(&acc.email)) {
        Some(a) => {
            let rules = a.rules || acc.rules; // a permission once granted stays granted
            *a = acc.clone();
            a.rules = rules;
        }
        None => list.push(acc.clone()),
    }
    save_accounts(&list)
}

pub fn refresh_token_for(email: &str) -> Result<String> {
    kr_get(&rt_key(email)).context("Not signed in")
}

pub fn store_refresh_token(email: &str, rt: &str) -> Result<()> {
    kr_set(&rt_key(email), rt)
}

fn app_key(email: &str) -> String {
    format!("app:{}", email.to_lowercase())
}

/// IMAP accounts: the Google app password. It lives only in the keyring, never in a file or the database.
pub fn store_app_password(email: &str, password: &str) -> Result<()> {
    kr_set(&app_key(email), password)
}

pub fn app_password(email: &str) -> Result<String> {
    kr_get(&app_key(email))
}

pub fn forget_app_password(email: &str) {
    kr_delete(&app_key(email));
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(not(test))]
fn entry(key: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, key)?)
}

#[cfg(not(test))]
fn kr_get(key: &str) -> Result<String> {
    Ok(entry(key)?.get_password()?)
}

#[cfg(not(test))]
fn kr_set(key: &str, value: &str) -> Result<()> {
    Ok(entry(key)?.set_password(value)?)
}

#[cfg(not(test))]
fn kr_delete(key: &str) {
    if let Ok(e) = entry(key) {
        let _ = e.delete_credential();
    }
}

/// Tests use an in-memory keyring instead of the desktop's.
#[cfg(test)]
pub(crate) static TEST_KEYRING: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);

#[cfg(test)]
fn kr_get(key: &str) -> Result<String> {
    TEST_KEYRING.lock().unwrap().get_or_insert_with(Default::default).get(key).cloned().ok_or_else(|| anyhow!("No such keyring entry"))
}

#[cfg(test)]
fn kr_set(key: &str, value: &str) -> Result<()> {
    TEST_KEYRING.lock().unwrap().get_or_insert_with(Default::default).insert(key.to_string(), value.to_string());
    Ok(())
}

#[cfg(test)]
fn kr_delete(key: &str) {
    TEST_KEYRING.lock().unwrap().get_or_insert_with(Default::default).remove(key);
}

/// Runs the interactive sign-in. `open` is called with the URL to show in the system browser.
pub async fn sign_in(open: impl FnOnce(&str) -> Result<()>) -> Result<Account> {
    sign_in_with(open, false, None).await
}

/// Sign-in that can also ask for the mail-rules permission, and pre-select an account with `hint`.
pub async fn sign_in_with(open: impl FnOnce(&str) -> Result<()>, rules: bool, hint: Option<&str>) -> Result<Account> {
    let scope = scopes(rules);
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = b64(&Sha256::digest(verifier.as_bytes()));
    let state = uuid::Uuid::new_v4().simple().to_string();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());

    let mut url = url::Url::parse(&format!("{AUTHORITY}/authorize"))?;
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_mode", "query")
        .append_pair("scope", &scope)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", if hint.is_some() { "consent" } else { "select_account" });
    if let Some(h) = hint {
        url.query_pairs_mut().append_pair("login_hint", h);
    }
    open(url.as_str())?;

    let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state))
        .await
        .map_err(|_| anyhow!("Sign-in timed out"))??;

    let tok = token_request(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("code_verifier", &verifier),
        ("scope", &scope),
    ])
    .await?;

    let mut account = fetch_me(&tok.access_token).await?;
    account.rules = rules;
    remember(&account, tok.refresh_token.as_deref().ok_or(anyhow!("No refresh token"))?)?;
    cache_put(&account.email, &tok.access_token, tok.expires_in);
    Ok(account)
}

pub(crate) async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let n = sock.read(&mut buf).await?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.split_whitespace().nth(1).unwrap_or("/");
        let u = url::Url::parse(&format!("http://localhost{path}"))?;
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        if q.is_empty() {
            continue; // favicon etc.
        }
        let (ok, body) = match (q.get("code"), q.get("error")) {
            (Some(c), _) if q.get("state").map(|s| s == state).unwrap_or(false) => (
                Some(c.clone()),
                "Signed in. You can close this tab and return to Graphite Mail.",
            ),
            (_, Some(_)) => (
                None,
                "Sign-in was cancelled or failed. Return to Graphite Mail.",
            ),
            _ => (None, "Invalid response."),
        };
        let html = format!(
            "<html><body style='font-family:sans-serif;padding:3em'><h2>Graphite Mail</h2><p>{body}</p></body></html>"
        );
        let _ = sock
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes())
            .await;
        return ok.ok_or_else(|| {
            anyhow!(
                q.get("error_description")
                    .cloned()
                    .unwrap_or_else(|| body.to_string())
            )
        });
    }
}

async fn token_request(form: &[(&str, &str)]) -> Result<TokenResponse> {
    let resp = crate::http::client()
        .post(format!("{AUTHORITY}/token"))
        .form(form)
        .send()
        .await?;
    if !resp.status().is_success() {
        let text = resp.text().await.unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        return Err(match v["error"].as_str() {
            Some("invalid_grant" | "interaction_required" | "consent_required") => {
                anyhow!("Your Microsoft sign-in has expired. Please sign out and sign in again.")
            }
            _ => anyhow!(
                "Microsoft sign-in failed: {}",
                v["error_description"]
                    .as_str()
                    .and_then(|d| d.lines().next())
                    .unwrap_or("unknown error")
            ),
        });
    }
    let tok: TokenResponse = resp.json().await?;
    Ok(tok)
}

async fn fetch_me(token: &str) -> Result<Account> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Me {
        display_name: Option<String>,
        mail: Option<String>,
        user_principal_name: String,
    }
    let me: Me = crate::http::client()
        .get("https://graph.microsoft.com/v1.0/me")
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(Account {
        email: me.mail.unwrap_or(me.user_principal_name.clone()),
        display_name: me.display_name.unwrap_or(me.user_principal_name),
        provider: "microsoft".into(),
        rules: false,
    })
}

/// Returns a valid access token for the active account, refreshing silently with its stored refresh token.
pub async fn access_token() -> Result<String> {
    #[cfg(test)]
    if let Ok(t) = std::env::var("GM_TEST_TOKEN") {
        return Ok(t);
    }
    let email = active_email().ok_or(anyhow!("Not signed in"))?;
    if let Some(t) = cache_get(&email) {
        return Ok(t);
    }
    let acc = accounts().into_iter().find(|a| a.email.eq_ignore_ascii_case(&email));
    match acc.as_ref().map(|a| a.provider.as_str()) {
        Some("google") => return crate::google::access_token(&email).await,
        // IMAP accounts sign in to the mail server with their app password; they have no OAuth token.
        Some(crate::imap::PROVIDER) => return Err(anyhow!("{email} uses an app password, not a sign-in token")),
        _ => {}
    }
    let rt = refresh_token_for(&email)?;
    let scope = scopes(acc.as_ref().is_some_and(|a| a.rules));
    let tok = token_request(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", &rt),
        ("scope", &scope),
    ])
    .await
    .map_err(|e| anyhow!("{email}: {e}"))?;
    if let Some(new_rt) = tok.refresh_token {
        store_refresh_token(&email, &new_rt)?;
    }
    cache_put(&email, &tok.access_token, tok.expires_in);
    Ok(tok.access_token)
}

/// Cache a token obtained elsewhere (Google sign-in/refresh).
pub fn cache_token(email: &str, token: &str, expires_in: u64) {
    cache_put(email, token, expires_in);
}

pub fn current_account() -> Option<Account> {
    accounts().into_iter().next()
}

/// Signs out one account (or all when `email` is None) and forgets its keyring entries.
pub fn sign_out_account(email: Option<&str>) -> Result<()> {
    let list = accounts();
    let (gone, keep): (Vec<_>, Vec<_>) = list.into_iter().partition(|a| email.map_or(true, |e| a.email.eq_ignore_ascii_case(e)));
    for a in &gone {
        kr_delete(&rt_key(&a.email));
        kr_delete(&app_key(&a.email));
        if let Some(c) = CACHE.lock().unwrap().as_mut() {
            c.remove(&a.email.to_lowercase());
        }
    }
    if keep.is_empty() {
        kr_delete("accounts");
    } else {
        save_accounts(&keep)?;
    }
    Ok(())
}

pub fn sign_out() -> Result<()> {
    sign_out_account(None)
}
