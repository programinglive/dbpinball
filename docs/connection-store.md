# Connection store: architecture, migration and rollback

Status: M2.1. The desktop and the CLI share one authoritative store.

## 1. One store, two front ends

```
Desktop (React)                CLI (dbpinball)
      │                              │
      │ invoke(...)                  │ connections ...
      ▼                              ▼
Tauri adapter (thin)          ──►  dbpinball-core
      │                              │
      └──────────► ConnectionStore ◄─┘
                   (single source of truth)
                        │
                        ├── connections.json  (versioned, atomic, locked)
                        └── OS keyring        (service "dbpinball", account = id)
```

| Concern | Owner |
|---|---|
| Store path | `dbpinball_core::store::default_config_dir()` |
| Schema | `dbpinball_core::store::StoreFile` |
| Validation | `dbpinball_core::store::connection_store` |
| Identity and dedupe | `dbpinball_core::store::connection_store` |
| Concurrency | `dbpinball_core::store::lock` |
| Credentials | `dbpinball_core::adapters::keyring` |

The Tauri adapter (`src-tauri/src/connections.rs`) contains no persistence logic.
It resolves the shared directory, calls Core, and maps errors to a safe message.

## 2. Store location

`dirs::config_dir()/dbpinball/connections.json`

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\dbpinball\connections.json` |
| macOS | `~/Library/Application Support/dbpinball/connections.json` |
| Linux | `~/.config/dbpinball/connections.json` |

The CLI `--config-dir` override exists for tests and advanced use. The desktop
always uses the default, so the two cannot drift.

## 3. File format

```json
{
  "version": 1,
  "migratedFrom": "localStorage@v1",
  "revision": 7,
  "connections": [
    {
      "id": "0f0c2f7e-...",
      "name": "Analytics",
      "engine": "postgres",
      "host": "analytics.example.com",
      "port": 5432,
      "database": "analytics",
      "username": "analyst",
      "useSsl": true,
      "target": "unknown"
    }
  ]
}
```

- `version` is the only supported schema version.
- `migratedFrom` records provenance. It is written once and preserved.
- `revision` is a monotonic counter used for optimistic concurrency.
- `target` is a user hint, never verified evidence.
- **No password field exists.** A test asserts the key is absent.

## 4. Concurrency

Every mutation (`create`, `add`, `update`, `remove`, `import`, `save`) takes a
cross-process lock at `<config>/dbpinball/connections.lock`.

| Property | Behaviour |
|---|---|
| Exclusivity | `OpenOptions::create_new` — the OS guarantees one creator |
| Bounded wait | 2 s, 20 ms poll, then `ErrorCode::Busy` (CLI exit 9) |
| Stale recovery | Only a **confirmed-dead** owner is recovered |
| Live owner | Never displaced, regardless of how old the record looks |
| Unknown liveness | Treated as live (fail closed) |
| Concurrent recovery | A recovery mutex makes "re-check, then remove" one critical section, so two processes cannot both recover the same stale lock |
| Atomic write | temp file + `fs::rename`, `0600` on Unix |
| Optimistic revision | A stale `expectedRevision` returns `ErrorCode::Conflict` (CLI exit 8) |

Liveness is checked with `ps -p <pid>` on Unix and `tasklist` on Windows. `kill
-0` is deliberately not used: it exits non-zero for both "no such process" and
"not permitted", so it cannot prove death and would let a live owner be
displaced.

The lock is an operating-system file lock, not an in-process mutex. The
cross-process tests spawn the real `dbpinball` binary.

## 5. Migration from localStorage

The desktop migrates automatically on first launch.

```
NOT_STARTED → VALIDATING → IMPORTING → VERIFYING → COMPLETED
                                   ↘ RECOVERABLE_ERROR
```

| Step | Behaviour |
|---|---|
| Read | `localStorage["dbpinball_connections"]` |
| Validate | Empty id/name/host/database/username or port 0 is skipped with a reason |
| Preserve ids | A valid legacy id is kept unchanged |
| Deduplicate | `(engine, host, port, database, username)` |
| Conflicts | Same id with different details → skipped, shared record preserved |
| Target | Imported records are set to `unknown` |
| Provenance | `migratedFrom = "localStorage@v1"`, written only after a successful store write |
| Verify | The store is re-read after the write |
| Idempotent | The marker short-circuits; dedupe makes a re-run safe |
| localStorage | **Never deleted and never written again** |

The migration report shows imported, duplicate, invalid and conflicting counts.
The report is **persisted in the store** (`migrationReport`), so unresolved
conflicts and skipped records survive a restart instead of disappearing with the
UI banner. `connections_migration_status` returns `complete: false` when any
record was skipped or conflicted, and the desktop keeps a warning visible until
the user resolves it.

### Credentials

- The keyring service (`dbpinball`) and account (connection id) are unchanged.
- Migration imports metadata only. It never reads, writes or regenerates a password.
- A missing credential is not a migration failure: the profile is imported and the
  UI asks for the password.
- The CLI environment provider (`DBPINBALL_PASSWORD`) is **not** a persistent
  replacement for desktop keyring credentials. It is per-invocation and cannot
  store or delete.

## 6. Rollback

| Scenario | Recovery |
|---|---|
| Bad migration result | Delete `connections.json`; the desktop reads `localStorage` again |
| Partial migration | Re-run; dedupe makes it safe; the report shows what remains |
| Store write failed | `localStorage` is untouched; the desktop keeps working from it |
| Keyring entry missing | The profile remains; set the password again |
| Want the old behaviour | `localStorage` is a one-release read-only fallback |

Rollback never deletes `localStorage`, never deletes CLI profiles, and never
touches keyring entries.

## 7. Platform support

The store path and the lock are implemented for Windows, macOS and Linux. The
cross-process tests run on the platform that executes CI (Linux) and on the
developer machine. A platform that has not been exercised is not claimed as
verified.

## 8. What M2.1 does not do

- No security policy or destination verification (M3).
- No safe sync (M4).
- No credential migration between keyring backends.
