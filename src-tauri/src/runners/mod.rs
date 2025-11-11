pub mod mysql;
pub mod postgres;

use crate::models::*;
use anyhow::Result;
use tauri::Window;

pub async fn test_connection(conn: DbConnection) -> Result<()> {
    match conn.engine {
        DbEngine::Mysql => mysql::test(conn).await,
        DbEngine::Postgres => postgres::test(conn).await,
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
    if std::mem::discriminant(&src.engine) == std::mem::discriminant(&dst.engine) {
        match src.engine {
            DbEngine::Mysql => mysql::copy_same_engine(window, src, dst, opts).await,
            DbEngine::Postgres => postgres::copy_same_engine(window, src, dst, opts).await,
        }
    } else {
        anyhow::bail!("Cross-engine copy is not implemented yet")
    }
}
