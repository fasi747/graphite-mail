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
const KEYRING_SERVICE: &str = "graphite-mail";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Account {
    pub email: String,
    pub display_name: String,
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

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn entry(key: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, key)?)
}

/// Runs the interactive sign-in. `open` is called with the URL to show in the system browser.
pub async fn sign_in(open: impl FnOnce(&str) -> Result<()>) -> Result<Account> {
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
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
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
        ("scope", SCOPES),
    ])
    .await?;

    let account = fetch_me(&tok.access_token).await?;
    entry("refresh_token")?.set_password(
        tok.refresh_token
            .as_deref()
            .ok_or(anyhow!("No refresh token"))?,
    )?;
    entry("account")?.set_password(&serde_json::to_string(&account)?)?;
    Ok(account)
}

async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String> {
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
                v["error_description"].as_str().and_then(|d| d.lines().next()).unwrap_or("unknown error")
            ),
        });
    }
    let tok: TokenResponse = resp.json().await?;
    *CACHE.lock().unwrap() = Some(Cached {
        token: tok.access_token.clone(),
        expires: Instant::now() + Duration::from_secs(tok.expires_in.saturating_sub(120)),
    });
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
    })
}

/// Returns a valid access token, refreshing silently via the keyring-stored refresh token.
pub async fn access_token() -> Result<String> {
    #[cfg(test)]
    if let Ok(t) = std::env::var("GM_TEST_TOKEN") {
        return Ok(t);
    }
    if let Some(c) = CACHE.lock().unwrap().as_ref() {
        if Instant::now() < c.expires {
            return Ok(c.token.clone());
        }
    }
    let rt = entry("refresh_token")?
        .get_password()
        .context("Not signed in")?;
    let tok = token_request(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", &rt),
        ("scope", SCOPES),
    ])
    .await?;
    if let Some(new_rt) = tok.refresh_token {
        entry("refresh_token")?.set_password(&new_rt)?;
    }
    Ok(tok.access_token)
}

pub fn current_account() -> Option<Account> {
    entry("account")
        .ok()?
        .get_password()
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn sign_out() -> Result<()> {
    *CACHE.lock().unwrap() = None;
    for k in ["refresh_token", "account"] {
        let _ = entry(k)?.delete_credential();
    }
    Ok(())
}
