# DBPinball M4 — PostgreSQL Safe Production-to-Local Sync

## Architecture Specification

Status: **design approved for review; not implemented.**

Baseline: `main` at `e2b84960fb43e7bac6b8c20b211adad6dbd3126a` (M3 merged).

Planning branch: `docs/m4-safe-sync-plan`.

This document is the design of record for M4. It describes what will be built, the
trust boundaries it assumes, and what it will not guarantee. The accompanying
implementation plan is
`docs/superpowers/plans/2026-11-14-dbpinball-m4-safe-sync-plan.md`.

---

## 1. Scope and non-goals

### 1.1 The one use case

A developer needs a small, representative slice of production PostgreSQL data to
reproduce a bug locally, without copying unmasked personal data into the
development environment. M4 performs that extraction once, explains exactly what
it will do before doing it, masks sensitive values on the way out, and imports
into a verified development destination.

### 1.2 In scope

| # | Capability | Shape in M4 |
| --- | --- | --- |
| 1 | Source PostgreSQL connection | A stored profile, verified as PostgreSQL and read-only |
| 2 | Verified development destination | An M3-enrolled `verified_development` or `verified_disposable` destination |
| 3 | Explicit table selection | Declarative roots; no implicit "whole database" mode |
| 4 | Row filtering | Parameterized `eq`/`in`/`gte`/`lte` predicates only |
| 5 | Dependency discovery | `pg_catalog` foreign-key graph, parent-directed by default |
| 6 | Safe extraction | Read-only session, bounded, cancellable |
| 7 | Data masking | Mandatory, fail-closed, deterministic where keys require it |
| 8 | Foreign-key preservation | Insertion ordered by the FK graph, or refusal |
| 9 | Preview and dry-run | Metadata-only and row dry-run, both without writes |
| 10 | Controlled local import | One destination transaction by default |
| 11 | Failure recovery | Transactional rollback, run journal, bounded retry rules |
| 12 | CLI-first execution | `dbpinball safe-sync …`, fully testable without the desktop |
| 13 | Privacy-safe audit records | Counts and identities only; never row values |

### 1.3 Explicit non-goals

M4 does **not** provide, and its interface must not imply:

- full or unrestricted production database cloning;
- any write to the production source;
- MySQL Safe Sync (M6);
- unmasked production import by default, or any fallback to unmasked;
- execution of user-supplied transformation code (SQL, expressions, scripts);
- production schema changes;
- background synchronization or scheduled replication;
- bypassing destination enrollment or the M3 policy;
- merging Safe Sync into the existing backup/restore/copy administrative paths.

The existing `backup`, `restore` and `copy` commands remain separate administrative
tools with their own M3 gates. Safe Sync is a new, narrower pipeline.

### 1.4 Supported engine matrix

| Source | Destination | Supported |
| --- | --- | --- |
| PostgreSQL | PostgreSQL | **yes, M4** |
| MySQL | PostgreSQL | no — refused (`unsupported_engine`) |
| PostgreSQL | MySQL | no — refused |
| MySQL | MySQL | no — refused (M6) |

The source **and** destination must both report `DbEngine::Postgres`. This is
checked from the connection profile before any connection is opened and from the
live session identity after it is opened.

---

## 2. Threat model

Format: threat → prevention → detection → recovery → residual. Numbering is stable
and referenced by the acceptance tests.

| # | Threat | Prevention | Detection | Recovery | Residual |
| --- | --- | --- | --- | --- | --- |
| 1 | Production mislabeled `local` | M3 policy ignores labels; destination must be enrolled dev/disposable | `PolicyCode::NotEnrolled` / `DestinationNotAllowed` | Refused before any write | An owner can deliberately enroll production — enrollment is authorization, not proof |
| 2 | Localhost tunnel redirected to production | Identity = server `system_identifier` + database OID, not the endpoint | Session identity compared to plan identity | Refused (`destination_changed`) | A proxy that is already connected to production before verification can still serve production; identity is what the server reports |
| 3 | Source/destination reversal | Plan records both identity keys; policy denies equal keys; source role check requires no write privilege | `SameSourceAndDestination`; source privilege probe | Refused before extraction | A source and destination that are genuinely the same cluster cannot be used; that is correct |
| 4 | Connection changed after approval | Plan fingerprint binds source and destination identity keys; execute re-verifies on the execution session | Identity mismatch at execute | Refused; new plan required | A change between the re-verification query and the first write is not detected without server cooperation |
| 5 | Query or identifier injection | Identifiers validated against `pg_catalog` and quoted via `quote_ident`; values bound as parameters | Catalog allowlist mismatch | Refused (`invalid_identifier`, `invalid_filter`) | None known in the designed path; arbitrary expressions are not accepted at all |
| 6 | Unbounded dependency traversal | `max_depth`, `max_rows_per_table`, `max_total_rows`, `max_bytes`, `timeout_ms` | Limit counters during planning and extraction | Refused (`limit_exceeded`), nothing imported | A permitted depth can still be large; limits are the control |
| 7 | Unmasked sensitive column | Every selected column needs an explicit policy disposition; unknown type or unmapped column refuses | Masking coverage report in the plan | Refused (`unmapped_column`) | A column that is sensitive but named and typed innocuously is only caught by an explicit policy rule |
| 8 | Reidentification from deterministic masking | Keyed HMAC pseudonyms, secret key in the OS keyring, low-entropy columns refused or redacted | Uniqueness/collision check; policy class | Refused on collision | Frequency analysis on low-entropy columns can re-link pseudonyms; output is **de-identified**, not anonymous |
| 9 | Raw data leaked through temp files | No plaintext file is written; extraction holds masked rows in memory within `max_bytes` | Byte accounting; destination-only writes | Refused (`limit_exceeded`) | Process memory and swap are outside application control |
| 10 | Sensitive values in logs | Audit records contain counts, names, identities and outcomes only; `CoreError::redact` on every message | Audit schema review; secret scan | Redacted records are the only ones written | A database driver may echo a value in an error string; redaction is best-effort |
| 11 | Foreign-key inconsistency | Topological insertion order from the FK graph; parent rows always selected | Constraint violation aborts the transaction | Full rollback | A selected child whose parent is filtered out by the source role's RLS can still be missing; detected as a constraint failure |
| 12 | Cyclic FK deadlock or import failure | Cycles detected; deferred constraints used only when every edge is `DEFERRABLE`, otherwise refuse | Graph cycle analysis at plan time | Refused (`cyclic_fk_requires_deferrable`) | A deferrable cycle can still fail on data; rollback applies |
| 13 | Duplicate rows after retries | Single transaction; completed run recorded; identical fingerprint re-run is a no-op | Run journal and destination run table | No-op or rollback | A crash after commit but before the local journal update leaves the destination correct and the journal stale; `status` reconciles from the destination run table |
| 14 | Schema drift between plan and execute | Plan records a schema hash per table; execute re-reads the catalog | Hash mismatch | Refused (`schema_drift`); new plan required | A compatible change that does not alter the recorded columns is indistinguishable |
| 15 | Source privilege escalation | Refuse `rolsuper` and `rolbypassrls`; refuse if the role has any write privilege on a selected table; `default_transaction_read_only=on` | Privilege probe and `SHOW transaction_read_only` | Refused (`source_privileged`) | A role can gain privileges between the probe and the query; the read-only transaction is the backstop |
| 16 | Destination connection substitution | Destination identity read on the same connection that imports; no pool | Session identity vs plan identity | Refused; reconnect re-verifies | Same as #4 |
| 17 | Extensions or functions with side effects | Views, materialized views and foreign tables are refused; only base and partitioned tables are selected; no user function is called | `relkind` allowlist | Refused (`unsupported_relation`) | RLS policies and generated-column expressions can call functions; the restricted role's own privileges bound the damage |
| 18 | Crash during import | One destination transaction | Transaction state at reconnect | Rollback; destination unchanged; retry with the same plan | A crash during `COMMIT` is resolved by the server; the run journal may need `status` |
| 19 | Concurrent Safe Sync operations | Local run lock per destination; destination advisory lock inside the transaction | Lock acquisition failure | Refused (`busy`) | Two machines are coordinated only by the destination advisory lock, not by the local lock |
| 20 | Misleading success from partial extraction | The plan carries `complete`; execute refuses an incomplete plan; a failed table aborts the whole run | Per-table row counts vs plan | Refused or rolled back | None in the designed path: partial import is never defined as success |

---

## 3. Trust boundaries

```
┌──────────────────────────────────────────────────────────────────┐
│ Operator                                                         │
│  · writes the spec                                               │
│  · acknowledges the plan fingerprint                             │
└───────────────┬──────────────────────────────────────────────────┘
                │ CLI (trusted adapter, no policy decisions)
┌───────────────▼──────────────────────────────────────────────────┐
│ dbpinball-core::safesync   (policy owner)                        │
│  · source protection        · masking engine                     │
│  · bounded planner         · destination policy (M3)             │
└───────┬───────────────────────────────────┬──────────────────────┘
        │ BoundSession::Source              │ BoundSession::Destination
        │ (one connection, read-only tx)    │ (one connection, one transaction)
┌───────▼───────────────┐          ┌────────▼─────────────────────┐
│ Production PostgreSQL │          │ Development PostgreSQL       │
│ SELECT-only role      │          │ enrolled, verified, writable │
└───────────────────────┘          └──────────────────────────────┘
```

Trust assumptions, stated explicitly:

1. The **core** is trusted. It is the only component that decides.
2. The **CLI and desktop** are untrusted adapters. They pass arguments and render
   results; they never decide.
3. The **source server** is trusted to enforce its own read-only transaction and
   privilege model. M4 cannot prove a hostile server's behaviour.
4. The **destination server** is trusted to enforce constraints and transactions.
5. The **network** is untrusted. Identity is what the server reports on the
   authenticated session, not what DNS or a tunnel claims.
6. The **operator** is trusted to choose the destination honestly. M4 cannot know
   that an enrolled "development" database is not a copy of production.

---

## 4. Source protection

M4 treats the source as a read-only, potentially hostile-in-privilege system. Four
distinct facts are recorded, and only the last is an enforcement guarantee:

| Fact | Where it comes from | What it means |
| --- | --- | --- |
| Identity verification | `system_identifier`, database OID, `current_user`, backend PID | Which server and database answered |
| Declared source role | The spec's `source` connection profile | What the operator claims |
| Effective database privileges | `pg_roles`, `has_table_privilege`, `pg_class.relrowsecurity` | What the role can actually do |
| Actual enforcement guarantee | `default_transaction_read_only=on` plus `BEGIN READ ONLY` | What the server refuses |

**The client-side read-only flag is not a safety guarantee on its own.** The
guarantee is the *database-enforced* read-only transaction, and even that is only
as strong as the server's implementation and the absence of superuser escape
hatches. That is why privileged roles are refused.

### 4.1 Required source role

The source role must have, and only have:

```text
CONNECT on the source database
USAGE   on each selected schema
SELECT  on each selected table (or the selected columns)
```

The source role must **not** have, on any selected table:
`INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`, `REFERENCES`, `TRIGGER`, or ownership.
The enforced probe checks `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE` with
`has_table_privilege`, and separately refuses ownership
(`pg_class.relowner = current_user::regrole`). `REFERENCES` and `TRIGGER` are
recorded in the audit when present; they do not enable a data write, so they do
not block a read, but a role holding them is reported in the plan.

The source role must **not** be `rolsuper` and must **not** be `rolbypassrls`.
Both are checked from `pg_roles` for `current_user` and produce
`source_privileged` refusal.

### 4.2 Session setup

On opening the source `BoundSession`, in this order:

1. `SET default_transaction_read_only = on`
2. `SET statement_timeout = <limits.timeout_ms>`
3. `SET lock_timeout = <min(timeout_ms, 5000)>`
4. `SET idle_in_transaction_session_timeout = <timeout_ms>`
5. `SET client_encoding = 'UTF8'`
6. assert `SHOW transaction_read_only` is `on`
7. assert `SHOW default_transaction_read_only` is `on`

Every extraction transaction then starts with `BEGIN READ ONLY` and ends with
`ROLLBACK`. M4 never issues `COMMIT` on the source. The extraction client never
calls a user function, never runs `EXPLAIN ANALYZE`, and never uses
`SELECT ... INTO`.

### 4.3 Relation kinds

Only these `pg_class.relkind` values may be selected:

| relkind | Meaning | Allowed |
| --- | --- | --- |
| `r` | ordinary table | yes |
| `p` | partitioned table | yes (children are the leaves) |
| `f` | foreign table | **no** — refused |
| `v` / `m` | view / materialized view | **no** — refused |
| `S` | sequence | no |
| `t` | TOAST table | no |

Views and foreign tables are refused because their definitions or remote side can
execute code that the read-only transaction does not bound in the way the operator
expects.

### 4.4 Row-level security

If `pg_class.relrowsecurity` is true for a selected table:

- if the source role is the table owner or has `rolbypassrls`, **refuse**
  (`source_privileged`) — the role would read rows outside the intended policy;
- otherwise the rows the role can see are the rows M4 extracts, and the plan
  records `rls_active: true` for that table.

### 4.5 Source identity change

The plan records the source identity key. At execute time, the source
`BoundSession` re-reads it. A mismatch is `source_changed`; the plan is refused and
a new plan is required. This is the same rule as the destination, applied to the
source.

### 4.6 Authorization contract (approved)

The two Safe Sync operations have different authorization needs and must not share
one decision.

#### `SafeSyncPrepare`

Reading an explicitly authorized source does **not** require destination
enrollment. It does require all of the following, and fails closed on each:

| Requirement | Enforced by | Refusal code |
| --- | --- | --- |
| A source connection is declared | policy | `missing_source` |
| The source server identified itself | policy | `unverifiable_source` |
| The source is not the destination | policy | `same_source_and_destination` |
| The source engine is PostgreSQL | session | `unsupported` / `unsupported_engine` |
| The source role is not superuser / `rolbypassrls` | session | `source_privileged` |
| The source role has no write privilege on a selected table | session | `source_privileged` |
| The session is database-enforced read-only | session | `source_not_read_only` |
| The extraction scope is declared and catalog-validated | planner | `invalid_identifier` / `unsupported_relation` |
| Discovery is bounded by the declared limits | planner | `limit_exceeded` |
| No source write is issued | session + policy | any write is impossible; a bug is `source_write_attempt` |

`evaluate` returns `Allow { code: SourceAuthorized }` when the source is present,
identified, and not equal to a supplied destination. All the session and planner
checks are part of `SafeSyncPrepare` authorization even though they are not policy
inputs, because a policy `Allow` alone is never treated as sufficient.

#### `SafeSyncExecute`

Requires **both** an authorized read-only source session and an enrolled, verified
development/disposable destination session. Neither the source classification nor
the operator's acknowledgement is sufficient on its own.

| Requirement | Enforced by | Refusal code |
| --- | --- | --- |
| A source identity is present | policy | `missing_source` |
| A destination identity is present | policy | `missing_destination` |
| The source server identified itself | policy | `unverifiable_source` |
| The destination server identified itself | policy | `unverifiable_destination` |
| Source identity ≠ destination identity | policy | `same_source_and_destination` |
| Source session is read-only | session | `source_not_read_only` |
| Destination enrollment matches and is fresh | policy (M3, unchanged) | `not_enrolled` / `destination_changed` / `stale_verification` |
| Destination category allows writes | policy (M3, unchanged) | `destination_not_allowed` |
| Destination is not declared production | policy (M3, unchanged) | `protected_source` |
| Acknowledgement matches the plan and both identities | service | `acknowledgement_mismatch` |

`evaluate` returns `Allow { code: Allowed }` only when the destination checks pass
exactly as they do for `DatabaseRestore` and `DatabaseCopy`; M3 destination
verification is not weakened, not re-ordered, and not made optional.

### 4.7 Extraction scope

The source authorization is scoped to the tables the spec names. The planner
validates every root against the catalog and the relation allowlist, and the
extractor may read only the columns of those tables plus the parent tables the FK
graph adds (and only when `direction` permits). A table that is not in the resolved
plan is never queried, even if the source role could read it.

---

## 5. Same-session identity binding

This is the mandatory M4 security gate and the direct answer to the M3 TOCTOU
residual.

### 5.1 The rule

**A verification result never authorizes a write on a different connection.**
Identity is read on the same owned connection that runs the operation, and the
connection cannot be swapped afterwards.

### 5.2 `BoundSession`

```rust
pub enum SessionRole { Source, Destination }

pub struct SessionIdentity {
    pub engine: DbEngine,              // must be Postgres
    pub identity_key: String,          // postgres://host:port/db_oid#system_identifier
    pub system_identifier: String,
    pub database_oid: String,
    pub database_name: String,
    pub current_user: String,
    pub backend_pid: i32,
    pub backend_start: String,         // pg_stat_activity.backend_start
    pub server_addr: String,
    pub server_port: u16,
    pub transaction_read_only: bool,
}

pub struct BoundSession { /* owns a sqlx::PgConnection; fields private */ }

impl BoundSession {
    pub async fn open(
        role: SessionRole,
        connection: &DbConnection,
        credentials: &dyn CredentialProvider,
        limits: &Limits,
    ) -> Result<BoundSession, CoreError>;

    pub fn identity(&self) -> &SessionIdentity;

    /// Re-assert that this session is still the one recorded in the plan.
    pub async fn assert_binding(&self, expected: &SessionIdentity) -> Result<(), CoreError>;
}
```

`BoundSession::open` checks the profile engine, opens a single `PgConnection` (not a
pool), reads `session_identity()` **on that connection**, applies the role setup,
and stores both the connection and the identity. Later SQL runs through methods on
`BoundSession`, which take `&mut self`. There is no path that hands the connection
back to a pool.

### 5.3 Source binding

- The plan records `source: SessionIdentity` (identity key plus backend metadata).
- `execute` opens the source session and calls `assert_binding(plan.source)`.
- If the source connection dropped, a new session is opened and asserted; if the
  identity key differs, `source_changed`.
- `backend_pid`/`backend_start` are recorded for the audit trail but are **not**
  part of the identity key: a reconnect legitimately changes them.

### 5.4 Destination binding

- The plan records `destination: SessionIdentity`.
- `execute` opens the destination session **after** extraction and before any
  write, calls `assert_binding(plan.destination)`, then evaluates the M3 policy
  `SafeSyncExecute` **with the identity from this session**.
- The import then runs on the same session. There is no second connection and no
  timeout between the check and the first statement: the check and the write share
  one transaction.
- The destination transaction is `BEGIN` (read-write) immediately before the check,
  and the check query runs inside it. Any identity-changing reconnect aborts the
  transaction.

### 5.5 What this does and does not remove

Removed: connection-pool substitution, a second DNS resolution between verify and
write, a stale 10-minute M3 verification authorizing a write, and connect-retry
creating a fresh session after a successful check.

Not removed: a server that lies about `system_identifier`, a network path that is
already redirected before the first query, or an operator who enrolls production.
The freshness window from M3 is no longer used to authorize Safe Sync: the plan's
own identity binding and expiry are the controls.

### 5.6 Retry and reconnect rules

| Event | Behaviour |
| --- | --- |
| Source connection drops during extraction | Rollback source transaction; reopen; re-assert identity; restart extraction from the beginning (no partial source state is persisted) |
| Destination connection drops before commit | Transaction aborted by the server; reopen; re-assert; re-run the policy check; retry the whole import |
| Destination connection drops after commit | Treat as committed; `status` reconciles from the destination run table |
| Plan expired | Refused; new plan required |
| Credential expired | Refused (`connection`); re-authenticate and produce a new plan |

No retry may reuse a previous authorization without re-running `assert_binding` and
the policy check.

### 5.7 `BoundSession` contract (final)

One `BoundSession` is created per side per execution. The same authenticated
`sqlx::PgConnection` is used to:

1. establish the server and database identity (`system_identifier`,
   `current_database()` and its OID, `current_user`, `pg_backend_pid()`,
   `pg_stat_activity.backend_start`, `inet_server_addr()`, `inet_server_port()`);
2. verify the intended database role (superuser / `rolbypassrls` / write privilege
   / ownership, per §4.1);
3. enforce the transaction state — the source session opens with
   `default_transaction_read_only = on` and every extraction transaction is
   `BEGIN READ ONLY`; the destination session opens read-write and its policy check
   runs inside the import transaction;
4. bind the destination policy authorization to the identity just read;
5. execute every statement of the operation.

Invariants:

- `BoundSession` owns the connection; there is no accessor that returns it to a
  pool and no `Clone`.
- `identity()` returns the identity read at open; it is never re-derived from the
  profile.
- `assert_binding` compares only `identity_key`. `backend_pid` and `backend_start`
  are audit fields and legitimately change on reconnect.
- Any reconnect constructs a new `BoundSession` and therefore a new authorization.
  There is no code path that reuses an old authorization after a reconnect.
- The source session's read-only state is asserted at open and re-asserted after
  every `BEGIN` because a transaction-scoped setting does not survive a new
  transaction.

Limits, recorded rather than eliminated:

- a malicious server can lie about every identity value it reports;
- a network path that is already redirected before the first query is undetected;
- RLS policies and generated-column expressions can call functions with effects
  outside the read-only transaction, bounded only by the role's privileges;
- a compromised transport (TLS downgrade or termination) is outside what an
  application-level identity check can detect.

---

## 6. Selection specification

The spec is YAML at M4.0 and is validated into core types with
`#[serde(deny_unknown_fields)]`. YAML is chosen because it is declarative, comment
friendly, and already the repository's documentation style; the CLI never parses
YAML into SQL directly.

### 6.1 Shape

```yaml
version: 1
source: prod-profile-id
destination: local-profile-id

selection:
  roots:
    - table: public.orders
      filter:
        - column: order_id
          op: eq
          value: 12345
  relationships:
    depth: 2
    direction: parents        # parents | both

limits:
  max_rows_per_table: 1000
  max_total_rows: 20000
  max_depth: 3
  max_bytes: 33554432         # 32 MiB of masked row payload
  timeout_ms: 60000

masking:
  policy: development-default@1

execution:
  acknowledgement_ttl_ms: 900000
```

### 6.2 Rules

- `version` must be `1`; any other value is `unsupported_spec_version`.
- `source` and `destination` are connection ids that must already exist in the
  connection store. They must differ.
- `selection.roots` must be non-empty. There is no "all tables" form.
- Filters are `eq`, `in`, `gte`, `lte` only. `in` takes a list (bounded to 100
  values). Values are always bound parameters.
- Identifiers are validated against the catalog: the schema and table must exist
  and be allowed by §4.3; the filter column must exist and be of a comparable type.
  A validated identifier is emitted with `quote_ident`. No other user string reaches
  SQL text.
- `relationships.direction` defaults to `parents`. `both` pulls children as well and
  is visibly labelled in the plan.
- Limits are mandatory; omitted limits take the built-in defaults, and the defaults
  are shown in the plan.
- `masking.policy` must name an installed policy version. `development-default@1`
  is built in.

### 6.3 Limit behaviour

Limits are evaluated during planning (estimates and catalog sizes) and enforced
during extraction (exact counts). If a limit is exceeded at either stage:

- the plan is emitted with `complete: false` and the failing limit identified;
- `execute` refuses an incomplete plan with `limit_exceeded` and imports nothing;
- the CLI prints a remediation hint (narrow the filter, reduce depth, raise the
  limit deliberately).

M4 never silently truncates. An inconsistent subset is worse than no subset.

---

## 7. PostgreSQL relationship graph

### 7.1 Discovery

One catalog query family over these relations, restricted to `current_database()`:

| Source | Used for |
| --- | --- |
| `pg_namespace` | schema names |
| `pg_class` | tables, relkind, RLS flag, partitioning |
| `pg_attribute` | columns, types, generated/identity flags, nullability |
| `pg_constraint` (`contype='f'`) | foreign keys, composite column order |
| `pg_constraint` (`contype='p'`, `'u'`) | primary and unique keys |
| `pg_index` | PK/unique index membership and order |
| `pg_attrdef` | generated column expressions (presence only) |
| `pg_type` | OIDs for type classification |

All queries run inside the read-only source transaction, with `SET LOCAL
search_path = pg_catalog` so an operator's `search_path` cannot shadow a catalog
relation.

### 7.2 Graph model

```rust
pub struct QualifiedName { pub schema: String, pub table: String }

pub struct ColumnInfo {
    pub name: String,
    pub type_oid: u32,
    pub type_name: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub is_generated: bool,   // GENERATED ALWAYS AS ... STORED
    pub is_identity: bool,    // GENERATED ... AS IDENTITY
    pub is_unique: bool,
}

pub struct TableNode {
    pub name: QualifiedName,
    pub relkind: char,          // 'r' or 'p'
    pub rls_active: bool,
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub unique_keys: Vec<Vec<String>>,
    pub has_row_identity: bool,
}

pub struct ForeignKeyEdge {
    pub name: String,
    pub child: QualifiedName,
    pub parent: QualifiedName,
    pub child_columns: Vec<String>,   // order matters
    pub parent_columns: Vec<String>,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub nullable: bool,               // any child column nullable
}

pub struct SchemaGraph {
    pub tables: BTreeMap<QualifiedName, TableNode>,
    pub foreign_keys: Vec<ForeignKeyEdge>,
    pub schema_hash: String,
}
```

### 7.3 Policies

**Parent dependencies.** A selected child must have its referenced parent rows, or
the destination import violates the FK. Default traversal is therefore
`parents`: from each root, follow outgoing FKs to the referenced tables, up to
`depth`, and select the referenced rows by their key values.

**Child dependencies.** Pulling children is optional (`direction: both`). It exists
for completeness (for example, order lines for selected orders) and is never
implied.

**Stable row identity.** A row is identified by its primary key tuple. If a table
has no primary key, its first non-null unique key is used. If it has neither, M4
refuses the table (`no_row_identity`) unless the policy file names an explicit
`row_identity` column set for it, which must be non-null and unique in the source
catalog. There is no "use ctid" option: `ctid` is not stable across a vacuum.

**Deduplication.** Rows are deduplicated within a table by the resolved identity
tuple. Traversal is breadth-first, and a visited node is not re-expanded beyond its
first encounter, so cycles terminate. Depth counts edges from a root.

**Cyclic relationships.** A cycle is legitimate in the data model (for example a
self-referencing `employees.manager_id`). Extraction handles it by identity
deduplication. Import handles it in one of two ways:

1. every FK edge in the cycle is `DEFERRABLE` → `SET CONSTRAINTS ALL DEFERRED` for
   the import transaction, then insert in any order;
2. at least one edge is not deferrable → **refuse** the plan
   (`cyclic_fk_requires_deferrable`).

M4 never sets `session_replication_role = replica`, never drops or disables a
constraint, and never truncates a table to make an import succeed.

**Tables without primary keys.** Refused unless an explicit row identity is
declared, as above.

**Partitioned tables.** A partitioned parent (`relkind = 'p'`) is planned as a
single logical table and extracted from its leaves; the import writes to the parent
and lets PostgreSQL route. The plan records the leaf set and their `relfilenode`
schema hash so a partition change between plan and execute is schema drift.

**Generated columns.** `GENERATED ALWAYS AS (expr) STORED` columns are never
inserted; the destination recomputes them. They are still read for dependency
discovery and are marked `is_generated` in the plan.

**Identity columns.** `GENERATED ALWAYS AS IDENTITY` values are inserted with
`OVERRIDING SYSTEM VALUE` when the column is part of a key that other rows
reference, and the owned sequence is raised with `setval` in a separate,
idempotent step after the row transaction commits (see 12.4). The sequence must
use `CACHE 1` and the destination role must own it; otherwise the rows still
import but the run reports `reconciliation_required`. If the column is the table's
only key and no other table references it, M4 omits it and lets the destination
generate values, which is safer but changes the identity values; the plan states
which choice was made.

**Unique constraints after masking.** Masking must be injective on the selected
values for every unique key. A collision is `mask_collision` and refuses the run;
M4 never deduplicates or overwrites to make a unique constraint pass.

---

## 8. Masking engine

This is the second mandatory security gate.

### 8.1 Pipeline position

```
source row → column classification → mask transform → type-cast validation
          → in-memory masked row → destination bind parameters
```

No plaintext row value is written to disk, a log, the plan, an audit record, or a
CLI argument. Masking happens before the value leaves the extractor's per-row
scope.

### 8.2 Column classification

Every column of every selected table gets exactly one disposition:

| Class | Rule |
| --- | --- |
| `primary_key` | Explicit policy rule required; usually `deterministic_pseudonym` for text keys or `keep` for opaque surrogate keys |
| `foreign_key` | Inherits the referenced column's strategy so joins survive masking |
| `masked` | Explicit strategy |
| `keep` | Explicit `keep` only; never a default |
| `unknown` | No explicit rule → **refuses the run** (`unmapped_column`) |

Name heuristics (`email`, `phone`, `name`, …) may *propose* a rule when an operator
authors a policy, but a heuristic is not authorization. At run time, a column
without an explicit rule in the installed policy is refused.

Type-risky columns — `json`, `jsonb`, `bytea`, `xml`, arrays, `tsvector`, and
user-defined types — are refused unless the policy gives an explicit `redact` or
`null` rule, because M4 cannot know what is inside them.

### 8.3 Strategies

| Strategy | Output | Use |
| --- | --- | --- |
| `keep` | unchanged | Explicitly approved non-sensitive column |
| `redact` | fixed token per column (`REDACTED`) | Free text, JSON, unknown content |
| `null` | NULL | Nullable columns where absence is acceptable |
| `deterministic_pseudonym` | keyed HMAC encoded to the column's type | Text identifiers, names, emails, phones |
| `deterministic_email` | `<pseudonym>@example.invalid` | Email columns |
| `deterministic_phone` | valid-format synthetic number | Phone columns |
| `date_shift` | date ± a deterministic per-row offset within a policy bound | Dates of birth, timestamps |
| `amount_jitter` | numeric ± a deterministic bounded proportion | Business amounts |
| `fixed` | a policy-supplied constant | Enumerations that must stay valid |

Authentication tokens, password hashes, government identifiers and binary blobs are
never `keep`: the built-in policy maps them to `redact`/`null`, and a policy that
tries to `keep` them is rejected at load time.

### 8.4 Determinism and keys

`deterministic_pseudonym(key, table, column, identity)` =
`base32(HMAC-SHA256(mask_key, canonical(table || column || identity)))`, truncated
and encoded to fit the destination type. The same identity and key always produce
the same value, so a masked primary key and every masked foreign key that
references it stay joined.

- The mask key is 32 random bytes from a CSPRNG (the OS entropy source via
  `getrandom`, never `rand::random` seeded from a clock) stored in the OS keyring
  under service `dbpinball-safe-sync`, account `mask-key:<key_id>`.
- The key id, not the key, appears in the plan and the audit record.
- The key never appears in the spec, a CLI argument, a plan file, a log, the audit
  record, the desktop state, or an export.
- `dbpinball safe-sync key rotate` creates a new key id. Rotating the key changes
  every pseudonym and therefore requires a fresh destination; the plan rejects a
  rotation between plan and execute.
- Tests inject a synthetic key through an explicit `MaskKeyProvider::ephemeral`
  constructor compiled only under `cfg(any(test, feature = "test-keys"))`. A test
  key is never the production key and is never written to the repository.

### 8.4A `development-default@1` (approved first policy)

The first policy is deliberately small. It covers the one table in Slice 1 and
refuses everything it does not name.

| Table | Column | Type | Disposition | Output |
| --- | --- | --- | --- | --- |
| `public.accounts` | `id` | `uuid` | `keep` (explicit, allowed because it is a non-personal surrogate key and the declared row identity) | unchanged UUID |
| `public.accounts` | `email` | `text` | `deterministic_email` | `<16 base32>@example.invalid` |
| `public.accounts` | `full_name` | `text` | `deterministic_pseudonym` | `Anon <10 base32>` |
| any other table | any column | any | none | **refused** (`unmapped_column`) |
| `public.accounts` | any column not listed | any | none | **refused** (`unmapped_column`) |

Required dispositions, restated as rules:

- `id` is `keep` **only** because the policy explicitly permits it and it is the
  declared row identity. A UUID that is a natural identifier (for example an email
  address, or a value derived from PII) is not covered by this rule and must be
  masked by an explicit rule; absent one, it is refused.
- `email` is always `deterministic_email`.
- `full_name` is always `deterministic_pseudonym`.
- an unexpected column refuses, even if it is nullable, has a default, or looks
  harmless;
- a missing mask key refuses (`mask_key_missing`), and the run does not fall back
  to any unkeyed hash;
- an invalid or unparsable policy refuses (`mask_policy_invalid`);
- an unsupported data type refuses (`unsupported_type`).

#### Algorithm

Let `mask_key` be 32 bytes and `identity` be the declared row identity rendered as
`col1=value1,col2=value2` in primary-key order, NFC-normalised and UTF-8 encoded.

```text
domain = "dbpinball/safesync/v1"
material = domain || 0x1F || schema_name || 0x1F || table_name || 0x1F
           || column_name || 0x1F || identity
digest = HMAC-SHA256(mask_key, material)            // 32 bytes
token  = base32_lower_no_padding(digest)            // 52 ASCII chars
```

- **Synthetic email domain:** `example.invalid`. The reserved `.invalid` TLD
  guarantees the address can never resolve, so a masked value cannot accidentally
  deliver mail to a real mailbox.
- **Domain separation:** `domain`, a `0x1F` separator, and the schema/table/column
  names are all inside the HMAC input. The same identity value in two different
  columns or tables therefore produces two different outputs, so a masked value
  cannot be correlated across fields.
- **Email:** local part = `token[..16]` (80 bits of digest), domain fixed. Total
  length 32 characters, within the 254-byte address limit and the 64-byte local-part
  limit.
- **Full name:** `"Anon " + token[..10]` (15 ASCII characters).
- **Type and length compatibility:** if the destination column is shorter than the
  output, the policy may declare `max_len` for that column, in which case the token
  is truncated deterministically; otherwise the run refuses
  (`mask_length_incompatible`). M4 never silently truncates and never pads with
  original data.
- **Unicode:** input is NFC-normalised before hashing. Output is pure ASCII, so it
  is valid in any database encoding and cannot be corrupted by a charset change.
- **Collisions:** 80 bits for email and 50 bits for the name token. The extractor
  checks all masked values for uniqueness against every unique key in the selected
  set; an actual collision refuses (`mask_collision`). The planner warns when the
  expected collision probability exceeds 1e-6.
- **Determinism across repeat executions:** the same key id + same identity →
  the same output. A repeated run of a completed plan is a no-op before any masking
  is recomputed.
- **Key rotation:** a new key id changes every output. A plan pinned to the old key
  id is refused (`mask_key_changed`); the destination must be regenerated into a
  clean database or schema.
- **Safe logging:** audit records and errors name the table, column, class and
  counts. They never contain the source value, the masked value, or the key.
  `CoverageReport` is the only per-column output and it carries no values.

#### What this policy does not claim

De-identified, not anonymous. A `full_name` pseudonym of 50 bits is not reversible
from the output without the key, but an attacker who already holds the key can test
candidate names. The policy therefore also refuses `keep` for email and name, and
would refuse a pseudonym rule on a low-cardinality column (for example a two-value
status) because frequency analysis would re-link it.

### 8.5 Uniqueness and collisions

For every unique key in the selected set, the extractor checks that masked values
are unique within the table and across the referencing tables. A collision refuses
the run. Because masked values are truncated encodings, the planner also estimates
the collision probability for the requested row count and warns when it is above
1e-6; the operator can raise the encoding width, but the run still refuses on an
actual collision.

### 8.6 Reidentification risk

Deterministic masking is **not** anonymization. A keyed HMAC over a low-entropy
value (a date of birth, a two-value status, a small postcode set) can be inverted by
dictionary or frequency analysis if the attacker has the key or a candidate list.
Therefore:

- the default policy refuses or redacts low-entropy identifier columns rather than
  pseudonymizing them;
- `date_shift` uses a secret offset so the true date is not recoverable from the
  output without the key;
- the documentation and the plan describe the output as **de-identified test
  data**, never as anonymous data.

### 8.7 Policy versioning

A policy is `name@version`. The version, the rules, and a hash of the rules appear
in the plan. Changing a policy invalidates an old plan. The built-in
`development-default@1` is shipped in the binary and cannot be edited in place; an
operator may create a new versioned policy file.

---

## 9. Destination safety

M4 reuses the M3 policy and adds the Safe Sync operation semantics.

### 9.1 Required state

The destination must be:

- a `Postgres` profile;
- enrolled as `verified_development` or `verified_disposable`;
- freshly verified **on the execution session** (§5.4);
- not the source (identity keys differ);
- not declared `production`.

Any other state is refused before a write. Profile names, hostnames, private IP
ranges and `localhost` are never used as evidence.

### 9.2 Destination privileges

The destination role should hold only what the import needs:
`CONNECT`, `USAGE` on the target schema, `INSERT` on the selected tables, and
`USAGE` on their sequences. M4 records whether the destination role is a superuser
and warns in the plan when it is; it does not refuse a legitimate local
development superuser, because the destination is already enrolled and disposable.

Identity preservation adds two requirements. The role must **own** the identity
sequence, because the safe reconciliation locks it with `ALTER SEQUENCE ... CACHE
<current>` before it reads and raises the sequence; and the sequence must use
**`CACHE 1`**, because a larger cache hands out a preallocated block that this
session can neither see nor revoke. A role that can insert but does not own the
sequence, or a sequence with a larger cache, still imports the rows, but the run
reports `sequence_status = reconciliation_required` and the sequence is left
untouched. `setval` alone would need `UPDATE`; ownership is what allows the lock,
and the lock plus the absence of a preallocated block is what makes the
read-then-raise safe against external writers.

### 9.3 Default import behaviour

The default is **do not touch data that is already there**:

| Target state | Default behaviour |
| --- | --- |
| Selected tables empty | Import |
| Selected tables contain rows | Refuse (`destination_not_empty`) |
| A run table exists and records this plan fingerprint as completed | No-op (`already_completed`) |
| `--replace` given | Delete only rows in the selected tables inside the transaction, then import |

`--replace` requires a second explicit confirmation, because it is the only mode
that removes local data. M4 does not offer `--force`.

### 9.4 Isolation

The recommended destination is a dedicated disposable database or a dedicated
schema. A staging schema is not used by default: the import writes into the target
tables inside one transaction, which is atomic and does not leave half-copied
staging data behind. Where a table is large enough that a single transaction is
impractical, `--batch` exists but is **not** atomic and must be resumable; it is
explicitly opt-in and is not part of the default path.

### 9.5 Schema compatibility

Before importing, M4 compares the plan's recorded column set and types with the
destination catalog. A missing table, a missing column, an incompatible type, or a
missing unique key is `schema_drift`; the run is refused and a new plan is required.
M4 never creates or alters destination tables automatically.

---

## 10. Dry-run contract

### 10.1 Two dry-run modes

| Mode | Reads source rows | Contains values | Use |
| --- | --- | --- | --- |
| `--metadata-only` (default for `plan`) | no | no | Fast review of tables, graph, limits, coverage |
| `--estimate` | yes, aggregated only | no | Row counts and byte estimates for the selected subset |

The default `plan` never reads sensitive rows. `--estimate` reads rows only to count
and measure them; the values are used for the estimate and discarded, never logged.
The plan states which mode produced each number.

### 10.2 Plan contents

The plan is JSON. It contains **no row values and no filter values**. Filter values
live only in the spec the operator controls; the plan carries a keyed digest of
them so the fingerprint binds the exact selection without storing it.

```json
{
  "plan_version": 1,
  "run_id": "uuid",
  "created_at": "rfc3339",
  "expires_at": "rfc3339",
  "spec_hash": "sha256",
  "plan_fingerprint": "sha256",
  "source":  { "identity_key": "...", "current_user": "...", "engine": "postgres" },
  "destination": { "identity_key": "...", "engine": "postgres", "category": "verified_development" },
  "selection": {
    "roots": [
      { "schema": "public", "table": "accounts",
        "filter_columns": ["id"], "filter_ops": ["eq"],
        "filter_digest": "hmac-sha256:...", "contains_values": false }
    ],
    "direction": "parents",
    "depth": 0
  },
  "tables": [
    { "schema": "public", "table": "accounts", "schema_hash": "sha256",
      "row_identity": ["id"], "rows_estimated": 1, "rls_active": false }
  ],
  "graph": { "edges": 0, "cycles": 0, "max_depth_used": 0, "direction": "parents" },
  "masking": { "policy": "development-default@1", "policy_hash": "sha256",
               "key_id": "k-...", "columns": 3, "unmapped": 0 },
  "import": { "conflict_policy": "refuse", "mode": "single_transaction",
              "sequence_adjustment": false },
  "constraints": { "order": ["public.accounts"], "deferred": [], "cyclic_refused": [] },
  "limits": { "max_rows_per_table": 1000, "max_total_rows": 20000,
              "max_depth": 3, "max_bytes": 33554432, "timeout_ms": 60000 },
  "checks": [ { "limit": "max_total_rows", "observed": 1, "ok": true } ],
  "complete": true
}
```

`contains_values` is always `false` in M4. It exists so a future milestone cannot add
values silently.

Because the plan has no filter values, `execute` takes **both** the plan and the
spec: `execute --plan <file> --spec <file> --ack <fingerprint>`. It re-parses the
spec, recomputes `filter_digest`, and requires equality. The two files together are
the execution input; neither alone is sufficient.

### 10.3 Plan fingerprint

```text
plan_fingerprint = sha256( canonical_json( fingerprint_document ) )
```

`canonical_json` sorts object keys and emits no insignificant whitespace.
`fingerprint_document` is the plan with `run_id`, `created_at`, `expires_at` and
`plan_fingerprint` removed, so the fingerprint is stable for the same plan and
changes for any other. It binds, explicitly:

| Component | Excluded? | Why |
| --- | --- | --- |
| `plan_version` | no | a format change must invalidate a plan |
| `spec_hash` | no | binds the spec shape |
| `source.identity_key` | no | binds the source server and database |
| `destination.identity_key` | no | binds the destination server and database |
| `selection.roots[].filter_digest` | no | binds the exact filter without storing it |
| `selection.direction`, `selection.depth` | no | binds the traversal policy |
| `tables[].schema_hash` | no | binds the catalog shape of every table |
| `tables[].row_identity` | no | binds how rows are identified |
| `masking.policy`, `masking.policy_hash`, `masking.key_id` | no | binds the masking behaviour |
| `import.conflict_policy`, `import.mode`, `import.sequence_adjustment` | no | binds what the destination will do |
| `limits`, `checks`, `complete` | no | binds the bounds and completeness |
| `constraints.order`, `constraints.deferred` | no | binds the insertion strategy |
| `run_id`, `created_at`, `expires_at` | yes | per-execution metadata, not plan content |

The fingerprint is a hex SHA-256. Printing it, storing it in a log, or putting it in
a PR comment reveals nothing about the data: it is a digest of a document that
already contains no values.

`filter_digest` is `HMAC-SHA256(mask_key, "dbpinball/safesync/filter/v1" || 0x1F ||
schema || 0x1F || table || 0x1F || canonical(filter))`. It is keyed so an observer
who sees two plans cannot brute-force a low-entropy filter value (for example an
order number) by recomputing an unkeyed hash. A key rotation therefore changes the
digest, and the plan must be regenerated.

### 10.4 Acknowledgement

`execute` requires `--ack <plan_fingerprint>` or an interactive confirmation that
prints the same fingerprint. The acknowledgement is bound to:

- the plan fingerprint;
- the destination identity key recorded in the plan;
- the source identity key recorded in the plan.

An acknowledgement is **not** authorization. The M3/M4 policy still decides, and
the session binding still must match. A changed plan, a changed destination, or an
expired plan invalidates the acknowledgement. A mismatched acknowledgement is
`acknowledgement_mismatch` and exits 2.

### 10.5 Expiration

Plans expire after `execution.acknowledgement_ttl_ms` (default 15 minutes). An
expired plan is refused (`plan_expired`, exit 8) and must be regenerated. The
expiry is deliberately shorter than M3's 10-minute freshness default only as a
bound; it is not the security control.

### 10.6 Idempotency contract

A repeated execution is a **no-op only when the recorded run result and the expected
destination state are still valid**. M4 never silently ignores destination drift.

The destination records, inside the data transaction, a row in
`dbpinball_safe_sync_runs`:

```text
run_id uuid primary key,
plan_fingerprint text not null,
spec_hash text not null,
source_identity_key text not null,
destination_identity_key text not null,
schema_hash text not null,
masking_policy text not null,
mask_key_id text not null,
conflict_policy text not null,
import_mode text not null,
status text not null,             -- completed | rolled_back
rows_written bigint not null,
started_at timestamptz not null,
finished_at timestamptz not null
```

| Condition | Behaviour | Exit / detail |
| --- | --- | --- |
| Completed run, identical fingerprint, destination tables still match the recorded state | **No-op**; report `already_completed` | 0 |
| Completed run, identical fingerprint, destination rows were changed since the run | Refuse; `destination_drifted` | 8 |
| Completed run, different fingerprint | Refuse; `destination_not_empty` unless `--replace` | 8 |
| Failed run (rolled back) | Nothing persisted; retry allowed with the same plan if identities still match | — |
| Interrupted run (process died) | Destination transaction rolled back; destination unchanged; retry allowed | — |
| Destination identity changed | Refuse; `destination_changed` | 4/8 |
| Source identity changed | Refuse; `source_changed` | 8 |
| Schema changed (either side) | Refuse; `schema_drift` | 8 |
| Masking policy changed | Refuse; `mask_policy_changed` | 8 |
| Mask key changed | Refuse; `mask_key_changed` | 8 |
| Plan expired | Refuse; `plan_expired` | 8 |
| Acknowledgement mismatch | Refuse; `acknowledgement_mismatch` | 2 |

"Destination still matches the recorded state" means: the selected tables contain
exactly the rows the completed run recorded (same row count and the run's
`run_id` marker is still present in `dbpinball_safe_sync_runs`), and no writer
changed them. M4 does not attempt a full table checksum by default; it records the
row count per table and the run marker. A change that keeps the row count identical
and does not touch the marker is not detected by the default check, which is
recorded as a residual risk. `--strict-idempotency` compares a per-table content
digest instead and is available for operators who need it.

The no-op decision is revalidated on every execution; a stale local journal never
authorizes a skip.

---

## 11. Execution state machine

```
Planned ──validate──► Validated ──execute──► Authorized
   │                     │                      │
   │                     │                      ├─► Extracting ──► Importing ──► Committed
   │                     │                      │        │             │
   │                     │                      │        └─ rollback ──┴─► RolledBack
   │                     │                      └─ refuse ──► Refused
   └─ expire ──► Expired
```

| State | Entered when | Persisted where |
| --- | --- | --- |
| `Planned` | `plan` produced a complete plan | local plan file |
| `Validated` | `validate` re-checked the plan against both live identities and the catalog | local plan file |
| `Authorized` | destination session bound and policy allowed | in memory + run journal |
| `Extracting` | source rows are being read and masked | run journal |
| `Importing` | destination transaction open, rows inserted | run journal + destination run table |
| `Committed` | destination transaction committed | destination run table, then run journal |
| `RolledBack` | any failure after import began | destination run table, then run journal |
| `Refused` | any gate refused before a write | run journal |
| `Expired` | plan TTL passed | none |

The **destination run table** (`dbpinball_safe_sync_runs`) is written inside the
same transaction as the data, so it is atomic with it. The **local run journal**
(`<config>/safe-sync/runs/<run_id>.json`) is best-effort and reconciled from the
destination when it disagrees.

---

## 12. Failure recovery

### 12.1 Transaction model

See §12.5 for the approved model. In one line: one destination transaction by
default, savepoints only for reporting, and `--batch` is opt-in, non-atomic and
never described as transactional. §12.4 covers sequences.

### 12.2 Failure matrix

| Failure | Detected by | Behaviour | Retryable? |
| --- | --- | --- | --- |
| Network interruption (source) | query error | abort source tx, retry extraction from the beginning | yes, same plan if identities unchanged |
| Network interruption (destination, before commit) | transaction error | server rolls back; reopen, re-assert, re-check policy | yes |
| Network interruption (destination, after commit) | connection error on commit | resolve status from the destination run table | no re-run; status reconciles |
| Source timeout | `statement_timeout` | `limit_exceeded`/`timeout`; raise limits deliberately or narrow | yes with a new plan |
| Destination timeout | `statement_timeout` | rollback | yes |
| Authentication expiration | connection error | `connection`; re-authenticate | yes, new plan |
| Database restart (either side) | connection error | reconnect, re-assert identity, re-run policy | yes if identities match |
| Partial extraction | row-limit check | refuse before import; nothing written | yes with a new plan |
| Partial masking | per-row error | refuse; nothing written | yes |
| Unique constraint failure | destination error | rollback; report the table | after a policy or selection change |
| FK failure | destination error | rollback; report the edge | after a selection change |
| Schema drift | catalog hash mismatch | refuse | new plan |
| Concurrent local writer | destination advisory lock or constraint | rollback or `busy` | yes |
| Retried execution | run table fingerprint | no-op if completed | no-op |
| Process crash | transaction state | rollback; destination unchanged | yes |
| Insufficient disk | OS/`pg` error | rollback | after freeing space |

### 12.3 Never

- never retry after a successful commit without `status` reconciliation;
- never reuse a stale destination authorization across a reconnect;
- never disable constraints, truncate, or use `session_replication_role` to make a
  run pass;
- never report success for a run whose plan was incomplete.

### 12.4 Sequences (approved caveat)

PostgreSQL sequence operations are **not** rollback-atomic. `nextval()` and
`setval()` change sequence state outside the transaction: a rollback restores table
rows but leaves the sequence advanced or reset. `ALTER SEQUENCE ... RESTART` is
transactional, but it also blocks concurrent `nextval` callers and can hand out a
value that already exists if the sequence is not the only allocator.

Consequences, fixed for M4:

- **Slice 1 does not touch sequences at all.** `public.accounts.id` is a `uuid`
  supplied by the application, not a `serial` or identity column, so no sequence is
  involved and the import is fully transactional.
- **M4.6 default import mode is `single_transaction` and `sequence_adjustment:
  false`.** The plan records both, and both are part of the fingerprint. When the
  selection contains an identity column, the plan records
  `single_transaction_plus_sequence` and `sequence_adjustment: true` instead.
- A later milestone that imports into `GENERATED ... AS IDENTITY` columns must treat
  the sequence step as an independent, idempotent operation: record the intended
  sequence value in the run table, apply it after a successful data commit, and
  re-apply it on a re-run of `execute` if it did not complete. It must never claim
  that `setval()` participated in the data transaction.
- The safe reconciliation locks the sequence with `ALTER SEQUENCE ... CACHE
  <current>` inside the same transaction as the read and the `setval`, so an
  external `nextval`, `setval` or `ALTER SEQUENCE` blocks while the value is read
  and raised. The backend takes a `ShareRowExclusiveLock` on the sequence;
  PostgreSQL documents that `ALTER SEQUENCE` blocks those calls.
- Only `CACHE 1` sequences are reconciled. A larger cache preallocates a block
  that this session cannot see or revoke, and the PostgreSQL documentation says
  other backends keep using their cached values, so automatic reconciliation is
  refused for them instead of relying on undocumented invalidation. A two-session
  test on PostgreSQL 18 recorded both halves: after a bare `setval` a cached
  backend still returned `2`, while the `ALTER`-based step made a blocked
  `nextval` resume above the reconciled value. The check is a snapshot taken
  before the lock, so a cache changed by an uncoordinated writer mid-run is
  outside the supported mode. The cache check runs before the value is compared,
  so a `CACHE` greater than 1 is `reconciliation_required` even when the sequence
  is already ahead of the imported maximum. The lock also requires ownership of
  the sequence; a role that does not own it reports `reconciliation_required` and
  leaves the sequence untouched.
- A `reconciliation_required` or `pending` run is not full success: the JSON
  envelope is `ok: false` with `error.code = "sequence_reconciliation_required"`,
  `data` still carries the committed run so the rows are not hidden, the CLI
  exits 6, and the audit record carries the status. The retry is a re-run of
  `execute` with the same plan, which is idempotent and re-runs reconciliation;
  `status` reports the persisted state but does not re-apply it.
- M4 must not claim "atomic import" for any run whose plan has
  `sequence_adjustment: true`. The correct description is "transactional row
  writes with a separate, idempotent sequence step".

### 12.5 Transaction model, restated

- **Default (Slice 1 and M4.6):** one destination transaction for the whole import,
  `BEGIN` before the destination policy check, `COMMIT` only after every table and
  the run row are written. Any failure rolls the transaction back.
- **Savepoints:** one per table, used only to report which table failed; the whole
  transaction still rolls back.
- **Batches (`--batch`):** opt-in, one transaction per batch, **not atomic**. It is
  never described as transactional, and it requires the durable run journal plus a
  `status` reconciliation to resume.
- A global constraint-disable operation (`session_replication_role = replica`,
  `ALTER TABLE ... DISABLE TRIGGER ALL`) is never used, in any mode.

---

## 13. CLI contracts

### 13.1 Commands

```bash
dbpinball safe-sync inspect --source <id> --json
dbpinball safe-sync plan    --spec <file> [--estimate] --json
dbpinball safe-sync validate --plan <file> --spec <file> --json
dbpinball safe-sync execute --plan <file> --spec <file> --ack <fingerprint> [--replace] --json
dbpinball safe-sync status  --run <id> --json
dbpinball safe-sync key rotate --json
```

`validate` and `execute` take the spec as well as the plan because the plan
deliberately stores no filter values. The spec is the operator's input; the plan is
the reviewed artifact. Together they are the execution input.

All commands call `dbpinball_core::safesync` services. No synchronization logic
lives in the CLI.

### 13.2 Arguments and files

| Command | Required | Optional | Writes |
| --- | --- | --- | --- |
| `inspect` | `--source` | `--json` | nothing |
| `plan` | `--spec` | `--estimate`, `--out <file>`, `--json` | the plan file |
| `validate` | `--plan`, `--spec` | `--json` | nothing |
| `execute` | `--plan`, `--spec`, acknowledgement | `--replace`, `--yes`, `--strict-idempotency`, `--json` | destination data, run table, audit |
| `status` | `--run` | `--json` | nothing |
| `key rotate` | — | `--json` | a new keyring entry |

Spec and plan files are JSON or YAML for the spec and JSON for the plan. Paths are
local files; the plan never embeds a credential.

### 13.3 Credentials

Passwords come from the existing `CredentialProvider` (`keyring` by default,
`--credential-source env` for tests). M4 adds no new credential path and never
accepts a password argument.

### 13.4 Output envelopes and exit codes

Every command reuses the existing envelope:

```json
{ "ok": true, "data": { ... }, "error": null }
{ "ok": false, "data": null, "error": { "code": "policy_refusal", "message": "..." } }
{ "ok": false, "data": { ... }, "error": { "code": "sequence_reconciliation_required", ... } }
```

The third shape is the only one that is `ok: false` and carries `data`: the row
import committed, so the report is real, but the sequence step did not complete
and the run must not read as success.

Exit codes reuse the documented map:

| Code | Meaning | Typical Safe Sync cause |
| --- | --- | --- |
| 0 | success | plan produced, execute committed with sequence reconciliation complete or not required, already completed |
| 2 | usage | invalid spec, missing argument |
| 3 | connection | source/destination unreachable, auth expired |
| 4 | policy refusal | destination not enrolled or not writable |
| 5 | operation | incomplete plan, limit exceeded, mask collision, schema drift, cyclic FK, source privileged |
| 6 | committed, sequence reconciliation required | rows committed but the identity sequence could not be raised (non-owner, lock timeout, missing privilege, `CACHE` greater than 1); re-run execute to retry |
| 7 | not found | unknown run id or connection id |
| 8 | conflict | destination not empty, schema drift, plan expired or already used differently |
| 9 | busy | concurrent run lock |

The Safe Sync detail object carries a stable `safesync_code` (for example
`limit_exceeded`, `unmapped_column`, `mask_collision`, `mask_key_missing`,
`mask_policy_invalid`, `mask_key_changed`, `mask_policy_changed`,
`destination_not_empty`, `destination_drifted`, `plan_expired`,
`acknowledgement_mismatch`, `schema_drift`, `source_privileged`,
`source_not_read_only`, `unsupported_type`, `unsupported_relation`,
`unsupported_engine`) so a script can branch without parsing prose.

### 13.5 Interaction, cancellation, progress

- With `--json`, stdout receives exactly one envelope and nothing else. Progress
  goes to stderr as newline-delimited JSON events.
- Without a TTY, `execute` requires `--ack` and `--yes`; it never prompts.
- `Ctrl-C`/SIGINT cancels: the source transaction is rolled back and the destination
  transaction is rolled back before exit 5 with `cancelled` in the detail.
- Progress events reuse `ProgressSink` with new phases: `planning`, `extracting`,
  `masking`, `importing`, `committing`, `done`.

---

## 14. Desktop adapter contracts

The desktop is a thin adapter. It is implemented only after the CLI contract is
tested (M4.8).

| Tauri command | Core call | Decides anything? |
| --- | --- | --- |
| `safesync_inspect` | `safesync::inspect` | no |
| `safesync_plan` | `safesync::plan` | no |
| `safesync_validate` | `safesync::validate` | no |
| `safesync_execute` | `safesync::execute` | no |
| `safesync_status` | `safesync::status` | no |
| `safesync_key_rotate` | `safesync::rotate_key` | no |

UI responsibilities are presentation only: source and destination pickers, a
relationship preview, a masking-coverage table, the dry-run report, the refusal
message with its stable code, the plan fingerprint, an explicit confirmation bound
to that fingerprint, progress, cancellation, and recovery guidance. There is no UI
toggle that weakens a policy, and no UI-only security decision.

---

## 15. Residual risks

M4 documents these and does not claim to eliminate them:

1. **Server trust.** A malicious or compromised PostgreSQL server can misreport
   `system_identifier` and every other identity value. Identity is evidence, not
   proof.
2. **Network substitution before the first query.** If the connection is already
   redirected, the first identity query talks to the wrong server. Identity binding
   removes the *second* substitution window, not the first.
3. **Deterministic reidentification.** Keyed pseudonyms of low-entropy values can be
   re-linked by an attacker with a candidate list. Output is de-identified test
   data, not anonymous data.
4. **Operator intent.** Enrollment is authorization. An operator can enroll a
   production database as development; M4 cannot know better.
5. **RLS semantics.** Rows outside the source role's RLS policy are invisible, so a
   selected subset may omit parents; that is detected as an FK failure, not
   prevented.
6. **Function side effects.** RLS policies and generated-column expressions can call
   functions. Only the source role's own privileges bound what those functions can
   do outside the database.
7. **Process memory.** Masked rows are held in memory up to `max_bytes`; the
   operating system and swap are outside M4's control.
8. **Single large transaction.** The default atomic import can hold locks and memory
   proportional to the subset. `--batch` exists but is not atomic and is explicitly
   opt-in.
9. **No background or incremental sync.** M4 is a one-shot workflow. It is not
   replication, and it does not track source changes.
10. **Redis/other stores.** M4 covers PostgreSQL relational data only.

---

## 16. Slice 1 acceptance specification (approved)

Slice 1 is the first runtime slice and the only thing that may be implemented next.
It is one PostgreSQL table with no foreign keys, on two isolated disposable
clusters.

### 16.1 Fixture

```sql
-- source cluster
CREATE TABLE public.accounts (
  id uuid PRIMARY KEY,
  email text NOT NULL,
  full_name text NOT NULL
);
-- three synthetic rows; emails use example.com only

-- destination cluster
CREATE TABLE public.accounts (
  id uuid PRIMARY KEY,
  email text NOT NULL,
  full_name text NOT NULL
);
```

Two separate clusters (different `system_identifier`), a restricted source role
(`SELECT` only, not superuser, not `rolbypassrls`), and an enrolled
`verified_development` destination. All data is synthetic.

### 16.2 Acceptance matrix

Each row is one test that asserts real behaviour, not a command's exit code alone.
Tests marked *dest* inspect the actual destination rows.

| # | Requirement | Test | Assertion |
| --- | --- | --- | --- |
| 1 | Source role is restricted | `slice1_source_role_is_restricted` | `rolsuper=false`, `rolbypassrls=false`, no write privilege on `public.accounts`; a role with `UPDATE` is refused |
| 2 | Source transaction is read-only | `slice1_source_is_read_only` | `INSERT` on the source session fails with SQLSTATE `25006` |
| 3 | Source identity bound to the extraction session | `slice1_source_identity_is_bound` | `plan.source.identity_key == session.identity_key`; a session from the other cluster fails `assert_binding` |
| 4 | Destination identity bound to the write session | `slice1_destination_identity_is_bound` | the policy is evaluated with the execution session's identity; a session from the other cluster is `destination_changed` |
| 5 | Unknown destination refused | `slice1_unenrolled_destination_exits_4` | exit 4, `not_enrolled`, zero rows written |
| 6 | Protected destination refused | `slice1_production_destination_exits_4` | `production_like` enrollment → `destination_not_allowed` |
| 7 | Same source/destination refused | `slice1_same_database_exits_4` | identical identity keys → `same_source_and_destination` |
| 8 | Selection is explicitly bounded | `slice1_limits_are_enforced` | a spec exceeding `max_total_rows` produces `complete=false` and `limit_exceeded`, zero rows written |
| 9 | Email masked before persistence | `slice1_email_is_masked` *(dest)* | every destination email matches `^[a-z2-7]{16}@example\.invalid$` and none equals a source email |
| 10 | Full name masked before persistence | `slice1_full_name_is_masked` *(dest)* | every destination name matches `^Anon [a-z2-7]{10}$` and none equals a source name |
| 11 | UUID handling follows policy | `slice1_uuid_is_kept_by_explicit_rule` *(dest)* | source and destination `id` values are equal, and a policy that omits the `id` rule refuses |
| 12 | No plaintext in intermediate files | `slice1_no_plaintext_artifacts` | after plan+execute, no file under the config dir contains a source email or name |
| 13 | Dry-run has no sensitive values | `slice1_plan_has_no_values` | the plan JSON does not contain any source email, name, or UUID, and `contains_values=false` |
| 14 | CLI JSON has no secrets | `slice1_cli_output_has_no_secrets` | no password, mask key, source email, or source name appears in stdout or stderr |
| 15 | Destination import is transactional | `slice1_import_is_transactional` | a failure injected after the first row leaves zero destination rows |
| 16 | Completed repeat run is idempotent | `slice1_repeat_run_is_noop` *(dest)* | second execute exits 0 with `already_completed`, row count unchanged |
| 17 | Schema drift is detected | `slice1_schema_drift_is_refused` | a destination column type change → `schema_drift`, zero rows written |
| 18 | A failed run never reports success | `slice1_failure_is_not_success` | a forced mask collision exits 5 and the envelope is `ok:false` |
| 19 | Real destination contents inspected | all *(dest)* tests | queries run against the destination, not the CLI response |
| 20 | No source mutations | `slice1_source_is_unchanged` | a row-by-row snapshot (including `xmin`-independent content hash) is identical before and after |

### 16.3 Negative tests (deliberate refusals)

Each must fail closed, with the stated code, and write nothing:

| Test | Condition | Expected |
| --- | --- | --- |
| `slice1_password_argument_is_rejected` | `--password` passed | exit 2, clap error |
| `slice1_unknown_spec_field_is_rejected` | an extra YAML key | exit 2, `usage` |
| `slice1_missing_mask_key_is_refused` | keyring entry removed | exit 5, `mask_key_missing` |
| `slice1_invalid_policy_is_refused` | policy version unknown | exit 5, `mask_policy_invalid` |
| `slice1_unmapped_column_is_refused` | an extra `phone` column without a rule | exit 5, `unmapped_column` |
| `slice1_unsupported_type_is_refused` | a `jsonb` column without a rule | exit 5, `unsupported_type` |
| `slice1_ack_mismatch_is_refused` | wrong fingerprint | exit 2, `acknowledgement_mismatch` |
| `slice1_expired_plan_is_refused` | TTL passed | exit 8, `plan_expired` |
| `slice1_destination_not_empty_is_refused` | pre-existing row | exit 8, `destination_not_empty` |
| `slice1_source_privileged_is_refused` | superuser source role | exit 5, `source_privileged` |
| `slice1_view_is_refused` | a view in the selection | exit 5, `unsupported_relation` |
| `slice1_mysql_source_is_refused` | MySQL profile | exit 5, `unsupported_engine` |

### 16.4 Definition of done for Slice 1

- all 20 acceptance rows and all 12 negative tests pass against two real disposable
  PostgreSQL clusters, in CI, with a non-zero test count;
- no source row is modified;
- the destination contains masked rows only;
- the plan contains no values;
- a repeat run is a no-op;
- the docs still say Safe Sync is planned;
- `cargo fmt`, `clippy -D warnings`, the workspace tests, the boundary check and the
  docs checks pass.

