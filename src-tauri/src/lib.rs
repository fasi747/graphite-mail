mod storage;
mod auth;

use auth::Account;
use tauri_plugin_opener::OpenerExt;

type CmdResult<T> = Result<T, String>;

#[tauri::command]
async fn sign_in(app: tauri::AppHandle) -> CmdResult<Account> {
    auth::sign_in(|url| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn current_account() -> Option<Account> {
    auth::current_account()
}

#[tauri::command]
fn sign_out() -> CmdResult<()> {
    auth::sign_out().map_err(|e| e.to_string())
}

/// Confirms the stored session still works by silently refreshing a token.
#[tauri::command]
async fn check_session() -> CmdResult<bool> {
    Ok(auth::access_token().await.is_ok())
}

#[derive(serde::Serialize)]
struct SetupState {
    ready: bool,
    default_dir: String,
}

#[tauri::command]
fn setup_state() -> CmdResult<SetupState> {
    Ok(SetupState {
        ready: storage::open_existing().map_err(|e| e.to_string())?,
        default_dir: storage::default_data_dir(),
    })
}

#[tauri::command]
fn initialize_storage(path: String) -> CmdResult<()> {
    storage::initialize(&path).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            sign_in,
            current_account,
            sign_out,
            check_session,
            setup_state,
            initialize_storage
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
