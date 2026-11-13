# DBPinball Architecture Audit (M0)

Date: 2026-11-12
Scope: `D:\code\dbpinball` at commit `82a6a52` (branch `main`, 3 files dirty: `.husky/pre-commit`, `package.json`, `scripts/release.cjs`).
Purpose: Establish the evidence baseline required by the CLI-first directive before any refactor.

---

## 1. Repository inventory

### 1.1 Rust backend (`src-tauri`)

| File | Lines | Role |
| --- | --- | --- |
| `src/lib.rs` | 108 | 10 `#[tauri::command]` handlers + `run()` builder |
| `src/main.rs` | 5 | Calls `dbpinball_lib::run()` |
| `src/models.rs` | 56 | `DbConnection`, `DbEngine`, `BackupOptions`, `RestoreOptions`, `CopyOptions`, `QueryResult` |
| `src/secrets.rs` | 21 | `keyring` wrapper, hardcoded service name `dbpinball` |
| `src/runners/mod.rs` | 77 | Engine dispatcher |
| `src/runners/mysql.rs` | 414 | MySQL test/list/query/backup/restore/copy |
| `src/runners/postgres.rs` | 353 | PostgreSQL test/list/query/backup/restore/copy |

Crate: single package `dbpinball` v0.1.0, `[lib] name = "dbpinball_lib"`, `crate-type = ["staticlib", "cdylib", "rlib"]`. No workspace, no second binary, no CLI.

### 1.2 Frontend (`src`)

- React 19 + TypeScript + Vite 7, shadcn/ui + Tailwind v4.
- Pages: `connections.tsx` (21 KB), `backup.tsx`, `restore.tsx`, `copy.tsx`, `jobs.tsx`.
- Workspace components: `WorkspaceShell`, `ResultGrid`, `ItemsSidebar`, `DatabasePicker`, `ConnectionPicker`, `ConnectionDialog`, `TableTabs`.
- Hooks: `useWorkspaceQuery`, `useWorkspaceFullscreen`, `useTauriWindowControls`.
- Tests: 3 files, 10 tests (`ResultGrid`, `WorkspaceShell`, `connections`).

### 1.3 Tooling and release

- `package.json` v0.1.6; `src-tauri/tauri.conf.json` version `0.1.0` (drift).
- Release: `scripts/release.cjs` + `@programinglive/commiter` + `standard-version`, local only.
- No `.github/` directory, no CI pipeline, no runner configuration.
- Docs: `README.md`, `PRD.md`, `DEVELOPMENT.md`, `src-tauri/ARCHITECTURE.md`, `CHANGELOG.md`, `docs/RELEASE_NOTES.md`, plus `IMPLEMENTATION_SUMMARY.md`, `GITHUB_SETUP_SUMMARY.md`, `QUICKSTART.md`, `QUICK_COMMANDS.md`, `COMMITER_SETUP.md`.

---

## 2. Current architecture (as built)

```text
React pages ──invoke()──> #[tauri::command] in lib.rs
                              │
                              ▼
                     runners/mod.rs (dispatcher)
                              │  takes &tauri::Window
                              ▼
              runners/mysql.rs / runners/postgres.rs
                              │
                              ├── secrets::get_password (keyring)
                              ├── sqlx pools
                              └── tokio::process (mysqldump/pg_dump/mysql/psql)
```

The dependency arrow points **from the database logic into Tauri**: `runners/mysql.rs:11` and `runners/postgres.rs:10` both `use tauri::{Window, Emitter};`. The database layer therefore cannot run without a Tauri window, which is the exact inversion the directive forbids.

### 2.1 Command surface (10 commands, `lib.rs:94-105`)

`test_conn`, `list_databases_cmd`, `list_tables_cmd`, `run_query_cmd`, `backup`, `restore`, `copy_db`, `store_password`, `get_password`, `delete_password`.

All are thin wrappers except that `backup`, `restore`, `copy_db` forward `&tauri::Window` into the runner layer.

### 2.2 Frontend call sites

| Call | File:line |
| --- | --- |
| `invoke("backup")` | `src/pages/backup.tsx:86` |
| `invoke("restore")` | `src/pages/restore.tsx:76` |
| `invoke("copy_db")` | `src/pages/copy.tsx:83` |
| `invoke("test_conn")` | `src/pages/connections.tsx:249` |
| `invoke("store_password")` | `src/pages/connections.tsx:214,218` |
| `invoke("delete_password")` | `src/pages/connections.tsx:570` |
| `invoke("list_tables_cmd")` | `src/hooks/useWorkspaceQuery.ts:50` |
| `invoke("run_query_cmd")` | `src/hooks/useWorkspaceQuery.ts:128` |

`list_databases_cmd` and `get_password` are registered but have no frontend caller.

---

## 3. Business logic located inside the Tauri layer

Every database operation lives in `src-tauri/src/runners/*`, which is compiled only as part of the Tauri crate and imports `tauri::Window`. There is no reusable service module. Concretely:

| Capability | Location | Tauri coupling |
| --- | --- | --- |
| Connection test | `mysql.rs:19`, `postgres.rs:18` | none directly, but unreachable outside the crate |
| List databases | `mysql.rs:30`, `postgres.rs:29` | none directly |
| List tables | `mysql.rs:55`, `postgres.rs:65` | none directly |
| Run query | `mysql.rs:64`, `postgres.rs:78` | none directly |
| Backup | `mysql.rs:97`, `postgres.rs:49` | `&Window` parameter, `emit()` |
| Restore | `mysql.rs:222`, `postgres.rs:205` | `&Window` parameter, `emit()` |
| Copy | `mysql.rs:262`, `postgres.rs:245` | `&Window` parameter, `emit()` |
| Progress reporting | `mysql.rs:15`, `postgres.rs:15` | `window.emit("job-progress", …)` |
| Credential retrieval | `secrets.rs:11` | `keyring` only, no provider abstraction |

`runners/mod.rs` is a `match` dispatcher, not an application service: it has no policy, no validation, no confirmation, no dry-run, and it passes `&Window` through.

---

## 4. Security and safety assessment

### 4.1 Credentials

- Passwords are stored in the OS keyring under service `dbpinball`, account = connection id (`secrets.rs:3-8`). Good baseline.
- Connection metadata (host, port, user, database, SSL flag) is stored in **webview `localStorage`** as `dbpinball_connections` (`connections.tsx:149,159`; also read by `backup.tsx:63`, `restore.tsx:53`, `copy.tsx:54`). Not secret, but it is the only source of truth and is not validated.
- Connection URLs are built by raw string interpolation: `format!("mysql://{}:{}@{}:{}/{}", username, password, …)` (`mysql.rs:22,38,189,325`; `postgres.rs:21,37,172,308`). A password containing `@`, `:`, `/`, or `#` produces a malformed URL, and sqlx errors can echo the URL.
- No credential-provider abstraction, so a CLI or CI run has no alternative to the OS keyring.

### 4.2 Source/destination policy

- **None.** `restore` and `copy_db` accept any connection and write immediately. There is no notion of production vs local, no allow-list, no confirmation gate, no dry-run.
- `RestoreOptions.drop_existing` and `CopyOptions.drop_in_target` are **ignored**: both restore functions take `_opts` (`mysql.rs:226`, `postgres.rs:209`). The UI checkbox therefore does nothing.
- `CopyOptions` is reduced to a dump-then-restore through `~/dbpinball/backups` (`mysql.rs:282-289`, `postgres.rs:265-272`) with no confirmation and no cleanup.

### 4.3 Destructive and correctness hazards

1. **Internal dumper is a stub.** `internal_dump` creates the output file and only emits progress strings; it never writes SQL (`mysql.rs:182-220`, `postgres.rs:165-203`). When `mysqldump`/`pg_dump` is absent, the app reports "Backup completed." and leaves an **empty file**. This is a silent data-loss trap and contradicts `README.md:153` and `PRD.md` §3.2.
2. **Arbitrary SQL execution.** `run_query` passes frontend SQL straight to `sqlx::query` (`mysql.rs:73`, `postgres.rs:87`). Any `DROP`, `UPDATE`, or `DELETE` typed in the workspace executes with no read-only guard and no statement classification.
3. **`use_ssl` is never read.** `models.rs:13` declares it; no runner references it. The TLS toggle is decorative.
4. **Wrong mysqldump flag.** `include_views` maps to `--events` (`mysql.rs:147-149`). Views are dumped by default; events are not views.
5. **Unused options.** `bulk_insert`, `stop_on_error`, `include_views` (semantically) are never applied.
6. **Compression mismatch.** UI says "Compress (ZIP)" (`backup.tsx:212`) but the code uses deflate and writes `.sql.deflate` (`mysql.rs:165,304`; `postgres.rs:287`). `zip` is a dependency but unused for this path.
7. **No timeouts, cancellation, retries, or signal handling** anywhere in the process-spawning paths.
8. **No structured errors.** `anyhow` is flattened to `String` at the Tauri boundary (`lib.rs:15,20,25,37,48,59,72`). No error codes, no machine-readable contract.
9. **CSP disabled** (`tauri.conf.json` `"csp": null`).

---

## 5. Test and release assessment (executed)

| Command | Result |
| --- | --- |
| `npm run test` (vitest 4.0.8) | **3 files, 10 tests passed**, 11.7 s |
| `cargo test` (rustc 1.90.0) | **0 tests** in `dbpinball_lib`, `dbpinball` bin, and doc-tests; build finished in 2 m 20 s |
| CI | none configured |
| Release | `node scripts/release.cjs` local only; no pipeline, no runner |

There is **zero Rust test coverage** for the database layer, the safety policy, or the process-spawning paths. The only automated coverage is UI markup regression.

---

## 6. Gap analysis against the CLI-first directive

| Directive requirement | Current state | Gap |
| --- | --- | --- |
| Core must not depend on CLI/Desktop | `runners/*` import `tauri::Window` | **Violated** |
| Application services reusable by CLI and Desktop | No service layer; dispatcher takes `&Window` | **Missing** |
| `dbpinball` executable | No CLI binary; only the Tauri app | **Missing** |
| CLI is the primary feature contract | Features exist only as Tauri commands | **Missing** |
| Stable JSON output on stdout | Errors are `String`; no JSON contract | **Missing** |
| Predictable exit codes / structured error codes | None | **Missing** |
| Noninteractive mode, no prompts in CI | No CLI, no mode flag | **Missing** |
| Dry-run for mutating operations | None | **Missing** |
| Explicit destructive-operation confirmation | None; `drop_existing` ignored | **Missing** |
| Bounded retries and timeouts | None | **Missing** |
| Cancellation and signal handling | None | **Missing** |
| Redacted diagnostics | URLs interpolate passwords; no redaction | **Missing** |
| UI-independent progress abstraction | `window.emit` only | **Missing** |
| Credential provider abstraction | `keyring` hardcoded | **Missing** |
| Centralized source/destination security policy | None | **Missing** |
| No duplicate execution logic CLI vs Desktop | N/A today, but the refactor must preserve this | **At risk** |
| Unit/integration tests for services and CLI contracts | 0 Rust tests | **Missing** |
| Documentation of behavior and limits | Docs describe a fallback dumper that does not exist | **Inaccurate** |

---

## 7. Minimal refactor proposal

### 7.1 Target layout (Cargo workspace, 3 members)

```text
dbpinball/
├── Cargo.toml                  # [workspace] members
├── crates/
│   ├── dbpinball-core/         # domain + application services (no tauri, no clap)
│   │   └── src/
│   │       ├── domain/         # models, engine enum, error codes, safety policy
│   │       ├── ports/          # ProgressSink, CredentialProvider, ArtifactStore, Clock
│   │       └── services/       # inspect, query, backup, restore, copy
│   └── dbpinball-cli/          # bin "dbpinball" (clap, JSON, exit codes)
└── src-tauri/                  # desktop adapter, depends on dbpinball-core
```

This is deliberately **two new crates, not five**. The directive prefers a cohesive core over fragmentation, so domain, ports, and services stay as modules inside `dbpinball-core`.

### 7.2 Ports (traits) that remove the Tauri dependency

- `ProgressSink` — `fn emit(&self, event: ProgressEvent)`. Desktop implements it with `window.emit`; CLI implements it with stderr lines or NDJSON.
- `CredentialProvider` — `fn get(&self, id: &str) -> Result<Secret>`. Desktop/CLI use the keyring provider; CI can use an env provider.
- `ArtifactStore` — resolves and creates backup paths.
- `CancellationToken` — checked between tables and during stream copies.

### 7.3 Safety policy (centralized, in core)

- `TargetClass { Production, Staging, Local, Unknown }` derived from an explicit per-connection flag, not from hostname guessing.
- `OperationGuard` requires `--confirm <operation>` (CLI) or an explicit UI confirmation (desktop) before any write to a non-local target.
- `dry_run` produces the full plan (statements, target, artifact path) without executing.
- `run_query` gains a `ReadOnly` mode that rejects non-`SELECT`/`EXPLAIN`/`SHOW` statements by default.

### 7.4 CLI contract (proposed, to be validated)

```bash
dbpinball --version
dbpinball doctor
dbpinball connections list [--json]
dbpinball connections test <id> [--json]
dbpinball schema inspect --connection <id> --database <db> [--json]
dbpinball query run --connection <id> --database <db> --sql <sql> [--read-only] [--json]
dbpinball backup --connection <id> [--structure|--data] [--compress] [--json]
dbpinball restore --connection <id> --file <path> [--drop-existing] [--dry-run] [--confirm restore] [--json]
dbpinball copy --source <id> --target <id> [--dry-run] [--confirm copy] [--json]
```

Exit codes: `0` success, `2` usage error, `3` connection failure, `4` policy refusal, `5` operation failure, `6` cancelled.

### 7.5 Migration order (maps to directive milestones)

1. **M1** — Create the workspace and `dbpinball-core`; move `models.rs` and `secrets.rs` behind ports; introduce `ProgressSink`; make `runners/*` Tauri-free. Fix the safety-critical defects in the same pass: stub dumper, ignored `drop_existing`, unrestricted `run_query`, unused `use_ssl`, `include_views`→`--events`.
2. **M2** — Add `dbpinball-cli` with `doctor`, `connections`, `schema inspect`, JSON output, exit codes, noninteractive mode.
3. **M3** — PostgreSQL safe sync through the CLI (backup → restore → copy) with dry-run and confirmation.
4. **M4** — Rewire `src-tauri` commands to call `dbpinball-core` services; delete duplicated logic.
5. **M5** — MySQL through the same contracts.
6. **M6** — Snapshots, masking, portability.

### 7.6 Compatibility constraints

- The 10 existing Tauri command names and their camelCase payloads must keep working during M1–M4 so the React app is not broken mid-migration.
- `localStorage` connection storage stays until a core-owned connection store replaces it; the refactor must not silently drop saved connections.
- The 10 existing vitest tests must stay green.

---

## 8. Open questions for the owner

1. Should `dbpinball-core` be one crate with `domain`/`ports`/`services` modules (proposed), or separate `-domain` and `-app` crates?
2. Where should the connection store live after M1 — keep `localStorage`, or move to a core-owned file store so the CLI can read the same connections?
3. Is the production/local classification an explicit per-connection flag the user sets, or should it be inferred from a config allow-list?
4. Should the CLI ship in the same release artifact as the desktop app, or as a separate binary?
