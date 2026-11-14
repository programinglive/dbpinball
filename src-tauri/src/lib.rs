mod connections;
mod models;
mod runners;
mod secrets;
mod security;

use dbpinball_core::store::{ImportReport, MigrationStatus};
use models::*;
use runners::{
    list_databases, list_tables, run_query, start_backup, start_copy, start_restore,
    test_connection,
};

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
async fn list_tables_cmd(conn: DbConnection, database: String) -> Result<Vec<String>, String> {
    list_tables(conn, database).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_query_cmd(
    conn: DbConnection,
    database: String,
    sql: String,
    limit: Option<i64>,
) -> Result<QueryResult, String> {
    run_query(conn, database, sql, limit)
        .await
        .map_err(|e| e.to_string())
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

#[tauri::command]
fn connections_list() -> Result<Vec<DbConnection>, String> {
    connections::list()
}

#[tauri::command]
fn connections_create(
    connection: DbConnection,
    password: Option<String>,
) -> Result<DbConnection, String> {
    connections::create(connection, password)
}

#[tauri::command]
fn connections_update(
    connection: DbConnection,
    password: Option<String>,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    connections::update(connection, password, expected_revision)
}

#[tauri::command]
fn connections_delete(id: String, expected_revision: Option<u64>) -> Result<bool, String> {
    connections::delete(id, expected_revision)
}

#[tauri::command]
fn connections_import(records: Vec<DbConnection>) -> Result<ImportReport, String> {
    connections::import(records)
}

#[tauri::command]
fn connections_migrate_local_storage(records: Vec<DbConnection>) -> Result<ImportReport, String> {
    connections::migrate_local_storage(records)
}

#[tauri::command]
fn connections_migration_status() -> Result<MigrationStatus, String> {
    connections::migration_status()
}

#[tauri::command]
async fn security_status(
    connection: DbConnection,
) -> Result<dbpinball_core::security::SecurityStatus, String> {
    security::status(connection).await
}

#[tauri::command]
async fn security_enroll(
    connection: DbConnection,
    category: String,
) -> Result<dbpinball_core::security::EnrolledFingerprint, String> {
    security::enroll(connection, category).await
}

#[tauri::command]
fn security_forget(connection_id: String) -> Result<bool, String> {
    security::forget(connection_id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            test_conn,
            list_databases_cmd,
            list_tables_cmd,
            run_query_cmd,
            backup,
            restore,
            copy_db,
            store_password,
            get_password,
            delete_password,
            connections_list,
            connections_create,
            connections_update,
            connections_delete,
            connections_import,
            connections_migrate_local_storage,
            connections_migration_status,
            security_status,
            security_enroll,
            security_forget
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
