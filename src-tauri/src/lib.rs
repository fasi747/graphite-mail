mod storage;
mod auth;
mod mail;
mod outbox;
mod sync;

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

#[tauri::command]
async fn sync_now() -> CmdResult<sync::SyncReport> {
    sync::sync_all().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn list_folders() -> CmdResult<Vec<mail::Folder>> {
    mail::folders().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_messages(folder_id: String, offset: i64, limit: i64) -> CmdResult<Vec<mail::MessageRow>> {
    mail::messages(&folder_id, offset, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_message(id: String) -> CmdResult<mail::MessageDetail> {
    mail::message(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn search_messages(query: String) -> CmdResult<Vec<mail::MessageRow>> {
    mail::search(&query, 200).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_read(id: String, read: bool) -> CmdResult<()> {
    outbox::set_read(&id, read).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_flag(id: String, flagged: bool) -> CmdResult<()> {
    outbox::set_flag(&id, flagged).map_err(|e| e.to_string())
}

#[tauri::command]
fn move_message(id: String, folder_id: String) -> CmdResult<()> {
    outbox::move_to(&id, &folder_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_message(id: String) -> CmdResult<()> {
    outbox::delete(&id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_mail(draft: outbox::Draft) -> CmdResult<bool> {
    outbox::send(draft).map_err(|e| e.to_string())?;
    // Try to send right away; if offline it stays queued for the next sync.
    Ok(outbox::replay().await.is_ok())
}

#[tauri::command]
fn pending_actions() -> CmdResult<i64> {
    outbox::pending_count().map_err(|e| e.to_string())
}

/// Background loop: sync every 2 minutes while signed in; notify the UI after each pass.
fn spawn_sync_loop(app: tauri::AppHandle) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if auth::current_account().is_some() && storage::read_bootstrap().is_some() {
                let res = sync::sync_all().await.map_err(|e| e.to_string());
                let _ = app.emit("sync-finished", res.is_ok());
            }
        }
    });
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
            initialize_storage,
            sync_now,
            list_folders,
            list_messages,
            get_message,
            search_messages,
            set_read,
            set_flag,
            move_message,
            delete_message,
            send_mail,
            pending_actions
        ])
        .setup(|app| {
            spawn_sync_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
