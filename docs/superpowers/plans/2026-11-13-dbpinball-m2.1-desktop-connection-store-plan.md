# DBPinball M2.1 — Desktop Connection Store Migration

Status: **plan only**. No implementation in this document's change set.

Baseline: `main` at `e026758738b8a2e115e98500b43bce8480f32dd6` (M1 + M2 + identity docs merged).

## 1. Current persistence architecture

### 1.1 Desktop (today)

| Concern | Implementation |
|---|---|
| Profile storage | `localStorage["dbpinball_connections"]` — a JSON array of `DbConnection` |
| Read | `src/pages/connections.tsx:147-156` (`loadConnections`) on mount |
| Write | `src/pages/connections.tsx:158-161` (`saveConnections`) on every create, update and delete |
| Profile id | `conn_${Date.now()}` (`connections.tsx:193`) |
| Password | OS keyring via Tauri `store_password` / `get_password` / `delete_password` |
| Keyring service | `"dbpinball"`, account = connection id (`src-tauri/src/secrets.rs`) |
| Profile shape | `{ id, name, engine, host, port, database, username, useSsl }` — **no `target`** |
| Tauri commands | `test_conn`, `list_databases_cmd`, `list_tables_cmd`, `run_query_cmd`, `backup`, `restore`, `copy_db`, `store_password`, `get_password`, `delete_password` |

There is **no** connection CRUD command. The webview is the only writer of profiles.

### 1.2 CLI (today)

| Concern | Implementation |
|---|---|
| Store | `ConnectionStore` at `dirs::config_dir()/dbpinball/connections.json` |
| File shape | `StoreFile { version: 1, migratedFrom?: string, connections: DbConnection[] }` |
| Profile shape | `DbConnection` with `target: TargetClass` (`#[serde(default)]` → `unknown`) |
| Write | temp file + `fs::rename`, `0600` on Unix (`connection_store.rs:74-138`) |
| Import | `ConnectionStore::import` validates, dedupes by `same_identity`, returns `ImportReport` |
| Keyring | `KeyringCredentialProvider`, service `"dbpinball"`, account = connection id |
| Env credentials | `EnvCredentialProvider` reads `DBPINBALL_PASSWORD`; cannot store or delete |

### 1.3 What already lines up

- **Keyring identifiers are already identical.** Desktop `secrets.rs` and core `adapters/keyring.rs` both use service `"dbpinball"` and account = connection id. Migration must not touch them.
- **The Tauri layer already uses the core type.** `src-tauri/src/models.rs` re-exports `DbConnection` from `dbpinball-core`.
- **`target` defaults to `unknown`.** A desktop record without `target` deserializes to `TargetClass::Unknown`, which is exactly the required import value.
- **`migratedFrom` survives later writes.** `connection_store.rs:380-396` proves it.

### 1.4 What does not line up

| Gap | Consequence |
|---|---|
| No Tauri connection CRUD/import commands | Desktop cannot use the shared store |
| Desktop writes `localStorage` directly | Two sources of truth |
| `ConnectionStore` has no cross-process lock | Desktop and CLI can lose each other's updates (read-modify-write, last writer wins) |
| `import` does not set `migratedFrom` or `target` | Caller must; easy to forget |
| No migration report surface | User cannot see what was skipped |
| No idempotency marker check in the desktop | Re-running migration is only safe by dedupe, not by marker |
| Desktop `DbConnection` type lacks `target` | UI cannot show the imported classification |

## 2. Target architecture

```
Desktop (React)                CLI (dbpinball)
      │                              │
      │ invoke(...)                  │ commands::connections
      ▼                              ▼
Tauri adapter (thin)          ──►  dbpinball-core
      │                              │
      └──────────► ConnectionStore ◄─┘
                   (single source of truth)
                        │
                        ├── connections.json  (versioned, atomic, locked)
                        └── OS keyring        (service "dbpinball", account = id)
```

Rules:

1. **`ConnectionStore` is the only profile store.** Desktop and CLI both read and write it.
2. **Desktop never writes `localStorage` after migration.** It may read it once, for migration and rollback.
3. **No silent fallback.** If the shared store is unavailable, the desktop reports the error and does not fall back to `localStorage` for ongoing writes.
4. **Tauri commands are thin adapters.** They validate input, call core, and map `CoreError` to a safe message. No persistence logic in the adapter.
5. **Existing Tauri commands stay unchanged** (`test_conn`, `list_databases_cmd`, `list_tables_cmd`, `run_query_cmd`, `backup`, `restore`, `copy_db`, `store_password`, `get_password`, `delete_password`).

## 3. Migration state machine

```
idle
  └─(desktop launch, marker absent)→ reading
        └─(localStorage read ok)→ validating
              └─(records parsed)→ importing
                    └─(store write ok)→ done
                    └─(store write failed)→ failed (localStorage untouched)
              └─(parse failed)→ failed (localStorage untouched)
        └─(localStorage absent)→ done (nothing to migrate)
  └─(marker present)→ done (idempotent short-circuit)
```

States and transitions:

| State | Entry condition | Exit |
|---|---|---|
| `idle` | no `migratedFrom` marker | `reading` |
| `reading` | — | `validating` or `done` (no key) or `failed` |
| `validating` | raw JSON parsed | `importing` or `failed` |
| `importing` | records validated | `done` or `partial` or `failed` |
| `done` | store written, marker set | terminal |
| `partial` | some records imported, some skipped | terminal, report shown |
| `failed` | store write or parse failed | terminal, localStorage untouched |

Idempotency:

- The store's `migratedFrom` marker short-circuits a second run.
- Even without the marker, `import` dedupes by `same_identity`, so a re-run imports nothing new.
- The marker is written **only after** a successful store write.

## 4. Data conflict rules

| Case | Rule |
|---|---|
| Imported record matches an existing shared record by `same_identity` | **Skip.** Preserve the shared record. Count as `skippedDuplicates`. |
| Imported record has the same `id` but a different identity | **Conflict.** Preserve the shared record. Count as `skippedConflicts` (new report field). Never overwrite. |
| Imported record is invalid (empty id/name/host/database/username, port 0) | **Skip.** Count as `skippedInvalid` with the reason only. |
| Imported record has no `target` | Set `TargetClass::Unknown`. |
| Shared store already has profiles | Preserve all of them. Import only adds. |
| Shared store is newer than the import | The shared record wins; the import never overwrites. |

Deterministic dedupe key: `(engine, host, port, database, username)` — the existing `same_identity`. Documented and stable.

`migratedFrom` is set to `"localStorage@v1"` on the first successful migration and preserved thereafter.

## 5. Tauri command contracts

All commands return `Result<T, String>` with a **safe** message (no host, user, password or path). Errors carry a stable code prefix.

| Command | Input | Output |
|---|---|---|
| `connections_list` | — | `DbConnection[]` |
| `connections_create` | `connection: DbConnection`, `password?: string` | `DbConnection` |
| `connections_update` | `connection: DbConnection`, `password?: string` | `DbConnection` |
| `connections_delete` | `id: string` | `{ removed: boolean }` |
| `connections_import` | `records: DbConnection[]` | `ImportReport` |
| `connections_migration_status` | — | `{ state, migratedFrom, imported, skippedInvalid, skippedDuplicates, skippedConflicts }` |
| `connections_migrate_local_storage` | `records: DbConnection[]` | `ImportReport` + marker |

Behaviour:

- `connections_create` stores the password in the keyring **before** writing the profile; if the profile write fails, the credential is removed (mirrors the CLI).
- `connections_delete` deletes the credential first, then the profile (mirrors the CLI).
- `connections_update` writes the profile, then the credential if a new password was supplied.
- `connections_import` calls `ConnectionStore::import` and sets `migratedFrom` when the caller is the migration path.
- `connections_migration_status` reads the marker and the last report; it never mutates.

## 6. Credential handling

- **Service and account are unchanged:** `"dbpinball"` / connection id. Existing keyring entries keep working.
- **No password ever enters `ConnectionStore` or `localStorage`.** A test already asserts the store never contains `"password"`.
- **Migration never regenerates credentials.** It imports metadata only.
- **Missing credential** is not a migration failure: the profile is imported and the UI marks it "password required".
- **Keyring unavailable / permission denied** is reported as a safe error; the profile is still imported, and the UI offers "set password".
- **The CLI environment provider is not a persistent replacement.** `DBPINBALL_PASSWORD` is per-invocation and cannot store or delete. Document this in `docs/cli.md` and the migration notes.

## 7. Concurrency protection

`ConnectionStore` currently does read-modify-write with an atomic rename but no lock. Two writers can lose an update.

Design:

1. **Cross-process lock file** at `<config>/dbpinball/connections.lock`, created with `OpenOptions::new().create_new(true)`.
2. **Bounded wait:** 2 s, 20 ms poll, then `ErrorCode::Operation` with a "store is busy" message.
3. **Stale recovery:** the lock records `{ pid, host, startedAt }`. A lock is recoverable only when the owner is **confirmed dead** (same host, PID not alive). Unknown liveness (other host, unreadable record) is treated as live — fail closed.
4. **Atomic recovery:** recovering a stale lock requires winning an atomic rename of the lock file, so two processes cannot both recover it.
5. **All mutating operations** (`add`, `remove`, `import`, `save`) take the lock. Reads do not.
6. **Optimistic revision:** `StoreFile` gains `revision: u64` (default 0). A writer that read revision N and finds N+1 refuses with a conflict instead of overwriting. This catches a lost update even if the lock is bypassed.
7. **Interrupted operation:** the temp file is removed on failure; the lock is released in a guard; a leftover lock is recovered by the liveness rule.

No in-process-only mutex is used as the correctness boundary.

## 8. Test matrix

| # | Case | Layer |
|---|---|---|
| 1 | Fresh desktop install, empty store | Tauri + core |
| 2 | Legacy `localStorage` migration, all valid | Tauri + core |
| 3 | Duplicate records within the legacy array | core `import` |
| 4 | Invalid legacy data (empty host, port 0, bad JSON) | core + UI |
| 5 | Conflicting CLI/Desktop profile (same id, different identity) | core |
| 6 | Repeated migration (marker present) | Tauri |
| 7 | Migration interruption (write fails mid-way) | core |
| 8 | Store write failure (read-only dir) | core |
| 9 | Keyring failure (unavailable, permission denied) | Tauri |
| 10 | Cross-process concurrent updates (two processes) | core integration |
| 11 | CLI/desktop profile consistency (same file) | integration |
| 12 | Windows / macOS / Linux config path resolution | core unit |
| 13 | Existing Tauri command compatibility | Tauri |
| 14 | No password in store or localStorage | core + UI |
| 15 | `migratedFrom` preserved after later writes | core |

Synthetic data only. No real credentials, no real hosts.

## 9. Rollback and recovery

| Scenario | Recovery |
|---|---|
| Migration produced a bad store | Delete `connections.json`; the desktop falls back to reading `localStorage` (still intact) |
| Partial migration | Re-run migration; dedupe makes it safe; the report shows what remains |
| Store write failed | `localStorage` untouched; the desktop keeps working from it until the store is writable |
| Keyring entry missing | Profile remains; the UI asks for the password |
| User wants the old behaviour | `localStorage` is never deleted in M2.1; it is a one-release read-only fallback |
| CLI profiles at risk | The migration only adds; it never deletes or overwrites shared records |

A pre-migration snapshot `connections.json.bak` is written before the first migration write.

## 10. Milestone acceptance criteria

- [ ] `ConnectionStore` is the only profile store for Desktop and CLI.
- [ ] Desktop reads, creates, updates and deletes profiles through Tauri commands backed by core.
- [ ] No `localStorage` write after migration; `localStorage` is read-only for one release.
- [ ] Migration is idempotent and reports imported / invalid / duplicate / conflict counts.
- [ ] `migratedFrom = "localStorage@v1"` is set once and preserved.
- [ ] Imported records have `target: unknown`.
- [ ] Keyring service and account identifiers are unchanged.
- [ ] No password is written to the store or `localStorage`.
- [ ] Cross-process lock with bounded wait and liveness-based stale recovery.
- [ ] Optimistic revision prevents lost updates.
- [ ] Existing Tauri commands still work.
- [ ] Test matrix passes on Windows, macOS and Linux where practical.
- [ ] Rollback works without destroying `localStorage`, CLI profiles or keyring entries.

## 11. Remaining architectural decisions

1. **Lock implementation:** dedicated lock file (recommended, no new dependency) vs `fd-lock`/`fs2` crate.
2. **Config directory:** `dirs::config_dir()/dbpinball` (recommended, matches the CLI) vs Tauri's app config dir (would diverge from the CLI).
3. **Migration trigger:** automatic on first launch (recommended, with a report) vs an explicit user action.
4. **`target` in the desktop type:** add now (recommended, so the UI can show it) vs later.
5. **`migratedFrom` ownership:** set inside `ConnectionStore::import` (recommended, one place) vs the caller.
6. **Conflict reporting:** extend `ImportReport` with `skippedConflicts` (recommended) vs a separate report type.
