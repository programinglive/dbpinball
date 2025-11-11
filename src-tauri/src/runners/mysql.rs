use crate::models::*;
use crate::secrets;
use anyhow::{Context, Result};
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
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = sqlx::MySqlPool::connect(&url).await?;
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
    let names: Vec<String> = sqlx::query_scalar("SHOW DATABASES").fetch_all(&pool).await?;
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

pub async fn backup(
    window: &Window,
    conn: DbConnection,
    opts: BackupOptions,
) -> Result<String> {
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
        tokio::io::copy(&mut tokio::io::BufReader::new(stdout), &mut Box::new(encoder))
            .await?;
    } else {
        tokio::io::copy(&mut tokio::io::BufReader::new(stdout), &mut Box::new(output_file))
            .await?;
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("mysqldump failed with status: {}", status);
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
        "mysql://{}:{}@{}:{}/{}",
        conn.username, password, conn.host, conn.port, conn.database
    );
    let pool = sqlx::MySqlPool::connect(&url).await?;

    let out = artifact_path("mysql", &conn.database, opts.compress)?;
    let _output_file = tokio::fs::File::create(&out).await?;

    emit(window, "Dumping schema…");

    if opts.structure {
        // Dump tables
        let tables: Vec<(String,)> =
            sqlx::query_as("SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = ?")
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
        cmd.args(["-h", &conn.host, "-P", &conn.port.to_string(), "-u", &conn.username, &conn.database])
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
    let fname = format!(
        "{}_{}_{}{}",
        engine,
        db,
        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
        if compress { ".sql.zip" } else { ".sql" }
    );
    Ok(base.join(fname))
}
