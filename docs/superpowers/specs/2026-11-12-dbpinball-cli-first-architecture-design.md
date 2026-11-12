# DBPinball CLI-First Architecture — Design Spec (Revision 3)

Date: 2026-11-12
Status: **Draft for owner review — not approved for implementation**
Supersedes: Revision 2 (same path)
Audit baseline: `docs/ARCHITECTURE-AUDIT.md`, commit `82a6a52`

---

## 0. Changelog — what was corrected and why

| # | Finding | Correction in this revision |
| --- | --- | --- |
| 1 | `--confirm` was treated as authorization for production/unknown writes | Removed. Confirmation is now an acknowledgement, never an authorization. Non-local writes require identity verification, an explicit config opt-in, a typed connection-name match, and an audit record. Safe Sync has **no** override at all. |
| 2 | Safe Sync was conflated with raw backup/restore/copy | Split into two distinct capability families with separate services, separate commands, and separate permission rows. Raw PostgreSQL dump/restore is explicitly **not** described as sanitized synchronization. |
| 3 | Target classification was treated as proof of destination identity | Classification is now a user hint only. A `DestinationVerifier` resolves the live server identity, checks loopback, and probes writability. A connection marked `local` that points at a remote host is refused. |
| 4 | Read-only SQL keyword checking was implied to be a security boundary | Demoted to UX validation. The security boundary is a database-enforced read-only session, with a test that proves a write fails at the server. |
| 5 | Migration fixed all runners and unrelated behavior in one milestone | M1 now extracts one capability at a time behind the existing Tauri commands. `runners/` is not deleted until every replacement passes integration and regression tests. |
| 6 | Connection store used a home-directory path and ignored failure modes | Uses OS config/data directories, atomic writes, idempotent migration, duplicate and invalid-record handling, and explicit keyring-failure behavior. |
| 7 | Milestones did not match the required boundaries | Replaced with M0–M7 exactly as specified, each with prerequisites, security acceptance criteria, integration tests, regression tests, and rollback. |
| 8 | Security claims had no enforcement mechanism or test | Every security property now has a table row naming the mechanism and the test that proves it (§4.6). |
| 9 | Working tree was not inspected | Inspected and documented in §13. The release tooling was migrated to `commiter` 1.5.1 at the owner's explicit request; the previous local script is backed up, not discarded. |
| 10 | `--confirm` still gated non-local writes as a single flag | Replaced with risk tiers T0–T3 (§4.5). Read and remote backup need no acknowledgement; non-local writes need opt-in, identity verification, a short-lived plan, and an audit entry; production adds an operation-specific acknowledgement bound into the plan. |
| 11 | HMAC was presented as the whole masking strategy | §9.4 now defines canonical metadata-derived identity mapping, type and constraint preservation, collision prevention and detection, composite keys, synthetic PII, non-authenticating secret replacement, JSON/free-text policies, key lifecycle, and pre-promotion validation. Pseudonymization is explicitly not anonymization. |
| 12 | Loopback was treated as proof of locality | §4.4 now defines the verified development target (socket, loopback instance, Docker network, authorized remote dev) and refuses undeclared tunnels. Engine-specific identity strategies document privilege requirements and fail closed on low confidence. |
| 13 | Commiter migration was not reviewed against the original modification | §13.2 compares the backup with the 1.5.1 engine, records that recovery is not needed, and makes no further release-process change. |

---

## 1. Context

DBPinball is a Tauri 2 + React 19 database workspace for MySQL and PostgreSQL. The audit (`docs/ARCHITECTURE-AUDIT.md`) established:

- All database logic lives in `src-tauri/src/runners/*`, which imports `tauri::Window` (`mysql.rs:11`, `postgres.rs:10`). Nothing runs headless.
- There is no CLI, no application-service layer, no safety policy, and zero Rust tests (`cargo test` → 0 tests).
- Five confirmed defects: stub internal dumper, ignored `drop_existing`, unrestricted `run_query`, unused `use_ssl`, `include_views` mapped to `--events`.

This revision defines the minimum refactor that makes the core headless, makes a `dbpinball` CLI the primary contract for new features, and keeps the desktop app working throughout.

## 2. Goals

1. A `dbpinball-core` library that compiles without Tauri, without a CLI parser, and without terminal I/O.
2. A `dbpinball` CLI that exposes every capability with stable JSON output and predictable exit codes.
3. One centralized authorization path that no interface can bypass.
4. The desktop app becomes a thin adapter over the same services.
5. Safe Sync as a separate, fail-closed capability that cannot write outside a verified development target.

## 3. Non-goals

- Cross-engine copy (MySQL ↔ PostgreSQL).
- SSH tunnels, scheduled backups, cloud storage.
- Rewriting the React UI beyond the connection data source.
- New database engines.
- Implementing the internal dumper (M1 returns `Unsupported`; real work is M7).

---

## 4. Security model

### 4.1 Threat model

| Asset | Threat | Consequence |
| --- | --- | --- |
| Production database contents | Operator selects the wrong connection | Unintended write or destructive restore |
| Production database contents | Stale connection metadata (host changed, DNS, tunnel) | Writes land on a different server than the label says |
| Production database contents | Ad-hoc SQL typed into the workspace | `DROP`/`UPDATE` executes |
| Credentials | Password in argv, logs, error strings, serialized metadata | Credential disclosure |
| Local machine | Malicious or corrupted config file | Redirected destination, injected connection |
| Snapshot artifacts | Unmasked sensitive data in a debugging snapshot | Data exfiltration |

### 4.2 Trust boundaries

```text
┌──────────────────────────────────────────────────────────────┐
│ Trusted process: dbpinball CLI / desktop backend             │
│   - owns SafetyPolicy, DestinationVerifier, MaskingEngine    │
└───────────────┬──────────────────────────┬───────────────────┘
                │                          │
     untrusted data                 trusted store
                │                          │
   ┌────────────▼───────────┐   ┌──────────▼─────────────┐
   │ connections.json       │   │ OS keyring             │
   │ (validated on read)    │   │ (passwords only)       │
   └────────────────────────┘   └────────────────────────┘
                │
   ┌────────────▼───────────┐   ┌────────────────────────┐
   │ Source database        │   │ Destination database   │
   │ (untrusted, read-only) │   │ (must be verified)     │
   └────────────────────────┘   └────────────────────────┘
```

The config file is **untrusted input**. Every record is validated on read; a malformed record is rejected, never coerced.

### 4.3 Two capability families

These are separate services, separate commands, and separate permission rows. They must never share a code path that could let one inherit the other's guarantees.

| Family | Purpose | Writes to | Sanitization |
| --- | --- | --- | --- |
| **Administrative** - `backup`, `restore`, `copy`, `query` | Raw operational tooling. Byte-faithful dumps. | Any target, under the risk tiers (§4.5) | None. Raw data. |
| **Safe Sync** - `safesync plan`, `safesync run` | Production → local debugging copy | **Verified development target only** | Mandatory deterministic masking |

**Raw PostgreSQL backup/restore/copy is not sanitized synchronization and must never be documented or presented as such.** The CLI help text, README, and UI copy must state this explicitly.

### 4.4 Destination verification and the verified development target

`TargetClass` is a **user hint**, not proof. Loopback is not proof either: an SSH tunnel to a production database binds to `127.0.0.1` and is indistinguishable from a local server by address alone.

A destination is accepted for Safe Sync only when it is a **verified development target**: a record that binds a destination kind, a live server identity, and the evidence used to establish it.

```rust
pub enum DevelopmentTargetKind {
    LocalSocket,          // Unix socket or named pipe
    LoopbackInstance,     // server bound to 127.0.0.1/::1, identity matches a local instance
    DockerNetwork,        // container network, identity matches a declared container
    AuthorizedRemoteDev,  // explicitly allow-listed remote dev environment, time-bounded
}

pub struct VerifiedDevelopmentTarget {
    pub kind: DevelopmentTargetKind,
    pub server_identity: ServerIdentity,
    pub database: String,
    pub writable_probe: bool,
    pub evidence: VerificationEvidence,
    pub verified_at: Instant,
    pub expires_at: Option<Instant>,   // required for AuthorizedRemoteDev
}
```

Verification steps, in order:

1. **Resolve identity** using the engine-specific strategy in §4.4.1.
2. **Classify the destination kind** from the connection record and the live evidence.
3. **Reject undeclared tunnels.** If the resolved address is loopback but the server identity does not match a known local instance, the connection is treated as a tunnel. An undeclared tunnel is refused with `ErrorCode::DestinationUnverified`. A declared tunnel must name the expected remote identity and is accepted only as `AuthorizedRemoteDev`.
4. **Compare fingerprint** to the identity recorded when the target was authorized. A mismatch is `ErrorCode::DestinationMismatch`.
5. **Probe writability** (`BEGIN; CREATE TEMPORARY TABLE dbpinball_probe(x int); ROLLBACK;`). This is a capability signal, not a guarantee: a role may create temp tables yet lack `INSERT` on the target tables. The database's own permission checks remain the final bound, and a permission failure surfaces as `ErrorCode::Operation` with the server's message.
6. **Revalidate** immediately before the first write and after any phase longer than 60 s. A destination that changes identity mid-run aborts the operation.

#### 4.4.1 Engine-specific identity strategies

| Engine | Identity query | Privilege required | Fail-closed behavior |
| --- | --- | --- | --- |
| PostgreSQL | `SELECT system_identifier FROM pg_control_system()` plus `inet_server_addr()`, `inet_server_port()`, `current_database()`, `pg_postmaster_start_time()` | `pg_control_system()` requires superuser or a role granted `EXECUTE` on it | If the function is not permitted, identity confidence is `Low`; Safe Sync refuses unless the target is `LocalSocket` or `DockerNetwork` with a matching declared identity |
| MySQL | `SELECT @@server_uuid, @@hostname, @@port, @@datadir` | none (readable by any connected user) | If `@@server_uuid` is unavailable (pre-5.7), identity confidence is `Low`; Safe Sync refuses |

`ServerIdentity` carries `confidence: High | Low`. Safe Sync requires `High` unless the target kind is `LocalSocket` or `DockerNetwork` and the declared identity matches.

PostgreSQL `inet_server_addr()` returns `NULL` for a socket connection; `NULL` is treated as `LocalSocket` only when the connection host is a socket path or `localhost`.

#### 4.4.2 Declaring a development target

```bash
dbpinball targets declare --connection <id> --kind <kind> \
    [--expected-identity <fingerprint>] [--expires <duration>]
```

- `LocalSocket` and `LoopbackInstance` are declared from live evidence.
- `DockerNetwork` requires the container name or network alias to be recorded.
- `AuthorizedRemoteDev` requires an explicit expected identity and a TTL (default 8 h, maximum 24 h). It is the only kind that may be non-loopback.

A target declaration is stored in the config file and revalidated on every use.

### 4.5 Risk-based authorization

Authorization is decided by risk tier, not by a single confirmation flag. A confirmation flag is an acknowledgement; it never grants permission.

| Tier | Operations | Requirements |
| --- | --- | --- |
| **T0 — Read** | `inspect`, `query --read-only`, `backup` (remote or local), `safesync plan` | None. No destructive-write acknowledgement. |
| **T1 — Local write** | `query --write`, `restore`, `copy` to a verified local target | Destination verification (§4.4). No acknowledgement. |
| **T2 — Non-local write** | `restore`, `copy`, `query --write` to staging or unknown | Config opt-in `allow_non_local_writes = true`, verified destination identity, short-lived execution plan, audit log entry. |
| **T3 — Production write** | The same operations against a production target | Everything in T2 **plus** an operation-specific confirmation bound into the execution plan. |
| **Safe Sync** | `safesync run` | Verified development target only. No override exists at any tier. |

Matrix:

| Operation | Verified local | Staging | Production | Unknown |
| --- | --- | --- | --- | --- |
| `inspect` | allow | allow | allow | allow |
| `query --read-only` | allow | allow | allow | allow |
| `backup` (reads source) | allow | allow | allow | allow |
| `safesync plan` (reads source) | allow | allow | allow | allow |
| `query --write` | allow | T2 | T3 | T2 |
| `restore` | allow | T2 | T3 | T2 |
| `copy` | allow | T2 | T3 | T2 |
| `safesync run` | allow if verified | **refuse** | **refuse** | **refuse** |

`Unknown` is treated as T2, not T3: it is a classification gap, not a confirmed production target. It still requires opt-in, identity verification, a plan, and an audit entry.

#### 4.5.1 Short-lived validated execution plans

High-risk writes do not execute from ad-hoc flags. They execute from a plan bound to the exact operation:

```rust
pub struct ExecutionPlan {
    pub id: String,
    pub operation: Operation,
    pub destination_identity: ServerIdentity,
    pub database: String,
    pub input_artifact_hash: Option<String>,  // e.g. the dump file
    pub created_at: Instant,
    pub expires_at: Instant,                  // default 10 minutes
    pub single_use: bool,
    pub acknowledgement: Option<String>,      // T3 only
}
```

`plan run` revalidates every binding before executing: identity unchanged, database unchanged, artifact hash unchanged, not expired, not already used. Any mismatch is `ErrorCode::PlanInvalid`.

#### 4.5.2 Noninteractive execution

`--non-interactive` (implied when stdin is not a TTY) never bypasses a tier. A missing opt-in, plan, or acknowledgement is a `policy_refusal` — never a prompt, never a silent allow.

### 4.6 Enforcement mechanisms and their tests

No security property is claimed unless the mechanism and its test are named.

**None of these tests exist yet.** Each is a required deliverable of the milestone that introduces its mechanism, and the milestone cannot close until the test is green. The table is a commitment, not a status report.

| Property | Enforcement mechanism | Test |
| --- | --- | --- |
| Safe Sync never writes non-local | `SafetyPolicy::authorize_safesync` accepts only a `VerifiedDevelopmentTarget`; no override exists | `policy::tests::safesync_refuses_production`, `policy::tests::safesync_refuses_unknown`, `policy::tests::safesync_has_no_override` |
| Classification is not trusted | `DestinationVerifier` compares live server identity to the recorded fingerprint | `verifier::tests::identity_mismatch_refused` |
| Loopback is not proof of locality | Undeclared tunnel detection: loopback address with a non-local identity is refused | `verifier::tests::undeclared_tunnel_refused`, `verifier::tests::declared_tunnel_requires_expected_identity` |
| Docker and socket targets are identified | `DevelopmentTargetKind` matched against declared container/socket evidence | `verifier::tests::docker_target_requires_declared_container`, `verifier::tests::socket_target_accepted` |
| Low-confidence identity fails closed | `ServerIdentity.confidence == Low` refuses Safe Sync unless kind is socket/docker with a match | `verifier::tests::low_confidence_identity_refused` |
| Destination is writable | `CREATE TEMPORARY TABLE` probe inside a rolled-back transaction | `verifier::tests::read_only_destination_refused` |
| High-risk writes are plan-bound | `ExecutionPlan` binds identity, database, artifact hash, operation, expiry, single use | `plan::tests::identity_change_invalidates_plan`, `plan::tests::expired_plan_refused`, `plan::tests::artifact_hash_mismatch_refused`, `plan::tests::plan_is_single_use` |
| Noninteractive does not bypass policy | `--non-interactive` returns `policy_refusal` instead of prompting | `cli_contract::tests::noninteractive_missing_plan_refused` |
| Source is read-only | PostgreSQL `BEGIN TRANSACTION READ ONLY` + `default_transaction_read_only=on`; MySQL `SET SESSION TRANSACTION READ ONLY` | `safesync::tests::source_write_rejected_by_server` (expects SQLSTATE 25006 / MySQL 1792) |
| Keyword check is not the boundary | Documented as UX only; the DB session is the boundary | `query::tests::volatile_function_write_fails_in_read_only_session` |
| No password in argv | `connections add` reads the password from stdin or the keyring; no `--password` flag exists | `cli_contract::tests::no_password_flag_accepted` |
| No password in logs or errors | `CoreError` redaction; connection URLs never formatted into messages | `error::tests::redaction_removes_password`, `error::tests::url_never_in_message` |
| No password in serialized metadata | `ConnectionRecord` has no password field; serialization test asserts the key is absent | `store::tests::serialized_record_has_no_secret` |
| Masking is deterministic and join-preserving | Canonical metadata-derived identity mapping: FK columns resolve to the referenced PK's canonical identity | `masking::tests::same_input_same_output`, `masking::tests::fk_matches_pk`, `masking::tests::composite_key_components_consistent` |
| Identifier collisions are prevented and detected | Deterministic collision resolution plus unique-constraint detection at load | `masking::tests::collision_resolved_deterministically`, `load::tests::unique_violation_detected` |
| Types and constraints are preserved | Generated values validated against column type, length, nullability, enum domain, and check constraints | `masking::tests::type_and_length_preserved`, `masking::tests::enum_domain_respected` |
| PII is synthesized, not just hashed | Deterministic synthetic generators for email, name, phone, address | `masking::tests::email_is_synthetic_and_valid`, `masking::tests::phone_matches_locale_format` |
| Secrets are non-authenticating | Secrets nullified or replaced with a fixed non-authenticating marker | `masking::tests::secret_replaced_with_marker` |
| JSON and free text follow explicit policy | Per-column policy: `json:redact-keys`, `json:mask-values`, `text:redact-patterns`, `text:passthrough` | `masking::tests::json_unknown_key_refused`, `masking::tests::text_policy_applied` |
| Unknown sensitive data fails closed | Unclassified column in an extracted table → `ErrorCode::UnclassifiedColumn` | `masking::tests::unclassified_column_refused` |
| Masking key lifecycle is defined | CSPRNG key, keyring storage, rotation, destruction after validation | `masking::tests::key_never_serialized`, `masking::tests::key_destroyed_after_validation` |
| Snapshot contains no residual secrets | Post-write scan for sampled source values and sensitive patterns | `snapshot::tests::residual_secret_detected` |
| Referential integrity holds | Per-FK orphan count must be zero | `safesync::tests::no_orphan_rows` |
| Core cannot depend on Tauri or clap | CI runs `cargo tree -p dbpinball-core` and fails on `tauri`/`clap` | `scripts/check-boundaries.cjs` in CI |
| Cancellation terminates children | Process-group kill on cancel | `process::tests::cancel_kills_child_group` |

### 4.7 Read-only SQL checking is UX only

`QueryMode::ReadOnly` rejects statements whose first keyword is not `SELECT`, `SHOW`, `EXPLAIN`, `DESCRIBE`, or `WITH`. This exists to catch mistakes early and produce a clear message. It is **not** a security boundary: it can be bypassed by a `SELECT` that calls a volatile function, a CTE that writes, or a stored procedure.

The actual boundary is the database session:

- PostgreSQL: `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY` and `SET default_transaction_read_only = on` on the source connection, plus `BEGIN TRANSACTION READ ONLY` around extraction.
- MySQL: `SET SESSION TRANSACTION READ ONLY` on the source connection.

The test proves the server rejects a write even when the keyword check is bypassed.

### 4.8 Least-privilege source credentials

Safe Sync requires a source role with only:

```sql
GRANT CONNECT ON DATABASE <db> TO dbpinball_reader;
GRANT USAGE ON SCHEMA public TO dbpinball_reader;
GRANT SELECT ON <each extracted table> TO dbpinball_reader;
```

No `SUPER`, no `REPLICATION`, no `CREATEDB`, no write grants. `dbpinball safesync grants --source <id>` prints the exact statements for the selected tables. The CLI warns when the connected role has `SUPER`/`REPLICATION` or write privileges on the source.

### 4.9 Authorization lives in services

`SafetyPolicy` is a field of `ServiceContext`. Every write service takes `&ServiceContext` and calls `authorize` before any I/O. There is no public service function that performs a write without a `ServiceContext`. The CLI and the Tauri adapter both construct a `ServiceContext`; neither can construct one that skips the policy, because the constructor is `pub(crate)` and the only public builders are `ServiceContext::for_cli(...)` and `ServiceContext::for_desktop(...)`, both of which install the policy.

---

## 5. Architecture

### 5.1 Dependency direction

```text
                    dbpinball-core
        ┌──────────────────────────────────┐
        │  domain/   models, errors, policy │
        │  ports/    traits (no I/O impl)   │
        │  services/ admin + safesync       │
        └──────────────────────────────────┘
                 ▲                  ▲
                 │                  │
      dbpinball-cli          src-tauri (desktop)
      (clap, JSON,           (tauri commands,
       exit codes)            window.emit adapter)
```

`dbpinball-core` must not list `tauri` or `clap` in its dependencies. This is enforced in CI, not by convention.

### 5.2 Workspace layout

This is the **end-state** layout. M1 creates only `domain/`, `ports/`, and `services/inspect.rs`; the remaining service modules arrive in the milestone that owns them (§11).

```text
dbpinball/
├── Cargo.toml                       # [workspace] members = crates/*, src-tauri
├── crates/
│   ├── dbpinball-core/
│   │   └── src/
│   │       ├── domain/
│   │       │   ├── connection.rs    # DbConnection, DbEngine, TargetClass, ServerIdentity
│   │       │   ├── options.rs
│   │       │   ├── result.rs
│   │       │   ├── error.rs         # CoreError, ErrorCode, redaction
│   │       │   └── policy.rs        # SafetyPolicy, Operation, AdminOverride
│   │       ├── ports/
│   │       │   ├── progress.rs
│   │       │   ├── credentials.rs
│   │       │   ├── artifacts.rs
│   │       │   └── cancellation.rs
│   │       └── services/
│   │           ├── inspect.rs
│   │           ├── query.rs
│   │           ├── backup.rs        # admin, raw
│   │           ├── restore.rs       # admin, raw
│   │           ├── copy.rs          # admin, raw
│   │           ├── verify.rs        # DestinationVerifier
│   │           └── safesync/
│   │               ├── mod.rs
│   │               ├── plan.rs
│   │               ├── extract.rs
│   │               ├── mask.rs
│   │               ├── load.rs
│   │               └── validate.rs
│   └── dbpinball-cli/
│       └── src/
│           ├── main.rs
│           ├── output.rs
│           └── commands/
└── src-tauri/                       # desktop adapter
```

### 5.3 Ports

```rust
pub trait ProgressSink: Send + Sync { fn emit(&self, event: ProgressEvent); }
pub trait CredentialProvider: Send + Sync {
    fn get(&self, id: &str) -> Result<Secret, CoreError>;
    fn set(&self, id: &str, secret: &Secret) -> Result<(), CoreError>;
    fn delete(&self, id: &str) -> Result<(), CoreError>;
}
pub trait ArtifactStore: Send + Sync {
    fn allocate(&self, kind: ArtifactKind, engine: DbEngine, database: &str) -> Result<PathBuf, CoreError>;
}
pub trait CancellationToken: Send + Sync { fn is_cancelled(&self) -> bool; }
```

| Port | Desktop | CLI |
| --- | --- | --- |
| `ProgressSink` | `window.emit("job-progress", …)` | stderr lines, or NDJSON on stderr with `--json` |
| `CredentialProvider` | OS keyring | OS keyring; env provider only with `--credential-source env` |
| `ArtifactStore` | OS data dir | OS data dir |
| `CancellationToken` | window close | Ctrl-C handler |

### 5.4 Cancellation, cleanup, timeouts, child processes

- `CancellationToken` is checked between batches, before each child spawn, and before each write phase.
- Child processes are spawned in their own process group. On cancel: `SIGTERM` to the group, 5 s grace, then `SIGKILL` (Unix); `taskkill /T /F` (Windows).
- Every spawned child is wrapped in a guard that kills it on drop, so an early return cannot leak a process.
- Timeouts: connect (10 s), statement (configurable, default 30 s), per-table (configurable), total operation (configurable). A timeout maps to `ErrorCode::Operation` with a `timeout` detail.
- Temp files and partial artifacts are removed by a drop guard on failure; a successful run renames the temp file into place atomically.

---

## 6. Domain model

### 6.1 Connection

```rust
pub struct DbConnection {
    pub id: String,
    pub name: String,
    pub engine: DbEngine,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub use_ssl: bool,
    pub target: TargetClass,
    pub identity: Option<ServerIdentity>,   // captured at classification time
}

pub enum TargetClass { Production, Staging, Local, Unknown }
```

`use_ssl` becomes functional: it selects the TLS mode on the sqlx connection options. `identity` is `None` until the connection is classified; a `None` identity forces re-verification before any write.

### 6.2 Errors

```rust
pub enum ErrorCode {
    Usage, Connection, PolicyRefusal, DestinationUnverified, DestinationMismatch,
    PlanInvalid, CredentialUnavailable, UnclassifiedColumn, UnsupportedSchema,
    IntegrityViolation, Operation, Cancelled, NotFound, Unsupported,
}
```

`CoreError` carries `code`, a redacted `message`, and an optional `detail`. `anyhow` may be used internally but is converted at every public boundary. Connection URLs are never formatted into a message.

### 6.3 Safety policy

```rust
pub enum Operation { Inspect, QueryRead, QueryWrite, Backup, Restore, Copy, SafeSyncPlan, SafeSyncRun }

pub struct SafetyPolicy;

impl SafetyPolicy {
    pub fn authorize(&self, op: Operation, target: &DbConnection, ctx: &AuthContext)
        -> Result<(), CoreError>;
}
```

`AuthContext` carries the config opt-in, the verified development target (if any), and the execution plan (if any). `authorize` implements the tiers in §4.5 exactly. `SafeSyncRun` accepts only a `VerifiedDevelopmentTarget` and ignores every other field, so no opt-in or plan can widen it.

---

## 7. Connection management

### 7.1 Locations

| Purpose | Path |
| --- | --- |
| Config | `dirs::config_dir()/dbpinball/connections.json` |
| Artifacts (backups, snapshots) | `dirs::data_dir()/dbpinball/artifacts/` |
| Audit log | `dirs::data_dir()/dbpinball/audit.log` |

Resolved: `%APPDATA%\dbpinball\` on Windows, `~/Library/Application Support/dbpinball/` on macOS, `$XDG_CONFIG_HOME/dbpinball/` or `~/.config/dbpinball/` on Linux.

### 7.2 Record format

```json
{
  "version": 1,
  "connections": [
    {
      "id": "uuid",
      "name": "local-dev",
      "engine": "postgres",
      "host": "127.0.0.1",
      "port": 5432,
      "database": "app",
      "username": "postgres",
      "useSsl": false,
      "target": "local",
      "identity": { "systemIdentifier": "7412...", "serverAddress": "127.0.0.1", "port": 5432, "confidence": "high" },
      "developmentTarget": {
        "kind": "loopback-instance",
        "expectedIdentity": "7412...",
        "expiresAt": null
      }
    }
  ]
}
```

No password field exists. A serialization test asserts the key is absent.

### 7.3 Credential lifecycle

| Stage | Behavior |
| --- | --- |
| Create | Password read from stdin (never argv), stored in the OS keyring under service `dbpinball`, account = connection id |
| Read | Keyring only. Env provider only with an explicit `--credential-source env` and a named variable |
| Rotate | `connections set-password <id>` reads from stdin and overwrites the keyring entry |
| Delete | `connections remove <id>` deletes the keyring entry, then the record. A keyring failure is reported, not swallowed |
| Keyring unavailable | `ErrorCode::CredentialUnavailable`. **Never** falls back to plaintext |

### 7.4 Migration from localStorage

Idempotent and rollback-safe:

1. If `connections.json` exists and has `migratedFrom`, stop (no-op).
2. Read `localStorage.dbpinball_connections` (desktop only).
3. Validate each record: required fields, engine in `{mysql, postgres}`, port in `1..=65535`, non-empty host/database/username.
4. Drop invalid records and report them by index and reason; never coerce.
5. Deduplicate by `(engine, host, port, database, username)`; keep the first, report the rest.
6. Set `target: "unknown"` and `identity: null` for every migrated record.
7. Write atomically (temp file + rename, `0600` on Unix).
8. Set `migratedFrom: "localStorage@v1"`.
9. Leave the localStorage key in place for one release as a rollback path.

Rollback: delete `connections.json`; the localStorage key is still intact. Re-running the migration after a rollback is safe because step 1 only short-circuits when the marker exists.

---

## 8. CLI contract

Binary: `dbpinball`.

```bash
dbpinball --version
dbpinball doctor [--json]

dbpinball connections list [--json]
dbpinball connections add --name <n> --engine <mysql|postgres> --host <h> --port <p> \
    --database <d> --username <u> --target <production|staging|local|unknown> [--json]
dbpinball connections set-password <id>          # reads stdin
dbpinball connections test <id> [--json]
dbpinball connections remove <id> [--json]

dbpinball schema inspect --connection <id> [--database <db>] [--json]
dbpinball query run --connection <id> --database <db> --sql <sql> [--read-only] [--limit <n>] [--json]

dbpinball backup --connection <id> [--structure] [--data] [--compress] [--json]
dbpinball restore --connection <id> --file <path> [--drop-existing] [--dry-run] [--json]
dbpinball copy --source <id> --target <id> [--dry-run] [--json]

dbpinball targets declare --connection <id> --kind <local-socket|loopback-instance|docker-network|authorized-remote-dev> \
    [--expected-identity <fingerprint>] [--expires <duration>] [--json]
dbpinball targets list [--json]

dbpinball plan create --operation <restore|copy|query-write> --connection <id> \
    [--file <path>] [--sql <sql>] [--acknowledge <token>] [--json]
dbpinball plan show <plan-id> [--json]
dbpinball plan run <plan-id> [--json]

dbpinball safesync plan --source <id> --target <id> [--tables <list>] [--json]
dbpinball safesync run  --source <id> --target <id> --plan <plan-id> [--json]
dbpinball safesync grants --source <id> [--tables <list>]
```

A write to a verified local target may run directly. A write to a non-local target must go through `plan create` → `plan run`; the plan carries the destination identity, database, artifact hash, operation, expiry, and (for production) the acknowledgement. There is no flag that substitutes for a plan.

### 8.1 Output rules

- Human-readable by default.
- `--json` emits exactly one JSON object on **stdout** and nothing else.
- Progress, warnings, and errors go to **stderr** (NDJSON events on stderr with `--json`).
- Envelope:

```json
{ "ok": true,  "data": { }, "error": null }
{ "ok": false, "data": null, "error": { "code": "destination_not_local", "message": "..." } }
```

- `--non-interactive` is implied when stdin is not a TTY. No command prompts in that mode; a missing acknowledgement is a `policy_refusal`.

### 8.2 Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Usage error |
| 3 | Connection failure |
| 4 | Policy refusal |
| 5 | Operation failure |
| 6 | Cancelled |
| 7 | Not found |
| 8 | Destination verification failed |
| 9 | Integrity or validation failure |

---

## 9. Safe Sync

Safe Sync is a dedicated production-to-local debugging workflow. It is not a backup tool and not a copy tool.

### 9.1 Pipeline

```text
plan ──> inspect schema ──> classify columns ──> select rows ──> mask ──> load ──> verify ──> validate
  │                                                                                        │
  └── preview: tables, row estimates, masked columns, refusals, unsupported features ───────┘
```

### 9.2 Stages

1. **Explicit endpoints.** `--source` and `--target` are mandatory and distinct. Same connection id for both is `ErrorCode::Usage`.
2. **Source read-only.** The source session is opened read-only (§4.7). A write attempt fails at the server.
3. **Verified writable development target.** §4.4. Non-local is refused with no override.
4. **Schema inspection.** Enumerate tables, columns, types, PKs, FKs, unique constraints, indexes, sequences, generated columns, partitions, triggers, RLS policies, extensions, custom types, materialized views, foreign tables.
5. **Selective extraction.** `--tables` limits scope. Without it, all tables in the selected schema are candidates, but any table with an unclassified column is refused rather than silently included.
6. **Sensitive field classification.** Each column is `Identifier`, `Sensitive`, or `NonSensitive`. Classification comes from an explicit policy file plus name/type heuristics. A column that matches a sensitive heuristic but has no explicit classification is treated as `Sensitive`.
7. **Deterministic pseudonymization and synthetic masking.** See §9.4. HMAC-SHA256 is the pseudonymization primitive, not the whole strategy.
8. **Fail-closed on unknown sensitive data.** An unclassified column in an extracted table, or an unsupported type in a sensitive column, aborts the plan with `ErrorCode::UnclassifiedColumn` or `ErrorCode::UnsupportedSchema`.
9. **Preview and validated execution planning.** `safesync plan` writes a plan artifact containing the table list, row estimates, masked columns, refused tables with reasons, unsupported features, and the destination fingerprint. `safesync run` requires the plan id and re-verifies the destination before writing.
10. **Bounded streaming.** Batch size, max rows per table, max total bytes, statement timeout, per-table timeout, total timeout. Cancellation is checked between batches.
11. **Referential integrity verification.** For every FK, the orphan count in the destination must be zero. Row counts per table are compared against the source selection.
12. **Sanitized snapshot validation.** Scan the destination for sampled source sensitive values and for sensitive patterns (email, phone, national id). Any hit fails the run.
13. **Unsupported schema features.** Each feature has an explicit policy: supported, supported-with-caveat, or refused. Refusals are listed in the plan preview, never discovered mid-run.

### 9.3 Unsupported feature policy

| Feature | Policy |
| --- | --- |
| Generated columns | Supported (recomputed on load) |
| Partitioned tables | Supported-with-caveat (parent + partitions copied as plain tables) |
| Exclusion constraints | Refused |
| Custom types / enums | Supported if the type exists in the destination, else refused |
| Extensions | Refused unless explicitly allow-listed |
| Triggers | Not copied; reported |
| RLS policies | Not copied; reported |
| Materialized views | Refused |
| Foreign tables | Refused |
| Sequences with custom increment | Supported-with-caveat (increment preserved, value reset) |

### 9.4 Masking architecture

HMAC-SHA256 is approved as a **deterministic pseudonymization primitive**. It is not the complete masking strategy, and pseudonymization is not anonymization: a holder of the key can re-identify the data. The spec, CLI help, and UI must say so.

#### 9.4.1 Canonical identity mapping

Related PK/FK columns must resolve to the same canonical identity, derived from schema metadata rather than from column names:

```rust
pub struct CanonicalIdentity {
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,   // ordered; one entry for a simple key, N for a composite key
}
```

- A foreign key resolves to the `CanonicalIdentity` of the **referenced** primary or unique key, not to its own column.
- Two FKs pointing at the same PK therefore share one mapping, and joins survive.
- Composite keys use the ordered column list as the identity; each component is masked with a namespace derived from the composite identity plus its ordinal, so `(a, b)` and `(b, a)` never collide.

#### 9.4.2 Type and constraint preservation

Every generated value is validated against the destination column before load:

- declared type and length (`VARCHAR(64)` stays ≤ 64),
- nullability (a `NOT NULL` column never receives `NULL`),
- enum and domain membership,
- check constraints,
- unique constraints.

A value that cannot satisfy the constraints is a hard failure, not a silent truncation.

#### 9.4.3 Collision prevention and detection

- The masking engine keeps an in-memory map from `(canonical identity, original value)` to masked value.
- On a collision (two originals mapping to one masked value), the input is extended deterministically with a counter and re-hashed until unique.
- At load time, a unique-constraint violation is reported as `ErrorCode::IntegrityViolation` with the table and column, never swallowed.

#### 9.4.4 Synthetic PII generation

Hashing alone produces opaque tokens that break application behavior (an email column must still look like an email). PII columns use deterministic synthetic generators seeded by the HMAC output:

| Class | Generator |
| --- | --- |
| Email | Deterministic local part + fixed reserved domain (`example.invalid`) |
| Person name | Deterministic pick from a bundled name table |
| Phone | Valid format for the configured locale, reserved range |
| Address | Deterministic street/city from a bundled table |
| Free-form identifier | Format-preserving token of the same length and character class |

#### 9.4.5 Secrets

Passwords, API keys, tokens, and private keys are **nullified or replaced with a fixed non-authenticating marker** (`***REDACTED***`). They are never replaced with a value that could authenticate, and never with a reversible encoding.

#### 9.4.6 JSON and free text

These have no reliable structure, so they require an explicit per-column policy:

| Policy | Behavior |
| --- | --- |
| `json:redact-keys` | Remove or mask a declared key list; unknown keys fail closed |
| `json:mask-values` | Mask every scalar leaf; preserve structure |
| `text:redact-patterns` | Apply declared regex patterns (email, phone, national id) |
| `text:passthrough` | Copy verbatim; only allowed when the column is explicitly classified `NonSensitive` |

A JSON column with no policy is refused. A free-text column with no policy is refused.

#### 9.4.7 Key lifecycle

| Stage | Behavior |
| --- | --- |
| Generation | 32 bytes from the OS CSPRNG per run |
| Storage | OS keyring, separate service entry (`dbpinball-masking`), never in the config file or the snapshot |
| Rotation | New key per run by default; a shared key may be configured for reproducible snapshots across runs |
| Destruction | Destroyed after snapshot validation; a retained key is reported in the run summary |
| Re-identification warning | The run summary states that retaining the key permits re-identification |

#### 9.4.8 Validation before availability

A snapshot is written to a quarantine path, validated, and only then atomically promoted:

1. Referential integrity: every FK orphan count is zero.
2. Row counts per table match the source selection.
3. Residual-secret scan: sampled source sensitive values and sensitive patterns must not appear.
4. Constraint check: no unique or check violation.
5. Masking outcome check: every masked column differs from its source value, and every `NonSensitive` column is byte-identical.

A failure leaves the snapshot in quarantine and returns `ErrorCode::IntegrityViolation`.

---

## 10. Desktop adapter

`src-tauri/src/lib.rs` keeps all 10 existing command names and payload shapes. Each body becomes a call into `dbpinball-core` with a `TauriProgressSink` and `KeyringCredentialProvider`.

New commands: `connections_list`, `connections_save`, `connections_delete`, `safesync_plan`, `safesync_run`.

`runners/` is **not deleted** until every replacement passes integration and regression tests (see M1 exit criteria).

### 10.1 Compatibility handling for changed behavior

| Change | Compatibility handling |
| --- | --- |
| `run_query` becomes read-only by default | The workspace keeps a write toggle. When the user enables it, the UI shows the same confirmation gate as the CLI. Existing read-only usage is unaffected. |
| `restore`/`copy` honor `drop_existing` | Previously a no-op. Now it actually drops. The UI must show an explicit warning before enabling it, and the command requires the confirmation gate. |
| `use_ssl` becomes functional | Connections with `useSsl: true` that previously connected in plaintext may now fail if the server has no TLS. The migration sets `useSsl` unchanged; a connection failure surfaces a clear message naming the flag. |
| `include_views` no longer maps to `--events` | Views are dumped by default; events are no longer silently included. Documented in the release notes. |
| Internal dumper returns `Unsupported` | Previously produced an empty file. Now fails loudly. This is a deliberate behavior change and is called out in the release notes. |

---

## 11. Milestones

Each milestone lists prerequisites, security acceptance criteria, integration tests, regression tests, and rollback.

### 11.0 Defect remediation map

Confirmed defects are fixed in the milestone that owns the affected code, never bundled into an unrelated milestone. **M1 fixes no defect**, because M1 is extraction and compatibility only.

| Defect | Fixed in | Why there |
| --- | --- | --- |
| D1 internal dumper writes an empty file | M3 (fail loudly in place); real implementation M7 | Safety: a silent empty backup is worse than a loud failure |
| D2 `drop_existing` / `drop_in_target` ignored | M3 | Safety: the option must either work or be refused |
| D3 unrestricted `run_query` | M3 | Security: read-only default belongs with the policy |
| D4 `use_ssl` unused | M3 | Security: TLS belongs with the policy |
| D5 `include_views` mapped to `--events` | M3 | Correctness: small, same file as D1/D2 |
| D6 password interpolated into connection URLs | M3 | Security: redaction and `ConnectOptions` belong with the policy |
| D7 version drift (`package.json` 0.1.6 vs `tauri.conf.json` 0.1.0) | M2 | The CLI needs a single version source |

### M0 — Audit and baseline verification

- **Prerequisites:** none.
- **Deliverables:** `docs/ARCHITECTURE-AUDIT.md`; recorded baseline of `npm test`, `cargo test`, release tooling state.
- **Security acceptance:** every confirmed defect is listed with file:line evidence.
- **Integration tests:** none (read-only milestone).
- **Regression tests:** baseline commands recorded with their output.
- **Rollback:** delete the audit document.
- **Status:** audit complete. Baseline re-verified in this revision (§13).

### M1 — Incremental core extraction with existing Desktop compatibility

- **Prerequisites:** M0 complete.
- **Deliverables:** workspace root; `dbpinball-core` with `domain/{connection,error}.rs` and `ports/{progress,credentials}.rs`; **one** capability extracted (`inspect`: test, list databases, list tables); `KeyringCredentialProvider`; `NoopProgress`; `scripts/check-boundaries.cjs`. `runners/` remains in place and untouched. `options.rs`, `result.rs`, `ArtifactStore`, `CancellationToken`, and `TauriProgressSink` arrive with the services that need them.
- **Explicitly out of scope:** the CLI crate, `SafetyPolicy`, `DestinationVerifier`, execution plans, Safe Sync, masking, and every defect in §11.0. M1 must not start M2 or Safe Sync.
- **Security acceptance:** `dbpinball-core` compiles without `tauri` or `clap`; the boundary check fails when either is added; no behavior change in the desktop app.
- **Integration tests:** `inspect` against Docker MySQL and PostgreSQL, gated behind `--features integration`.
- **Regression tests:** the 10 existing vitest tests stay green; `cargo test -p dbpinball-core` green; all 10 Tauri command names and payloads unchanged.
- **Rollback:** revert the workspace commit; `runners/` was never removed, so the desktop returns to its prior state with no data migration.

### M2 — CLI foundation and contract tests

- **Prerequisites:** M1 complete.
- **Deliverables:** `dbpinball-cli` with `doctor`, `connections`, `schema inspect`, `query run`; JSON envelope; exit codes; noninteractive mode.
- **Security acceptance:** no `--password` flag; password read from stdin; redaction tests pass.
- **Integration tests:** CLI contract tests spawn the binary and assert exit code plus JSON envelope.
- **Regression tests:** desktop still builds and its tests pass.
- **Rollback:** remove the CLI crate; the desktop is unaffected.

### M3 — Shared security policy and destination verification

- **Prerequisites:** M2 complete.
- **Deliverables:** `SafetyPolicy` with the T0–T3 tiers; `DestinationVerifier` with `DevelopmentTargetKind`; `ExecutionPlan`; audit log; `targets declare`; defect fixes D1–D6 from §11.0.
- **Security acceptance:** the full §4.6 matrix passes, including `undeclared_tunnel_refused`, `low_confidence_identity_refused`, `plan_is_single_use`, and `noninteractive_missing_plan_refused`.
- **Integration tests:** verification against a real PostgreSQL and MySQL, a read-only destination, a Docker-network target, and a declared tunnel.
- **Regression tests:** existing admin commands still work for local targets; the 10 vitest tests stay green.
- **Rollback:** the policy is additive; disabling the admin gate restores prior behavior for local targets only. Defect fixes D1–D6 are individually revertible commits.

### M4 — PostgreSQL sanitized sync through CLI

- **Prerequisites:** M3 complete.
- **Deliverables:** `safesync plan` and `safesync run` for PostgreSQL; masking engine; integrity and snapshot validation.
- **Security acceptance:** source write rejected by the server; non-local destination refused; unclassified column refused; residual secret detected; orphan count zero.
- **Integration tests:** end-to-end against a seeded PostgreSQL with FKs and sensitive columns.
- **Regression tests:** raw `backup`/`restore`/`copy` unchanged and still labeled raw.
- **Rollback:** Safe Sync is a new command surface; removing it leaves admin commands intact.

### M5 — Desktop Safe Sync integration

- **Prerequisites:** M4 complete.
- **Deliverables:** `safesync_plan` / `safesync_run` Tauri commands; UI preview and confirmation.
- **Security acceptance:** the UI cannot bypass the policy; the same `ServiceContext` is used.
- **Integration tests:** desktop flow against the same seeded database.
- **Regression tests:** all prior vitest tests plus new command tests.
- **Rollback:** hide the UI entry point; the CLI remains the source of truth.

### M6 — MySQL sanitized sync

- **Prerequisites:** M5 complete.
- **Deliverables:** MySQL implementation of the same contracts.
- **Security acceptance:** same matrix as M4, with MySQL-specific read-only enforcement (error 1792).
- **Integration tests:** same seeded scenario on MySQL.
- **Regression tests:** PostgreSQL Safe Sync unchanged.
- **Rollback:** MySQL Safe Sync is refused with `Unsupported` if the milestone is reverted.

### M7 — Advanced snapshots and relational dependency resolution

- **Prerequisites:** M6 complete.
- **Deliverables:** dependency-ordered extraction, cycle handling, portable snapshot format, real internal dumper.
- **Security acceptance:** masking and validation still hold under dependency ordering.
- **Integration tests:** cyclic FK graphs, self-referencing tables, large tables.
- **Regression tests:** all prior suites.
- **Rollback:** advanced features are opt-in flags; the M4/M6 path remains.

---

## 12. CI

| Check | Command | Fails when |
| --- | --- | --- |
| Headless compile | `cargo check -p dbpinball-core` | core does not compile without Tauri |
| Dependency boundary | `node scripts/check-boundaries.cjs` | `cargo tree -p dbpinball-core` contains `tauri` or `clap` |
| Rust tests | `cargo test --workspace` | any test fails |
| Frontend tests | `npm test` | any test fails |
| CLI contract | `cargo test -p dbpinball-cli --test contract` | exit code or JSON envelope drifts |
| Format and lint | `cargo fmt --check`, `cargo clippy -D warnings` | style or lint violation |

`scripts/check-boundaries.cjs` is a new file added in M1. It is not part of the existing release tooling.

---

## 13. Working tree review

Inspected at the start of this revision. Nothing was reset or discarded.

| Path | State | Action taken |
| --- | --- | --- |
| `.husky/pre-commit` | Modified: now runs `node scripts/verify-tests.cjs` | **Preserved unchanged** |
| `package.json` | Modified: `@programinglive/commiter` bumped | **Preserved**, then migrated to `^1.5.1` at the owner's request |
| `scripts/release.cjs` | Modified, and **broken**: requires `./update-release-notes.cjs` and `./preload/fs-f-ok.cjs`, neither of which exists (`MODULE_NOT_FOUND`) | Backed up to `%TEMP%\opencode\dbpinball-release.cjs.bak`, then removed as part of the commiter 1.5.1 migration |
| `scripts/verify-tests.cjs` | Untracked, working | **Preserved unchanged** |
| `docs/ARCHITECTURE-AUDIT.md` | New (this session) | Kept |
| `docs/superpowers/specs/…` | New (this session) | Kept |

### 13.1 Release tooling migration (owner-requested)

The owner reported that all eight repositories now use `commiter release` with `@programinglive/commiter@^1.5.1` and no local `scripts/release.cjs`. DBPinball was aligned:

- `package.json`: `release` → `commiter release`, plus `release:major|minor|patch` and `release:first`.
- `@programinglive/commiter` → `^1.5.1` (installed).
- `scripts/release.cjs` removed (backup retained in `%TEMP%`).
- `@testing-library/dom` added as a devDependency: the install dropped it as a transitive dependency and broke all three test files. Restored and verified.

Verification:

- `npm run release -- --dry-run` → reaches the engine and reports `Working tree is not clean` (expected while the tree is dirty). The CLI resolves; no `MODULE_NOT_FOUND`.
- `node scripts/verify-tests.cjs` → 3 files, 10 tests passed, pre-commit gate green.

### 13.2 Review of the Commiter 1.5.1 migration (Decision 4)

The original modified `scripts/release.cjs` was compared against the 1.5.1 engine before deciding whether to recover it.

| Feature in the local modification | Present in 1.5.1 | Verdict |
| --- | --- | --- |
| `--first-release` handling | Yes, and stricter (only accepted from version `0.0.0`) | Superseded |
| Unknown-release-type warning | Yes, as a `ReleaseError` | Superseded |
| `bun.lock` lockfile detection | Yes | Superseded |
| `ensurePrdPresence` (warn when `docs/PRD.md` is missing) | **No** | Unique, but it only warns and points at a path that does not exist in this repo (the PRD is at the root) |

**Decision: recovery is not needed.** The local script was a partial vendored copy that could not run (`MODULE_NOT_FOUND` for `./update-release-notes.cjs` and `./preload/fs-f-ok.cjs`). Restoring it would reintroduce a broken `npm run release`. The backup is retained at `%TEMP%\opencode\dbpinball-release.cjs.bak` and is not deleted.

`ensurePrdPresence` is recorded here as a candidate upstream contribution to `@programinglive/commiter`, not as a local script. No further release-process changes are made in this task.

### 13.3 Observation, not fixed

The removed local script checked `docs/PRD.md`, which does not exist; the PRD is at the repository root. The 1.5.1 engine does not perform this check, so the observation is now moot. No action taken.

---

## 14. Feature status classification

| Item | Status |
| --- | --- |
| Connection CRUD, keyring storage, workspace, query grid | **Existing, working** |
| Raw backup/restore/copy via vendor tools | **Existing, working** |
| Internal dumper | **Confirmed defect** — writes an empty file |
| `drop_existing` / `drop_in_target` | **Confirmed defect** — ignored |
| `use_ssl` | **Confirmed defect** — unused |
| `include_views` → `--events` | **Confirmed defect** — wrong flag |
| Unrestricted `run_query` | **Confirmed defect** — no guard |
| Password in connection URL | **Confirmed defect** — malformed URLs, leak risk |
| `dbpinball-core` extraction | **Proposed design** (M1) |
| `dbpinball` CLI | **Proposed design** (M2) |
| Safety policy and destination verification | **Proposed design** (M3) |
| Safe Sync with masking | **Proposed design** (M4–M6) |
| Dependency-ordered snapshots, real internal dumper | **Future capability** (M7) |
| Cross-engine copy, SSH tunnels, cloud storage | **Future capability, out of scope** |

---

## 15. Failure and recovery

| Failure | Behavior | Recovery |
| --- | --- | --- |
| Destination identity changes mid-run | Abort before the next write phase | Re-run `safesync plan` |
| Undeclared tunnel detected | `DestinationUnverified`; no write | Declare the target with its expected identity, or use a real local instance |
| Plan expired or already used | `PlanInvalid`; no write | Create a new plan |
| Masking key lost | Snapshot remains valid but cannot be reproduced | Accept, or re-run with a shared key |
| Low-confidence identity | Safe Sync refuses | Grant the identity privilege, or use a socket/Docker target |
| Source connection drops | `ErrorCode::Connection`; partial artifact removed | Re-run |
| Child process hangs | Timeout kills the process group | Re-run with a larger timeout |
| Cancel (Ctrl-C) | `ErrorCode::Cancelled`; children killed; temp files removed | Re-run |
| Keyring unavailable | `CredentialUnavailable`; no plaintext fallback | Fix the keyring, or use the env provider explicitly |
| Config file corrupt | Record rejected with index and reason; other records still load | Fix or remove the record |
| Migration interrupted | Atomic write means either the old or the new file exists | Re-run; step 1 makes it idempotent |
| Integrity check fails | Run fails; destination left in a known partial state | Drop and re-run |
| Snapshot validation fails | Run fails; artifact quarantined | Fix classification and re-run |

---

## 16. Acceptance criteria for this spec

The spec is ready for implementation when the owner confirms:

1. The risk tiers T0–T3 in §4.5 are the intended policy, including `Unknown` mapping to T2.
2. Safe Sync has no override at any tier, as stated.
3. The verified development target model in §4.4 (socket, loopback instance, Docker network, authorized remote dev) covers the environments you actually use.
4. The execution-plan binding in §4.5.1 (identity, database, artifact hash, operation, expiry, single use) is sufficient for high-risk writes.
5. The masking architecture in §9.4 (canonical identity mapping, synthetic PII, collision handling, key lifecycle) is acceptable.
6. The unsupported-feature policy in §9.3 is acceptable.
7. The milestone boundaries, defect map, and rollback plans are acceptable.

### 16.1 Consistency review

Cross-checked after the corrections:

| Check | Result |
| --- | --- |
| Permission matrix vs threat model | Every threat in §4.1 maps to a tier or a verification step. The tunnel threat is covered by §4.4 step 3. |
| Permission matrix vs enforcement table | Every matrix row has at least one §4.6 row naming its mechanism and test. |
| Safe Sync vs admin separation | §4.3, §4.5, and §9 agree: Safe Sync is one-way, verified-development-target only, with no override. |
| Milestone acceptance vs defect map | M1 fixes no defect; D1–D6 land in M3; D7 lands in M2. No milestone claims a fix it does not own. |
| Milestone acceptance vs §4.6 tests | Each test named in §4.6 is assigned to the milestone that introduces its mechanism. |
| Masking vs validation | §9.4.8 validates the properties §9.4.1–§9.4.6 promise. |
| Noninteractive vs policy | §4.5.2 and §8.1 agree: no prompt, no bypass. |
| Working tree vs scope | §13 records every change; no unrelated release-process change is made. |

## 17. Deferred and out of scope

- Cross-engine copy and type mapping.
- SSH tunnels.
- Scheduled and incremental backups.
- Cloud storage targets.
- Real internal dumper (M7).
- Snapshot portability across engines.
- UI redesign beyond the connection source and Safe Sync entry point.
