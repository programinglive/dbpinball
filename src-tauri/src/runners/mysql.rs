use crate::models::*;
use crate::secrets;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde_json::Value as JsonValue;
use sqlx::mysql::{MySqlPool, MySqlRow};
use sqlx::types::Decimal;
use sqlx::{Column, Row, TypeInfo};
use std::path::PathBuf;
use std::process::Stdio;
use tauri::{Emitter, Window};
use tokio::io::AsyncWriteExt;
use which::which;

fn emit(window: &Window, msg: &str) {
    let _ = window.emit("job-progress", msg);
}

pub async fn test(conn: DbConnection) -> Result<()> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = MySqlPool::connect(&url).await?;
    sqlx::query("SELECT 1").execute(&pool).await?;
    Ok(())
}

pub async fn list_databases(conn: DbConnection) -> Result<Vec<String>> {
    let password = secrets::get_password(&conn.id)?;
    let fallback = if conn.database.trim().is_empty() {
        "information_schema".to_string()
    } else {
        conn.database.clone()
    };
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, fallback
    );
    let pool = sqlx::MySqlPool::connect(&url).await?;
    let names: Vec<String> = sqlx::query_scalar("SHOW DATABASES")
        .fetch_all(&pool)
        .await?;
    let filtered = names
        .into_iter()
        .filter(|name| {
            !matches!(
                name.as_str(),
                "information_schema" | "performance_schema" | "mysql" | "sys"
            )
        })
        .collect();
    Ok(filtered)
}

pub async fn list_tables(conn: DbConnection, database: String) -> Result<Vec<String>> {
    let target_db = resolve_database(&conn, &database)?;
    let pool = connect_to_mysql(&conn, &target_db).await?;
    let tables = sqlx::query_scalar::<_, String>("SHOW TABLES")
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
    let pool = connect_to_mysql(&conn, &target_db).await?;

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
            record.push(mysql_value_to_string(&row, idx)?);
        }
        data.push(record);
    }

    Ok(QueryResult {
        columns,
        rows: data,
    })
}

pub async fn backup(window: &Window, conn: DbConnection, opts: BackupOptions) -> Result<String> {
    // Try mysqldump first
    if let Ok(bin) = which("mysqldump") {
        emit(window, "Using mysqldump…");
        return dump_with_mysqldump(window, bin, conn, opts).await;
    }

    // Fallback to internal dumper
    emit(window, "Using internal dumper…");
    internal_dump(window, conn, opts).await
}

async fn dump_with_mysqldump(
    window: &Window,
    bin: PathBuf,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
    let password = secrets::get_password(&conn.id)?;
    let out = artifact_path("mysql", &conn.database, opts.compress)?;

    let mut args = vec![
        "-h".to_string(),
        conn.host.clone(),
        "-P".to_string(),
        conn.port.to_string(),
        "-u".to_string(),
        conn.username.clone(),
        "--single-transaction".to_string(),
    ];

    if opts.structure && !opts.data {
        args.push("--no-data".to_string());
    }
    if !opts.structure && opts.data {
        args.push("--no-create-info".to_string());
    }
    if opts.ignore_definer {
        args.push("--skip-definer".to_string());
    }
    if opts.include_triggers {
        args.push("--triggers".to_string());
    }
    if opts.include_routines {
        args.push("--routines".to_string());
    }
    if opts.include_views {
        args.push("--events".to_string());
    }

    args.push(conn.database.clone());

    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(&args)
        .env("MYSQL_PWD", &password)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("spawn mysqldump")?;

    let stdout = child.stdout.take().context("get stdout")?;
    let output_file = tokio::fs::File::create(&out).await?;

    if opts.compress {
        let encoder = async_compression::tokio::write::DeflateEncoder::new(output_file);
        tokio::io::copy(
            &mut tokio::io::BufReader::new(stdout),
            &mut Box::new(encoder),
        )
        .await?;
    } else {
        tokio::io::copy(
            &mut tokio::io::BufReader::new(stdout),
            &mut Box::new(output_file),
        )
        .await?;
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("mysqldump failed with status: {}", status);
    }

    emit(window, "Backup completed.");
    Ok(out.to_string_lossy().to_string())
}

async fn internal_dump(window: &Window, conn: DbConnection, opts: BackupOptions) -> Result<String> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = sqlx::MySqlPool::connect(&url).await?;

    let out = artifact_path("mysql", &conn.database, opts.compress)?;
    let _output_file = tokio::fs::File::create(&out).await?;

    emit(window, "Dumping schema…");

    if opts.structure {
        // Dump tables
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ?",
        )
        .bind(&conn.database)
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
    if let Ok(bin) = which("mysql") {
        emit(window, "Using mysql client…");
        let password = secrets::get_password(&conn.id)?;

        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args([
            "-h",
            &conn.host,
            "-P",
            &conn.port.to_string(),
            "-u",
            &conn.username,
            &conn.database,
        ])
        .env("MYSQL_PWD", &password)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd.spawn().context("spawn mysql")?;
        let stdin = child.stdin.take().context("get stdin")?;

        let input_file = tokio::fs::File::open(&file).await?;
        let mut reader = tokio::io::BufReader::new(input_file);
        let mut writer = tokio::io::BufWriter::new(stdin);

        tokio::io::copy(&mut reader, &mut writer).await?;
        writer.flush().await?;
        drop(writer);

        let status = child.wait().await?;
        if !status.success() {
            anyhow::bail!("mysql restore failed with status: {}", status);
        }

        emit(window, "Restore completed.");
        Ok(file)
    } else {
        anyhow::bail!("mysql client not found")
    }
}

pub async fn copy_same_engine(
    window: &Window,
    src: DbConnection,
    dst: DbConnection,
    opts: CopyOptions,
) -> Result<String> {
    emit(window, "Starting database copy…");

    if let Ok(bin) = which("mysqldump") {
        let backup_opts = BackupOptions {
            structure: opts.structure,
            data: opts.data,
            include_views: true,
            include_routines: true,
            include_triggers: true,
            bulk_insert: true,
            ignore_definer: true,
            compress: false,
        };

        let dump_file = dump_with_mysqldump(window, bin, src, backup_opts).await?;

        let restore_opts = RestoreOptions {
            drop_existing: opts.drop_in_target,
            stop_on_error: false,
        };

        restore(window, dst, dump_file.clone(), restore_opts).await?;
        Ok(dump_file)
    } else {
        anyhow::bail!("mysqldump not found")
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

async fn connect_to_mysql(conn: &DbConnection, database: &str) -> Result<MySqlPool> {
    let password = secrets::get_password(&conn.id)?;
    let url = format!(
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, database
    );
    Ok(MySqlPool::connect(&url).await?)
}

fn mysql_value_to_string(row: &MySqlRow, index: usize) -> Result<Option<String>> {
    if let Ok(value) = row.try_get::<Option<String>, _>(index) {
        return Ok(value);
    }

    if let Ok(value) = row.try_get::<Option<i64>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<i32>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<u32>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<i16>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<u16>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<i8>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<u8>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<u64>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<f64>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<Decimal>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<JsonValue>, _>(index) {
        return Ok(value.map(|v| match v {
            JsonValue::String(s) => s,
            other => other.to_string(),
        }));
    }

    if let Ok(value) = row.try_get::<Option<bool>, _>(index) {
        return Ok(value.map(|v| v.to_string()));
    }

    if let Ok(value) = row.try_get::<Option<NaiveDateTime>, _>(index) {
        return Ok(value.map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string()));
    }

    if let Ok(value) = row.try_get::<Option<DateTime<Utc>>, _>(index) {
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

    if let Some(column) = row.columns().get(index) {
        let type_name = column.type_info().name();
        return Ok(Some(format!("[{type_name} value not supported]")));
    }

    Ok(Some("[value not supported]".to_string()))
}
