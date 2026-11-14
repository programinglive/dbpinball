pub mod mysql;
pub mod postgres;

use crate::models::*;
use anyhow::Result;
use dbpinball_core::adapters::keyring::KeyringCredentialProvider;
use dbpinball_core::services::inspect;
use tauri::Window;

pub async fn test_connection(conn: DbConnection) -> Result<()> {
    inspect::test_connection(&conn, &KeyringCredentialProvider)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

pub async fn list_databases(conn: DbConnection) -> Result<Vec<String>> {
    inspect::list_databases(&conn, &KeyringCredentialProvider)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

pub async fn list_tables(conn: DbConnection, database: String) -> Result<Vec<String>> {
    inspect::list_tables(&conn, &database, &KeyringCredentialProvider)
        .await
        .map_err(|error| anyhow::anyhow!(error.message))
}

pub async fn run_query(
    conn: DbConnection,
    database: String,
    sql: String,
    limit: Option<i64>,
) -> Result<QueryResult> {
    // The query editor can run arbitrary SQL. Classify it first: a statement
    // that can write must pass the Core security policy before it reaches the
    // database. Read-only statements skip the extra round-trip.
    let class = dbpinball_core::security::classify_sql(&sql);
    if class != dbpinball_core::security::SqlClass::ReadOnly {
        crate::security::authorize_write(&conn, class.operation())
            .await
            .map_err(|message| anyhow::anyhow!(message))?;
    }

    // A statement classified read-only is executed in a database-level
    // read-only transaction. The classifier is a risk signal; the engine is the
    // enforcement boundary.
    let read_only = class == dbpinball_core::security::SqlClass::ReadOnly;

    match conn.engine {
        DbEngine::Mysql => mysql::run_query(conn, database, sql, limit, read_only).await,
        DbEngine::Postgres => postgres::run_query(conn, database, sql, limit, read_only).await,
    }
}

pub async fn start_backup(
    window: &Window,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
    match conn.engine {
        DbEngine::Mysql => mysql::backup(window, conn, opts).await,
        DbEngine::Postgres => postgres::backup(window, conn, opts).await,
    }
}

pub async fn start_restore(
    window: &Window,
    conn: DbConnection,
    file: String,
    opts: RestoreOptions,
) -> Result<String> {
    // The page may not be routed in the UI, but the command is registered and
    // callable. A restore writes, so it must pass the policy first.
    crate::security::authorize_write(
        &conn,
        dbpinball_core::security::DatabaseOperation::DatabaseRestore,
    )
    .await
    .map_err(|message| anyhow::anyhow!(message))?;

    match conn.engine {
        DbEngine::Mysql => mysql::restore(window, conn, file, opts).await,
        DbEngine::Postgres => postgres::restore(window, conn, file, opts).await,
    }
}

pub async fn start_copy(
    window: &Window,
    src: DbConnection,
    dst: DbConnection,
    opts: CopyOptions,
) -> Result<String> {
    // Copy reads one database and writes another. Verify both sides so a
    // reversed or identical pair is refused before the dump starts.
    crate::security::authorize_copy(&src, &dst)
        .await
        .map_err(|message| anyhow::anyhow!(message))?;

    if std::mem::discriminant(&src.engine) == std::mem::discriminant(&dst.engine) {
        match src.engine {
            DbEngine::Mysql => mysql::copy_same_engine(window, src, dst, opts).await,
            DbEngine::Postgres => postgres::copy_same_engine(window, src, dst, opts).await,
        }
    } else {
        anyhow::bail!("Cross-engine copy is not implemented yet")
    }
}
