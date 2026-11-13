# dbpinball Backend Architecture

## Overview

The Rust backend is built on Tauri 2.x and provides async database operations for MySQL and PostgreSQL. It uses a vendor-first approach: prefer native tools (mysqldump, pg_dump) when available, with internal dumpers as fallback.

> **Core extraction (M1).** Connection test, database listing, and table listing now live in `crates/dbpinball-core` (`services::inspect`). The Tauri commands delegate to that service through `KeyringCredentialProvider`, so the desktop and the standalone CLI share the same implementation. Backup, restore, copy, and query still live in `runners/` and are unchanged.
>
> **CLI adapter (M2).** `crates/dbpinball-cli` exposes `doctor`, `connections`, and `schema inspect` over the same core services. It is read-only at the database layer and uses the core connection store. It must not contain database execution logic.
>
> `dbpinball-core` must not depend on Tauri or a CLI parser. `scripts/check-boundaries.cjs` enforces this in CI.

## Module Structure

### `lib.rs` - Tauri Commands

Exposes async commands callable from the React frontend:

- `test_conn(conn)` - Test database connection
- `backup(window, conn, opts)` - Start backup operation
- `restore(window, conn, file, opts)` - Start restore operation
- `copy_db(window, src, dst, opts)` - Start database copy
- `store_password(id, password)` - Store password in OS keyring
- `get_password(id)` - Retrieve password from OS keyring
- `delete_password(id)` - Delete password from OS keyring

All commands emit progress events via `window.emit("job-progress", message)`.

### `models.rs` - Data Models

Shared data structures:

- `DbConnection` - Connection details (host, port, database, username, engine)
- `DbEngine` - Enum: Mysql | Postgres
- `BackupOptions` - Backup configuration (structure, data, compression, etc.)
- `RestoreOptions` - Restore configuration (drop_existing, stop_on_error)
- `CopyOptions` - Copy configuration (structure, data, drop_in_target)
- `JobResult` - Result of an operation (status, artifact_path, error)

### `secrets.rs` - Password Management

Uses the `keyring` crate to store/retrieve passwords in OS keystore:

- `store_password(id, password)` - Store in OS keyring
- `get_password(id)` - Retrieve from OS keyring
- `delete_password(id)` - Delete from OS keyring

Service name: "dbpinball"

### `runners/mod.rs` - Operation Dispatcher

Routes operations to engine-specific runners:

- `test_connection(conn)` - Dispatch to MySQL or PostgreSQL test
- `start_backup(window, conn, opts)` - Dispatch to engine-specific backup
- `start_restore(window, conn, file, opts)` - Dispatch to engine-specific restore
- `start_copy(window, src, dst, opts)` - Same-engine copy or error for cross-engine

### `runners/mysql.rs` - MySQL Implementation

**Functions:**

- `test(conn)` - Test connection using sqlx
- `backup(window, conn, opts)` - Backup using mysqldump or internal dumper
- `restore(window, conn, file, opts)` - Restore using mysql client
- `copy_same_engine(window, src, dst, opts)` - Copy via dump+pipe

**Vendor Tool Fallback:**

1. Try `mysqldump` (preferred)
   - Spawns process with args
   - Pipes stdout to file (optionally compressed)
   - Emits progress events
2. Fallback: Internal dumper using sqlx
   - Queries table structure
   - Streams INSERT statements
   - Slower but works without external tools

**Key Features:**

- `--single-transaction` for consistent backups
- `--skip-definer` to avoid permission issues
- `--triggers`, `--routines`, `--events` for objects
- ZIP compression via `async-compression`
- Password via `MYSQL_PWD` env var

### `runners/postgres.rs` - PostgreSQL Implementation

**Functions:**

- `test(conn)` - Test connection using sqlx
- `backup(window, conn, opts)` - Backup using pg_dump or internal dumper
- `restore(window, conn, file, opts)` - Restore using psql
- `copy_same_engine(window, src, dst, opts)` - Copy via dump+pipe

**Vendor Tool Fallback:**

1. Try `pg_dump` (preferred)
   - Spawns process with args
   - Pipes stdout to file
   - Supports `-Fc` (custom format) for compression
2. Fallback: Internal dumper using sqlx
   - Queries table structure
   - Streams INSERT statements

**Key Features:**

- `-s` for structure only, `-a` for data only
- `-Fc` for custom format (compressed)
- Password via `PGPASSWORD` env var

## Async/Await Pattern

All I/O operations use Tokio:

```rust
// File operations
let file = tokio::fs::File::create(path).await?;

// Process spawning
let mut cmd = tokio::process::Command::new(bin);
let mut child = cmd.spawn()?;

// I/O copying
tokio::io::copy(&mut reader, &mut writer).await?;
```

## Error Handling

Uses `anyhow::Result<T>` for error propagation:

```rust
pub async fn backup(...) -> Result<String> {
    // Operations that may fail
    let password = secrets::get_password(&conn.id)?;  // ? propagates error
    let pool = sqlx::MySqlPool::connect(&url).await?;
    // ...
    Ok(artifact_path)
}
```

Errors are converted to strings in Tauri commands:

```rust
#[tauri::command]
async fn backup(...) -> Result<String, String> {
    start_backup(...).await.map_err(|e| e.to_string())
}
```

## Progress Streaming

Operations emit progress via Tauri events:

```rust
fn emit(window: &Window, msg: &str) {
    let _ = window.emit("job-progress", msg);
}

// Usage
emit(window, "Using mysqldump…");
emit(window, "Dumping table: users");
emit(window, "Backup completed.");
```

Frontend listens:

```typescript
const unlisten = listen<string>("job-progress", (e) => {
  setLogs((prev) => [...prev, e.payload]);
});
```

## Artifact Storage

Backups saved to `~/dbpinball/backups/`:

```
~/dbpinball/backups/
├── mysql_mydb_20240101T120000.sql
├── mysql_mydb_20240101T120000.sql.zip
├── postgres_mydb_20240101T120100.sql
└── postgres_mydb_20240101T120100.dump
```

Filename format: `{engine}_{database}_{timestamp}{.extension}`

## Dependencies

Key crates:

- `tauri` - Desktop app framework
- `tokio` - Async runtime (macros, rt-multi-thread, process, fs, io-util)
- `sqlx` - Async SQL toolkit (mysql, postgres, chrono features)
- `keyring` - OS password storage
- `which` - Find executables in PATH
- `zip` - ZIP file creation
- `async-compression` - Async compression (tokio, deflate)
- `chrono` - Timestamps
- `serde` - Serialization
- `anyhow` - Error handling

## Future Enhancements

### Phase 2

- **SSH Tunnels**: Use `ssh2` crate for remote connections
- **Cross-engine Copy**: Type mapping (MySQL TINYINT(1) → Postgres BOOLEAN, etc.)
- **Parallel Dumps**: Spawn multiple mysqldump processes per table
- **Resumable Jobs**: Store job state, resume on failure

### Phase 3

- **Incremental Backups**: Track changes since last backup
- **Scheduled Backups**: Use `tokio-cron-scheduler`
- **Cloud Storage**: Upload to S3, GCS, Azure Blob
- **Database Comparison**: Diff schema and data

## Testing

Run Rust tests:

```bash
cargo test -p dbpinball
```

Integration tests (future):

```bash
# Requires Docker Compose with MySQL + PostgreSQL
cargo test --test integration_tests -- --nocapture
```

## Performance Considerations

1. **Streaming**: Large dumps streamed to disk, not buffered in memory
2. **Compression**: Async compression doesn't block event loop
3. **Connection Pooling**: sqlx manages connection pools
4. **Process Spawning**: Native tools faster than internal dumpers
5. **Parallel Operations**: Future: spawn multiple processes per table

## Security

1. **Passwords**: Never logged, stored in OS keyring only
2. **Connection Strings**: Constructed with secrets, not logged
3. **File Permissions**: Backups created with default umask
4. **TLS**: Supported via sqlx connection strings (future: UI config)
