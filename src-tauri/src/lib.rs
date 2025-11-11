mod models;
mod runners;
mod secrets;

use models::*;
use runners::{list_databases, start_backup, start_copy, start_restore, test_connection};

#[tauri::command]
async fn test_conn(conn: DbConnection) -> Result<String, String> {
    test_connection(conn)
        .await
        .map(|_| "Connection successful".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_databases_cmd(conn: DbConnection) -> Result<Vec<String>, String> {
    list_databases(conn).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn backup(
    window: tauri::Window,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String, String> {
    start_backup(&window, conn, opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn restore(
    window: tauri::Window,
    conn: DbConnection,
    file: String,
    opts: RestoreOptions,
) -> Result<String, String> {
    start_restore(&window, conn, file, opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn copy_db(
    window: tauri::Window,
    src: DbConnection,
    dst: DbConnection,
    opts: CopyOptions,
) -> Result<String, String> {
    start_copy(&window, src, dst, opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn store_password(id: String, password: String) -> Result<(), String> {
    secrets::store_password(&id, &password).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_password(id: String) -> Result<String, String> {
    secrets::get_password(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_password(id: String) -> Result<(), String> {
    secrets::delete_password(&id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            test_conn,
            list_databases_cmd,
            backup,
            restore,
            copy_db,
            store_password,
            get_password,
            delete_password
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
