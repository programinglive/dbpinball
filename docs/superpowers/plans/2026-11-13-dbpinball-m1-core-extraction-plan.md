# M1 Implementation Plan — Incremental Core Extraction

Date: 2026-11-13
Status: Ready for implementation
Spec: `docs/superpowers/specs/2026-11-12-dbpinball-cli-first-architecture-design.md` §11 (M1)
Baseline: `080c3a0` (hardening complete, working tree clean)

---

## 1. Objective

Introduce a Cargo workspace and a `dbpinball-core` crate, and extract **only** the connection-inspection capability (test connection, list databases, list tables) into it. The desktop app keeps working through the same Tauri commands and payloads. Nothing else moves.

## 2. Scope

### In scope

- Root Cargo workspace with `crates/dbpinball-core` and `src-tauri` as members.
- Move `src-tauri/Cargo.lock` to the workspace root.
- `dbpinball-core` with `domain/`, `ports/`, `adapters/`, `services/inspect.rs`.
- `KeyringCredentialProvider` and `NoopProgress`.
- `runners/mod.rs` inspect functions delegate to core.
- `scripts/check-boundaries.cjs`.
- Rust unit tests and gated integration tests.
- Documentation updates.

### Out of scope

- The `dbpinball` CLI crate (M2).
- `SafetyPolicy`, `DestinationVerifier`, execution plans (M3).
- Safe Sync, masking, snapshots (M4–M7).
- Backup, restore, copy, and query extraction.
- Any behavior change to existing commands.
- New database engines or new dependencies beyond what the core needs.

## 3. Prerequisites

- Hardening commits present and working tree clean (`080c3a0`).
- `cargo` 1.90+, Node 22+, `npm ci` succeeds.
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `npm test` all green at baseline.

## 4. Target state

```text
dbpinball/
├── Cargo.toml                       # [workspace] members
├── Cargo.lock                       # moved from src-tauri/
├── crates/
│   └── dbpinball-core/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── domain/
│           │   ├── mod.rs
│           │   ├── connection.rs
│           │   └── error.rs
│           ├── ports/
│           │   ├── mod.rs
│           │   ├── progress.rs
│           │   └── credentials.rs
│           ├── adapters/
│           │   ├── mod.rs
│           │   └── keyring.rs
│           └── services/
│               ├── mod.rs
│               └── inspect.rs
├── scripts/
│   └── check-boundaries.cjs
└── src-tauri/                       # unchanged command surface
```

## 5. Work breakdown

### Task 1 — Create the workspace

Files:
- `Cargo.toml` (new, root)
- `Cargo.lock` (moved from `src-tauri/Cargo.lock`)
- `src-tauri/Cargo.lock` (deleted)

Root manifest:

```toml
[workspace]
resolver = "2"
members = ["crates/dbpinball-core", "src-tauri"]
```

Steps:

1. Create the root `Cargo.toml`.
2. `git mv src-tauri/Cargo.lock Cargo.lock`.
3. Run `cargo metadata --no-deps` to confirm both members resolve.
4. Run `cargo build -p dbpinball` to confirm the Tauri crate still builds from the workspace root.

Verification:

```bash
cargo metadata --no-deps --format-version 1
cargo build -p dbpinball
```

Rollback: delete the root `Cargo.toml`, `git mv Cargo.lock src-tauri/Cargo.lock`.

### Task 2 — Create the core crate

Files:
- `crates/dbpinball-core/Cargo.toml`
- `crates/dbpinball-core/src/lib.rs`

Manifest:

```toml
[package]
name = "dbpinball-core"
version = "0.1.0"
edition = "2021"
license = "MIT"

[dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "process", "fs", "io-util"] }
sqlx = { version = "0.8", default-features = false, features = [
  "runtime-tokio-native-tls", "postgres", "mysql", "chrono", "rust_decimal", "json"
] }
keyring = "2"
anyhow = "1"

[features]
integration = []
```

**No `tauri`, no `clap`.** The `json` feature is required for parity with the app's `serde_json::Value` decoding.

### Task 3 — Domain

`domain/connection.rs`:

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub engine: DbEngine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub use_ssl: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DbEngine { Mysql, Postgres }
```

This is byte-compatible with the current `src-tauri/src/models.rs` shape. No `target` field in M1; it arrives in M3.

`domain/error.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode { Usage, Connection, Operation, NotFound, Unsupported }

#[derive(Debug, Clone, Serialize)]
pub struct CoreError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<String>,
}
```

`CoreError::redacted` strips `scheme://user:pass@` from any message. Unit tests cover it.

### Task 4 — Ports

`ports/progress.rs`:

```rust
pub enum Phase { Connecting, Inspecting, Done }
pub struct ProgressEvent { pub phase: Phase, pub message: String }
pub trait ProgressSink: Send + Sync { fn emit(&self, event: ProgressEvent); }
pub struct NoopProgress;
```

`ports/credentials.rs`:

```rust
pub struct Secret(String);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self;
    pub fn expose(&self) -> &str;
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

pub trait CredentialProvider: Send + Sync {
    fn get(&self, id: &str) -> Result<Secret, CoreError>;
    fn set(&self, id: &str, secret: &Secret) -> Result<(), CoreError>;
    fn delete(&self, id: &str) -> Result<(), CoreError>;
}
```

`Secret` must not implement `Serialize`. The manual `Debug` is the enforcement for "no password in logs."

### Task 5 — Keyring adapter

`adapters/keyring.rs`:

```rust
pub struct KeyringCredentialProvider;
```

Uses service name `dbpinball` and account = connection id, exactly as `src-tauri/src/secrets.rs` does today. Existing keyring entries keep working.

### Task 6 — Inspect service

`services/inspect.rs`:

```rust
pub async fn test_connection(conn: &DbConnection, creds: &dyn CredentialProvider) -> Result<(), CoreError>;
pub async fn list_databases(conn: &DbConnection, creds: &dyn CredentialProvider) -> Result<Vec<String>, CoreError>;
pub async fn list_tables(conn: &DbConnection, database: &str, creds: &dyn CredentialProvider) -> Result<Vec<String>, CoreError>;
```

Move the implementations from `runners/mysql.rs` (`test`, `list_databases`, `list_tables`) and `runners/postgres.rs` (same). Preserve the exact SQL and filtering:

- MySQL: `SHOW DATABASES`, filter `information_schema`, `performance_schema`, `mysql`, `sys`.
- PostgreSQL: `SELECT datname FROM pg_database WHERE datistemplate = false ORDER BY datname`.
- MySQL tables: `SHOW TABLES`.
- PostgreSQL tables: `information_schema.tables WHERE table_schema = 'public' ORDER BY table_name`.

**Preserve the current URL construction in M1.** The password-in-URL defect (D6) is fixed in M3, not here. M1 must not change behavior.

### Task 7 — Rewire the Tauri adapter

Files:
- `src-tauri/Cargo.toml` (add `dbpinball-core = { path = "../crates/dbpinball-core" }`)
- `src-tauri/src/models.rs` (re-export core types)
- `src-tauri/src/runners/mod.rs` (inspect functions delegate to core)
- `src-tauri/src/runners/mysql.rs`, `postgres.rs` (remove the now-duplicated inspect functions)

`models.rs`:

```rust
pub use dbpinball_core::domain::connection::{DbConnection, DbEngine};
// BackupOptions, RestoreOptions, CopyOptions, QueryResult stay local
```

`runners/mod.rs`:

```rust
pub async fn test_connection(conn: DbConnection) -> Result<()> {
    dbpinball_core::services::inspect::test_connection(&conn, &KeyringCredentialProvider)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))
}
```

The same pattern for `list_databases` and `list_tables`. `lib.rs` is **not changed**: the 10 command names and payloads stay identical.

This keeps `runners/` in place and avoids dead code. Backup, restore, copy, and query runners are untouched.

### Task 8 — Boundary check

`scripts/check-boundaries.cjs`:

1. Parse `crates/dbpinball-core/Cargo.toml` and fail if it contains `tauri` or `clap`.
2. Run `cargo tree -p dbpinball-core --prefix none` and fail if any line starts with `tauri` or `clap`.

Exit 1 on violation, 0 otherwise. Add it to CI as a step in the `rust` job.

### Task 9 — Tests

Unit tests (in `dbpinball-core`):

| Test | Proves |
| --- | --- |
| `error::tests::redaction_removes_password` | `CoreError::redacted` strips credentials |
| `error::tests::url_never_in_message` | a URL-shaped message is redacted |
| `connection::tests::deserializes_frontend_payload` | camelCase payload from the frontend deserializes |
| `connection::tests::engine_is_lowercase` | `"mysql"` / `"postgres"` round-trip |
| `credentials::tests::secret_debug_is_redacted` | `format!("{:?}", Secret)` never contains the value |
| `inspect::tests::resolve_database_prefers_requested` | database resolution order |
| `inspect::tests::resolve_database_requires_name` | empty database is an error |

Integration tests (`crates/dbpinball-core/tests/inspect_integration.rs`, gated):

```rust
#![cfg(feature = "integration")]
```

Reads `DBPINBALL_TEST_PG_URL` and `DBPINBALL_TEST_MYSQL_URL`. Skips with a clear message when unset. Covers `test_connection`, `list_databases`, `list_tables` against disposable databases.

### Task 10 — Documentation

- `README.md`: project layout gains `crates/dbpinball-core`; testing section gains `cargo test -p dbpinball-core`.
- `src-tauri/ARCHITECTURE.md`: note that inspect now lives in core and the runner delegates.
- `CONTRIBUTING.md`: add the boundary check command.

## 6. Compatibility guarantees

| Guarantee | How it is preserved |
| --- | --- |
| 10 Tauri command names | `lib.rs` unchanged |
| Payload shapes | `models.rs` re-exports the same serde shape |
| Keyring entries | same service name and account key |
| Backup/restore/copy/query behavior | runners untouched |
| Frontend | no changes |
| Existing tests | 10 vitest tests stay green |

## 7. Verification commands

```bash
cargo metadata --no-deps --format-version 1
cargo check -p dbpinball-core
cargo test -p dbpinball-core
node scripts/check-boundaries.cjs
cargo build -p dbpinball
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
npm run lint
npm run build
```

## 8. Rollback

Revert the M1 commits. The workspace root and `Cargo.lock` move are reverted together. `runners/` still contains backup, restore, copy, and query, so the desktop returns to its prior state. No data migration, no config change, no keyring change.

## 9. Risks

| Risk | Mitigation |
| --- | --- |
| Workspace lockfile move breaks resolution | `cargo metadata` and `cargo build -p dbpinball` in Task 1 before any extraction |
| Tauri build script path assumptions | `tauri.conf.json` paths are relative to `src-tauri`; verify `cargo build -p dbpinball` |
| sqlx feature drift | core uses the same feature set including `json` |
| Dead code after extraction | runners delegate to core instead of being orphaned |
| Behavior change in inspect | SQL and filtering copied verbatim; integration tests compare results |
| Clippy `-D warnings` on the workspace | run `cargo clippy --workspace` before committing |

## 10. Acceptance criteria

1. `cargo check -p dbpinball-core` passes and `cargo tree -p dbpinball-core` contains no `tauri` or `clap`.
2. `node scripts/check-boundaries.cjs` passes, and fails when `tauri` is added to the core manifest.
3. `cargo test -p dbpinball-core` passes with the unit tests above.
4. `cargo build -p dbpinball` passes.
5. `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` pass.
6. `npm test` reports 10/10.
7. All 10 Tauri command names and payloads are unchanged.
8. `runners/` still contains backup, restore, copy, and query.
9. No Safe Sync, CLI, masking, or policy code is introduced.

## 11. Definition of done

- Workspace and core crate exist and build.
- Inspect is the only extracted capability.
- Desktop behavior is unchanged.
- Unit tests and gated integration tests exist and pass.
- Boundary check is wired into CI.
- Documentation reflects the new layout.
- Rollback is documented and verified by reverting the commits in a scratch branch.
