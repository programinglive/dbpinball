# M2 Implementation Plan — Standalone `dbpinball` CLI Foundation

Date: 2026-11-13
Status: Ready for implementation
Spec: `docs/superpowers/specs/2026-11-12-dbpinball-cli-first-architecture-design.md` §7, §8, §11 (M2)
Baseline: `605320d` (M1 merged, six CI jobs green, eight real-database integration tests passing)

---

## 1. Objective

Ship a standalone `dbpinball` binary that exposes the shared `dbpinball-core` inspection services headlessly, with a stable JSON envelope, deterministic exit codes, and no credential leakage. The desktop stays the same and keeps using the core through Tauri.

M2 is **read-only against databases**. It manages local connection profiles, but it never writes to a database.

## 2. Scope

### In scope

- `crates/dbpinball-cli` with Clap.
- `dbpinball --version`, `doctor`, `connections list|add|set-password|test|remove`, `schema inspect`.
- A core-owned connection profile store (`connections.json`), metadata only.
- Keyring credential provider plus an explicit environment provider for CI.
- JSON envelope, human output, exit-code matrix.
- Unit, CLI contract, and gated database integration tests.
- Migration design from the desktop's `localStorage` profiles.

### Out of scope

- `query run` (arbitrary SQL), even read-only. Deferred to a later milestone with the policy.
- Backup, restore, copy, Safe Sync, masking, snapshots.
- New database engines.
- Tauri command changes.
- Laravel adapter changes.

**Deviation from the spec:** §11 M2 lists `query run`. The owner's M2 scope explicitly forbids arbitrary SQL execution, so this plan omits it. The spec should be amended when M3 lands the read-only query policy.

## 3. CLI command contract

Global options: `--json`, `--config-dir <path>`, `--credential-source <keyring|env>`, `--version`, `--help`.

| Command | Inputs | Output (human) | Output (`--json` data) | Exit codes |
| --- | --- | --- | --- | --- |
| `dbpinball --version` | — | version line | — | 0 |
| `dbpinball doctor` | — | environment table | `{ checks: [{name, status, detail}] }` | 0, 5 |
| `dbpinball connections list` | — | table of profiles (no secrets) | `{ connections: [{id, name, engine, host, port, database, username, useSsl, target}] }` | 0, 5 |
| `dbpinball connections add` | `--name`, `--engine`, `--host`, `--port`, `--database`, `--username`, `--target`, `--use-ssl`; password on **stdin** | created profile summary | `{ connection: {...} }` | 0, 2, 3, 5 |
| `dbpinball connections set-password <id>` | password on **stdin** | confirmation | `{ updated: true }` | 0, 3, 5, 7 |
| `dbpinball connections test <id>` | — | result line | `{ id, reachable: true }` | 0, 3, 7 |
| `dbpinball connections remove <id>` | `--yes` (required when non-interactive) | confirmation | `{ removed: true }` | 0, 4, 5, 7 |
| `dbpinball schema inspect` | `--connection <id>`, optional `--database <db>` | database list or table list | see §7 | 0, 3, 5, 7 |

Rules:

- No `--password` flag exists. `connections add` and `set-password` read the password from stdin; a TTY uses hidden input, a pipe reads one line.
- `--non-interactive` is implied when stdin is not a TTY. No command prompts in that mode; `connections remove` without `--yes` is a policy refusal (exit 4).
- Every command resolves a profile by id from the store, then calls a shared core service.

## 4. Module boundaries

```text
crates/
├── dbpinball-core/                 # existing, unchanged services
│   └── src/
│       ├── domain/                 # + TargetClass on DbConnection (additive, serde default)
│       ├── ports/credentials.rs    # + set(), delete() on CredentialProvider
│       ├── adapters/keyring.rs     # + set(), delete()
│       └── store/                  # NEW: connection profile store
│           ├── mod.rs
│           ├── record.rs           # ConnectionRecord, StoreFile
│           └── store.rs            # load, save, add, remove, get, import
└── dbpinball-cli/                  # NEW
    ├── Cargo.toml
    └── src/
        ├── main.rs                 # tokio runtime, error -> exit code
        ├── cli.rs                  # clap definitions only
        ├── output.rs               # envelope, human rendering
        ├── exit.rs                 # ErrorCode -> exit code
        ├── config.rs               # config dir resolution
        └── commands/
            ├── mod.rs
            ├── doctor.rs
            ├── connections.rs
            └── schema.rs
```

Boundaries:

- `dbpinball-cli` depends on `dbpinball-core`, `clap`, `serde`, `serde_json`, `tokio`, `dirs`, `rpassword`.
- `dbpinball-core` still depends on neither `tauri` nor `clap`. The boundary check stays green.
- `main.rs` owns process concerns (exit code, stdout/stderr). `commands/*` own argument-to-service mapping. No command builds SQL.

## 5. Core additions

### 5.1 Connection profile store

`store` takes a directory path; it does not depend on `dirs`, so it is testable and reusable by the desktop.

```rust
pub struct ConnectionRecord {
    pub connection: DbConnection,
    pub target: TargetClass,          // defaults to Unknown
    pub migrated_from: Option<String>,
}

pub struct ConnectionStore { /* root: PathBuf */ }

impl ConnectionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self;
    pub fn path(&self) -> PathBuf;                 // <root>/connections.json
    pub fn load(&self) -> Result<Vec<ConnectionRecord>, CoreError>;
    pub fn save(&self, records: &[ConnectionRecord]) -> Result<(), CoreError>;
    pub fn add(&self, record: ConnectionRecord) -> Result<(), CoreError>;
    pub fn remove(&self, id: &str) -> Result<bool, CoreError>;
    pub fn get(&self, id: &str) -> Result<Option<ConnectionRecord>, CoreError>;
    pub fn import(&self, records: Vec<ConnectionRecord>) -> Result<ImportReport, CoreError>;
}
```

- File format: `{ "version": 1, "migratedFrom": null, "connections": [...] }`.
- Atomic write: temp file in the same directory, `0600` on Unix, `rename` into place.
- Invalid JSON is a `CoreError::Operation`; individual invalid records are dropped and reported by index.
- Duplicate detection by `(engine, host, port, database, username)`; the first wins.
- No password field exists; a serialization test asserts the key is absent.

### 5.2 `TargetClass` on the connection

Add `target: TargetClass` to `DbConnection` with `#[serde(default)]` so existing desktop payloads still deserialize. `TargetClass` is `Production | Staging | Local | Unknown`, default `Unknown`.

M2 uses it only as a label. It does not enforce policy; that is M3.

### 5.3 Credential port

Extend the trait:

```rust
pub trait CredentialProvider: Send + Sync {
    fn get(&self, connection_id: &str) -> Result<Secret, CoreError>;
    fn set(&self, connection_id: &str, secret: &Secret) -> Result<(), CoreError>;
    fn delete(&self, connection_id: &str) -> Result<(), CoreError>;
}
```

`KeyringCredentialProvider` implements all three with service `dbpinball` and account = connection id, unchanged from M1. The integration test's fixed provider is updated.

A new `EnvCredentialProvider` lives in the CLI (not the core), reading `DBPINBALL_PASSWORD` for the selected profile. It is only constructed when `--credential-source env` is passed.

## 6. Credential and configuration storage

### 6.1 Locations

| Purpose | Path |
| --- | --- |
| Config root | `dirs::config_dir()/dbpinball/` |
| Profiles | `<config root>/connections.json` |
| Keyring service | `dbpinball` |
| Keyring account | connection id |

Resolved: `%APPDATA%\dbpinball\` on Windows, `~/Library/Application Support/dbpinball/` on macOS, `$XDG_CONFIG_HOME/dbpinball/` or `~/.config/dbpinball/` on Linux.

`--config-dir` overrides the root for tests and advanced use.

### 6.2 Credential strategy

| Context | Provider | Notes |
| --- | --- | --- |
| Desktop, interactive CLI | `KeyringCredentialProvider` | Default |
| CI, headless automation | `EnvCredentialProvider` | Requires `--credential-source env`; reads `DBPINBALL_PASSWORD` |
| Keyring unavailable | Error `CredentialUnavailable` → exit 3 | Never falls back to plaintext |

Passwords never appear in argv, the store, JSON, stderr, or errors. `Secret` already has a redacted `Debug`.

## 7. JSON output schema

### 7.1 Envelope

```json
{ "ok": true,  "data": { }, "error": null }
{ "ok": false, "data": null, "error": { "code": "connection", "message": "..." } }
```

- Exactly one object on stdout when `--json` is set.
- `error.code` is the snake_case `ErrorCode`.
- `error.message` is redacted by `CoreError::redacted`.
- Progress and warnings go to stderr, never stdout.
- Without `--json`, stdout carries human text and stderr carries warnings.

### 7.2 `schema inspect` data

Without `--database`:

```json
{
  "connectionId": "abc",
  "databases": ["app", "postgres"]
}
```

With `--database`:

```json
{
  "connectionId": "abc",
  "database": "app",
  "tables": ["users", "orders"]
}
```

### 7.3 `doctor` data

```json
{
  "version": "0.1.6",
  "checks": [
    { "name": "config_dir", "status": "ok", "detail": "/home/me/.config/dbpinball" },
    { "name": "keyring", "status": "ok", "detail": null },
    { "name": "connection_store", "status": "ok", "detail": "2 profiles" },
    { "name": "mysqldump", "status": "warn", "detail": "not found in PATH" }
  ]
}
```

`status` is `ok | warn | fail`. A `fail` exits 5; a `warn` exits 0.

## 8. Exit-code matrix

| `ErrorCode` | Exit | Meaning |
| --- | --- | --- |
| success | 0 | Command completed |
| `Usage` | 2 | Invalid arguments or missing required input |
| `Connection` / `CredentialUnavailable` | 3 | Connection or credential failure |
| `PolicyRefusal` | 4 | Reserved; used by `connections remove` without `--yes` in non-interactive mode |
| `Operation` / `Unsupported` | 5 | Local operation or environment failure |
| `Cancelled` | 6 | Reserved |
| `NotFound` | 7 | Unknown connection id |
| `Destination*` | 8 | Reserved (M3) |
| `IntegrityViolation` | 9 | Reserved (M4+) |

Clap parse failures map to 2. A panic must not escape; `main` maps unknown failures to 5.

## 9. Migration strategy

The desktop stores profiles in webview `localStorage` under `dbpinball_connections` (`src/pages/connections.tsx:149,159`). The CLI cannot read that. M2 therefore ships the file store and an explicit migration path.

### 9.1 Store bootstrap for CLI users

- `connections add` creates the first profile directly in the file store.
- `connections import --file <path>` accepts a JSON array of records (or the store shape), validates, deduplicates, and writes atomically. This is the escape hatch for scripted onboarding.

### 9.2 Desktop migration (follow-up M2.1)

The migration must run in the desktop because only the webview can read `localStorage`.

1. Desktop reads `localStorage.dbpinball_connections`.
2. It calls a new Tauri command `connections_import(records)`.
3. The command validates, drops invalid records with a report, deduplicates, sets `target: "unknown"`, and writes the file with `migratedFrom: "localStorage@v1"`.
4. The localStorage key is kept for one release as a rollback path.
5. Re-running is idempotent: a non-null `migratedFrom` short-circuits.

This is a desktop change, so it is **not** part of the CLI crate. It is scheduled as M2.1 so M2 stays focused.

### 9.3 Compatibility contract

- Until M2.1 ships, the desktop and CLI can diverge: the desktop keeps its `localStorage` profiles and the CLI has its own file store.
- After M2.1, the desktop reads the file store as the source of truth and `localStorage` becomes a one-release fallback.
- Rollback: delete `connections.json`; `localStorage` is untouched, so the desktop behaves as before.

## 10. Cross-platform risks

| Risk | Mitigation |
| --- | --- |
| Config directory differs per OS | `dirs::config_dir()`; `--config-dir` override for tests |
| Keyring backend absent on headless Linux | `EnvCredentialProvider` opt-in; `doctor` reports keyring status |
| Windows ACLs ignore `0600` | Documented; the config directory is user-scoped by the OS |
| Binary name differs (`.exe`) | Clap uses `env!("CARGO_PKG_NAME")`; tests use `CARGO_BIN_EXE_dbpinball` |
| Path separators in output | Paths are printed as resolved; JSON carries strings from `dirs` |
| Hidden password input differs per terminal | `rpassword` handles TTYs; pipes read a plain line |

## 11. Test matrix

| Layer | Tests |
| --- | --- |
| Core unit | Store round-trip, atomic write, invalid record dropped, duplicate dedupe, missing file, version mismatch, no password field |
| Core unit | `TargetClass` default and serde compatibility with old payloads |
| CLI unit | Clap parses every command; unknown command exits 2; `--json` flag |
| CLI unit | Exit-code mapping for each `ErrorCode` |
| CLI contract | Spawn `CARGO_BIN_EXE_dbpinball`; `--version`, `doctor --json`, `connections list --json`, unknown id `connections test` (exit 7), `schema inspect` unknown id (exit 7), `connections remove` without `--yes` non-interactive (exit 4) |
| CLI contract | `connections add` reads the password from stdin; a `--password` flag is rejected (exit 2) |
| Security | `connections list --json` contains no password; error envelope is redacted |
| Integration (gated) | `connections test` and `schema inspect` against disposable PostgreSQL/MySQL, reusing `DBPINBALL_TEST_*` |
| Regression | `cargo test --workspace`, boundary check, desktop frontend 11 tests, PHP 55 tests |

CLI contract tests use `std::process::Command` with `env!("CARGO_BIN_EXE_dbpinball")` and a `--config-dir` temp directory; no extra test harness dependency.

## 12. Acceptance criteria

1. `cargo run -p dbpinball-cli -- --version` prints a version.
2. `dbpinball doctor --json` emits one valid envelope and exits 0.
3. `connections add` reads the password from stdin; `--password` is rejected with exit 2.
4. `connections list --json` never contains a password or a full connection URL.
5. `connections test` exits 3 for a wrong password and 7 for an unknown id.
6. `schema inspect` lists databases and, with `--database`, tables, against disposable PostgreSQL and MySQL.
7. `--json` writes exactly one object to stdout; diagnostics go to stderr.
8. `dbpinball-core` still has no `tauri` or `clap` dependency.
9. The desktop build and its 11 frontend tests pass; PHP 55 tests pass.
10. CLI contract tests run in the normal CI; database integration stays in the gated job.

## 13. Rollback

- Remove `crates/dbpinball-cli` and its CI job. The core and desktop are unaffected.
- The `store` module and the `TargetClass` field are additive; keep or revert them independently.
- No data migration is required: `connections.json` is new and separate from `localStorage`.

## 14. Open questions

1. **Desktop migration ownership.** Confirm M2.1 (desktop import command) as a separate change, or fold it into M2.
2. **`rpassword` dependency.** Accept it for hidden TTY input, or read plain stdin and document the echo risk.
3. **`TargetClass` in M2.** Confirm adding the field now (additive, default `Unknown`) so the store format is stable before M3.
4. **`connections remove` confirmation.** Confirm `--yes` plus non-interactive refusal as the M2 behavior.

No architectural blockers. The M1 core API is sufficient; the store and CLI are additive.
