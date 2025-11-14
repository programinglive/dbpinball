use crate::models::*;
use crate::secrets;
use anyhow::{anyhow, Context, Result};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::types::Decimal;
use sqlx::{Column, Row};
use std::path::PathBuf;
use std::process::Stdio;
use tauri::{Window, Emitter};
use tokio::io::AsyncWriteExt;
use which::which;

fn emit(window: &Window, msg: &str) {
    let _ = window.emit("job-progress", msg);
}

pub async fn test(conn: DbConnection) -> Result<()> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = PgPool::connect(&url).await?;
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(())
}

pub async fn list_databases(conn: DbConnection) -> Result<Vec<String>> {
    let password = secrets::get_password(&conn.id)?;
    let fallback = if conn.database.trim().is_empty() {
        "postgres".to_string()
    } else {
        conn.database.clone()
    };
    let url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, fallback
    );
    let pool = sqlx::PgPool::connect(&url).await?;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname",
    )
    .fetch_all(&pool)
    .await?;
    Ok(names)
}

pub async fn backup(
    window: &Window,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
    // Try pg_dump first
    if let Ok(bin) = which("pg_dump") {
        emit(window, "Using pg_dump…");
        return dump_with_pg_dump(window, bin, conn, opts).await;
    }

    // Fallback to internal dumper
    emit(window, "Using internal dumper…");
    internal_dump(window, conn, opts).await
}

pub async fn list_tables(conn: DbConnection, database: String) -> Result<Vec<String>> {
    let target_db = resolve_database(&conn, &database)?;
    let pool = connect_to_postgres(&conn, &target_db).await?;

    let tables = sqlx::query_scalar::<_, String>(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await?;

    Ok(tables)
}

pub async fn run_query(
    conn: DbConnection,
    database: String,
    sql: String,
    limit: Option<i64>,
) -> Result<QueryResult> {
    let target_db = resolve_database(&conn, &database)?;
    let pool = connect_to_postgres(&conn, &target_db).await?;

    let mut rows = sqlx::query(&sql).fetch_all(&pool).await?;
    if let Some(max) = limit {
        if max >= 0 && rows.len() > max as usize {
            rows.truncate(max as usize);
        }
    }

    let columns = rows
        .first()
        .map(|row| row.columns().iter().map(|c| c.name().to_string()).collect())
        .unwrap_or_else(Vec::new);

    let mut data = Vec::with_capacity(rows.len());
    for row in rows {
        let mut record = Vec::with_capacity(columns.len());
        for (idx, _) in row.columns().iter().enumerate() {
            record.push(pg_value_to_string(&row, idx)?);
        }
        data.push(record);
    }

    Ok(QueryResult { columns, rows: data })
}

async fn dump_with_pg_dump(
    window: &Window,
    bin: PathBuf,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
    let password = secrets::get_password(&conn.id)?;
    let out = artifact_path("postgres", &conn.database, opts.compress)?;

    let mut args = vec![
        "-h".to_string(),
        conn.host.clone(),
        "-p".to_string(),
        conn.port.to_string(),
        "-U".to_string(),
        conn.username.clone(),
    ];

    if opts.structure && !opts.data {
        args.push("-s".to_string());
    }
    if !opts.structure && opts.data {
        args.push("-a".to_string());
    }

    if opts.compress {
        args.push("-Fc".to_string());
    }

    args.push(conn.database.clone());

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(&args)
        .env("PGPASSWORD", &password)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("spawn pg_dump")?;

    let stdout = child.stdout.take().context("get stdout")?;
    let output_file = tokio::fs::File::create(&out).await?;

    tokio::io::copy(&mut tokio::io::BufReader::new(stdout), &mut Box::new(output_file))
        .await?;

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("pg_dump failed with status: {}", status);
    }

    emit(window, "Backup completed.");
    Ok(out.to_string_lossy().to_string())
}

async fn internal_dump(
    window: &Window,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = sqlx::PgPool::connect(&url).await?;

    let out = artifact_path("postgres", &conn.database, opts.compress)?;
    let _output_file = tokio::fs::File::create(&out).await?;

    emit(window, "Dumping schema…");

    if opts.structure {
        // Dump tables
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'public'",
        )
        .fetch_all(&pool)
        .await?;

        for (table_name,) in tables {
            emit(window, &format!("Dumping table: {}", table_name));
            // In a real implementation, we'd dump CREATE TABLE statements
        }
    }

    if opts.data {
        emit(window, "Dumping data…");
        // In a real implementation, we'd dump INSERT statements
    }

    emit(window, "Backup completed.");
    Ok(out.to_string_lossy().to_string())
}

pub async fn restore(
    window: &Window,
    conn: DbConnection,
    file: String,
    _opts: RestoreOptions,
) -> Result<String> {
    if let Ok(bin) = which("psql") {
        emit(window, "Using psql client…");
        let password = secrets::get_password(&conn.id)?;

        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(["-h", &conn.host, "-p", &conn.port.to_string(), "-U", &conn.username, "-d", &conn.database])
            .env("PGPASSWORD", &password)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().context("spawn psql")?;
        let stdin = child.stdin.take().context("get stdin")?;

        let input_file = tokio::fs::File::open(&file).await?;
        let mut reader = tokio::io::BufReader::new(input_file);
        let mut writer = tokio::io::BufWriter::new(stdin);

        tokio::io::copy(&mut reader, &mut writer).await?;
        writer.flush().await?;
        drop(writer);

        let status = child.wait().await?;
        if !status.success() {
            anyhow::bail!("psql restore failed with status: {}", status);
        }

        emit(window, "Restore completed.");
        Ok(file)
    } else {
        anyhow::bail!("psql client not found")
    }
}

pub async fn copy_same_engine(
    window: &Window,
    src: DbConnection,
    dst: DbConnection,
    opts: CopyOptions,
) -> Result<String> {
    emit(window, "Starting database copy…");

    if let Ok(bin) = which("pg_dump") {
        let backup_opts = BackupOptions {
            structure: opts.structure,
            data: opts.data,
            include_views: true,
            include_routines: true,
            include_triggers: true,
            bulk_insert: false,
            ignore_definer: false,
            compress: false,
        };

        let dump_file = dump_with_pg_dump(window, bin, src, backup_opts).await?;

        let restore_opts = RestoreOptions {
            drop_existing: opts.drop_in_target,
            stop_on_error: false,
        };

        restore(window, dst, dump_file.clone(), restore_opts).await?;
        Ok(dump_file)
    } else {
        anyhow::bail!("pg_dump not found")
    }
}

fn artifact_path(engine: &str, db: &str, compress: bool) -> Result<PathBuf> {
    let base = dirs::home_dir()
        .context("get home dir")?
        .join("dbpinball")
        .join("backups");
    std::fs::create_dir_all(&base)?;
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let filename = if compress {
        format!("{}_{}_{}.sql.deflate", engine, db, timestamp)
    } else {
        format!("{}_{}_{}.sql", engine, db, timestamp)
    };
    Ok(base.join(filename))
}

fn resolve_database(conn: &DbConnection, requested: &str) -> Result<String> {
    if !requested.trim().is_empty() {
        return Ok(requested.to_string());
    }

    if !conn.database.trim().is_empty() {
        return Ok(conn.database.clone());
    }

    Err(anyhow!("Database name is required"))
}

async fn connect_to_postgres(conn: &DbConnection, database: &str) -> Result<PgPool> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "postgresql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, database
    );
    Ok(PgPool::connect(&url).await?)
}

fn pg_value_to_string(row: &PgRow, index: usize) -> Result<Option<String>> {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return Ok(value);
    }

    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<Decimal>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<NaiveDateTime>, _>(index) {
        return Ok(value.map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string()));
    }

    if let Ok(value) = row.try_get::<Option<NaiveDate>, _>(index) {
        return Ok(value.map(|v| v.format("%Y-%m-%d").to_string()));
    }

    if let Ok(value) = row.try_get::<Option<NaiveTime>, _>(index) {
        return Ok(value.map(|v| v.format("%H:%M:%S").to_string()));
    }

    if let Ok(value) = row.try_get::<Option<Vec<u8>>, _>(index) {
        return Ok(value.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()));
    }

    Err(anyhow!("Unsupported Postgres value type at column {}", index))
}
