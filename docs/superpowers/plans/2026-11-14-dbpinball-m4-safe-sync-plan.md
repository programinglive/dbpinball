# DBPinball M4 — PostgreSQL Safe Production-to-Local Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a bounded, masked, verifiable PostgreSQL production-to-development sync in the Rust core, exposed through a CLI, so a developer can reproduce a bug locally without copying unmasked personal data.

**Architecture:** A new `dbpinball-core::safesync` module owns the whole pipeline. Two `BoundSession` values — source and destination — each own one `sqlx::PgConnection`, read their identity on that connection, and run every statement on it, so verification and execution share a session. The planner reads `pg_catalog` into a foreign-key graph, bounds traversal with explicit limits, and produces a value-free plan with a SHA-256 fingerprint. Masking is mandatory and fail-closed: every selected column needs an explicit policy disposition. The destination import is one transaction, gated by the M3 policy using the execution session's identity.

**Tech Stack:** Rust 2021, `sqlx` (PostgreSQL), `serde`/`serde_yaml`, `sha2`, `hmac`, `base32`, `clap`, OS keyring via the existing `CredentialProvider`.

**Spec:** `docs/superpowers/specs/2026-11-14-dbpinball-m4-safe-sync-architecture.md`

## Global Constraints

- PostgreSQL source and destination only. Both must report `DbEngine::Postgres`; anything else is `unsupported_engine`.
- `dbpinball-core` must not depend on `tauri` or `clap`. `scripts/check-boundaries.cjs` must keep passing.
- The CLI must call `dbpinball_core::safesync` services and contain no synchronization logic.
- No new credential path: passwords come from `CredentialProvider` (`keyring` default, `env` for tests). No `--password` argument.
- Every identifier that reaches SQL is validated against `pg_catalog` and emitted with `quote_ident`; every value is a bound parameter.
- No plaintext source value is written to a file, a log, a plan, an audit record, or a CLI argument.
- Reuse the documented exit-code map in `crates/dbpinball-cli/src/exit.rs`: 0, 2, 3, 4, 5, 7, 8, 9. Do not renumber.
- Real-database tests are gated by `--features integration` and `DBPINBALL_TEST_PG_SOURCE_URL` / `DBPINBALL_TEST_PG_DEST_URL`. Skipped tests are not acceptance.
- Tests use synthetic data only. No production credentials, databases or dumps.
- All builds and CI run on the self-hosted runner on `bw-tools`. Never target MMCH.
- M4 is a one-shot workflow. No background sync, no scheduling, no replication.

## File Structure

Created:

```text
crates/dbpinball-core/src/safesync/mod.rs        module exports
crates/dbpinball-core/src/safesync/spec.rs       SafeSyncSpec, Selection, Limits, validation
crates/dbpinball-core/src/safesync/session.rs    BoundSession, SessionIdentity, SessionRole
crates/dbpinball-core/src/safesync/graph.rs      SchemaGraph, TableNode, ForeignKeyEdge, discovery
crates/dbpinball-core/src/safesync/planner.rs    SelectionPlan, ordering, cycle analysis
crates/dbpinball-core/src/safesync/mask.rs       MaskingPolicy, Masker, classification
crates/dbpinball-core/src/safesync/key.rs        MaskKey, keyring-backed key provider
crates/dbpinball-core/src/safesync/extract.rs    Extractor, masked rows, bounds enforcement
crates/dbpinball-core/src/safesync/plan.rs       DryRunPlan, fingerprint, expiry, acknowledgement
crates/dbpinball-core/src/safesync/import.rs     DestinationImporter, transaction, conflicts
crates/dbpinball-core/src/safesync/audit.rs      AuditRecord, run journal, destination run table
crates/dbpinball-core/src/safesync/service.rs    inspect, plan, validate, execute, status, rotate_key
crates/dbpinball-core/tests/safesync_integration.rs
crates/dbpinball-cli/src/commands/safe_sync.rs
crates/dbpinball-cli/tests/safesync_cli_integration.rs
src-tauri/src/safesync.rs
```

Modified:

```text
crates/dbpinball-core/src/security/policy.rs     SafeSync policy arms
crates/dbpinball-core/src/security/mod.rs        re-export new PolicyCode
crates/dbpinball-core/src/security/verify.rs     expose the identity query for session reuse
crates/dbpinball-core/src/lib.rs                 add `pub mod safesync;`
crates/dbpinball-core/Cargo.toml                 serde_yaml, sha2, hmac, base32
crates/dbpinball-cli/src/cli.rs                  SafeSync subcommand
crates/dbpinball-cli/src/commands/mod.rs         register the module
crates/dbpinball-cli/src/main.rs                 dispatch
src-tauri/src/lib.rs                             register the adapter commands
.github/workflows/ci.yml                         run the Safe Sync integration suite
website/roadmap.md, website/security/safe-sync-design.md, website/.vitepress/config.mts
```

## Review Focus

- **A nullable FK whose parent row is filtered out.** A reasonable person expects either the parent to be pulled in or a clear refusal — not a silent NULL child or a partial import. Pinned in Task 4 (extraction) and Task 7 (import).
- **A `jsonb` column with no explicit policy rule.** A reasonable person expects the run to refuse, because M4 cannot know what is inside it. Pinned in Task 5.
- **A `GENERATED ALWAYS AS (expr) STORED` column.** A reasonable person expects the destination to recompute it, not receive a supplied value. Pinned in Task 6.
- **Running the same plan twice.** A reasonable person expects the second run to be a no-op with no duplicates. Pinned in Task 7.
- **Ctrl-C during import.** A reasonable person expects the destination to be unchanged. Pinned in Task 7.

---

### Task 1 (M4.0): Security contracts and policy arms

**Prerequisites:** M3 merged at `e2b8496`.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/mod.rs`, `crates/dbpinball-core/src/safesync/spec.rs`
- Modify: `crates/dbpinball-core/src/lib.rs`, `crates/dbpinball-core/src/security/policy.rs`, `crates/dbpinball-core/src/security/mod.rs`

**Interfaces:**
- Consumes: `security::{DatabaseOperation, OperationContext, PolicyDecision, PolicyCode}`, `domain::connection::{DbConnection, DbEngine, TargetClass}`, `domain::error::{CoreError, ErrorCode}`.
- Produces:
  - `PolicyCode::{MissingSource, SourceAuthorized, SourceNotReadOnly, UnverifiableSource}` (`"missing_source"`, `"source_authorized"`, `"source_not_read_only"`, `"unverifiable_source"`).
  - `evaluate` arm: `SafeSyncPrepare` allows when `source` is present and identified, returning `SourceAuthorized`; denies `MissingSource` when absent, `UnverifiableSource` when unidentified, `SourceNotReadOnly` when the session is not read-only, and `SameSourceAndDestination` when a destination is supplied with the same key. `SafeSyncExecute` requires both a source identity (else `MissingSource`) and the unchanged M3 destination checks.
  - `spec::{SafeSyncSpec, Selection, RootSelection, FilterPredicate, FilterOp, FilterValue, Limits, MaskingSelection, ExecutionOptions}`.
  - `spec::SafeSyncSpec::parse_yaml(&str) -> Result<SafeSyncSpec, CoreError>` and `validate(&self) -> Result<(), CoreError>`.

- [ ] **Step 1: Write the failing policy tests**

In `crates/dbpinball-core/src/security/policy.rs` tests:

```rust
#[test]
fn safe_sync_prepare_needs_an_identified_source() {
    let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000);
    assert_eq!(evaluate(&context).code(), PolicyCode::MissingSource);
}

#[test]
fn safe_sync_prepare_allows_an_identified_source() {
    let source = identity("srv-1", "prod", 1_000);
    let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000)
        .with_source(&source);
    assert!(evaluate(&context).is_allowed());
    assert_eq!(evaluate(&context).code(), PolicyCode::SourceAuthorized);
}

#[test]
fn safe_sync_prepare_refuses_an_unidentified_source() {
    let mut source = identity("srv-1", "prod", 1_000);
    source.server.server_uuid = None;
    source.identity_key = None;
    let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000)
        .with_source(&source);
    assert_eq!(evaluate(&context).code(), PolicyCode::UnverifiableSource);
}

#[test]
fn safe_sync_prepare_refuses_equal_source_and_destination() {
    let source = identity("srv-1", "prod", 1_000);
    let context = OperationContext::new(DatabaseOperation::SafeSyncPrepare, 1_000)
        .with_source(&source).with_destination(&source);
    assert_eq!(evaluate(&context).code(), PolicyCode::SameSourceAndDestination);
}

#[test]
fn safe_sync_execute_needs_both_identities() {
    let destination = identity("srv-9", "dev", 1_000);
    let only_destination = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
        .with_destination(&destination);
    assert_eq!(evaluate(&only_destination).code(), PolicyCode::MissingSource);
}

#[test]
fn safe_sync_execute_requires_an_enrolled_destination() {
    let source = identity("srv-1", "prod", 1_000);
    let destination = identity("srv-9", "dev", 1_000);
    let context = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
        .with_source(&source).with_destination(&destination);
    assert_eq!(evaluate(&context).code(), PolicyCode::NotEnrolled);
}

#[test]
fn safe_sync_execute_allows_an_enrolled_development_destination() {
    let source = identity("srv-1", "prod", 1_000);
    let destination = identity("srv-9", "dev", 1_000);
    let enrolled = enrollment(destination.identity_key.as_deref().unwrap(),
        DestinationCategory::VerifiedDevelopment);
    let context = OperationContext::new(DatabaseOperation::SafeSyncExecute, 1_500)
        .with_source(&source).with_destination(&destination)
        .with_enrollment(&enrolled);
    assert!(evaluate(&context).is_allowed());
}
```

Replace the M3 test `safe_sync_is_explicitly_unsupported` with these seven.

- [ ] **Step 2: Run the policy tests to verify they fail**

Run: `cargo test -p dbpinball-core --lib security::policy`
Expected: FAIL with `MissingSource` not found / `unsupported` returned.

- [ ] **Step 3: Implement the policy arms**

Add `MissingSource`, `SourceAuthorized`, `SourceNotReadOnly`, `UnverifiableSource` to `PolicyCode` and to `as_str`. In `evaluate`, before the `is_write` check:

```rust
if context.operation == DatabaseOperation::SafeSyncPrepare {
    let Some(source) = context.source else {
        return PolicyDecision::Deny { code: PolicyCode::MissingSource,
            reason: "reading a protected source needs a source identity".to_string() };
    };
    if !source.server.is_identified() {
        return PolicyDecision::Deny { code: PolicyCode::UnverifiableSource, .. };
    }
    if !source.server.read_only.unwrap_or(false) {
        return PolicyDecision::Deny { code: PolicyCode::SourceNotReadOnly, .. };
    }
    if let Some(destination) = context.destination {
        if source.identity_key == destination.identity_key {
            return PolicyDecision::Deny { code: PolicyCode::SameSourceAndDestination, .. };
        }
    }
    return PolicyDecision::Allow { code: PolicyCode::SourceAuthorized, evidence: .. };
}

if context.operation == DatabaseOperation::SafeSyncExecute && context.source.is_none() {
    return PolicyDecision::Deny { code: PolicyCode::MissingSource, .. };
}
```

Remove `SafeSyncPrepare | SafeSyncExecute` from the `Unsupported` early return. `SafeSyncExecute` then flows through the existing write checks unchanged, so M3 destination verification is not altered.

- [ ] **Step 4: Run the policy tests to verify they pass**

Run: `cargo test -p dbpinball-core --lib security::policy`
Expected: PASS.

- [ ] **Step 5: Write the failing spec tests**

In `crates/dbpinball-core/src/safesync/spec.rs` tests, with `SafeSyncSpec::parse_yaml`:

```rust
#[test] fn rejects_unknown_fields() { /* "version: 1\nunexpected: true" -> err Usage */ }
#[test] fn rejects_an_empty_root_list() { /* roots: [] -> err Usage */ }
#[test] fn rejects_an_unsupported_version() { /* version: 2 -> err Unsupported */ }
#[test] fn rejects_equal_source_and_destination() { /* source == destination -> err Usage */ }
#[test] fn rejects_an_in_filter_over_100_values() { /* 101 values -> err Usage */ }
#[test] fn rejects_a_password_field() { /* "password: x" is an unknown field -> err */ }
#[test] fn applies_default_limits_when_omitted() { /* defaults: 1000/20000/3/32MiB/60000 */ }
```

- [ ] **Step 6: Run the spec tests to verify they fail**

Run: `cargo test -p dbpinball-core --lib safesync::spec`
Expected: FAIL — module does not exist.

- [ ] **Step 7: Implement the spec types**

`#[derive(Debug, Clone, Deserialize, Serialize)]` with `#[serde(deny_unknown_fields)]` on every struct. `Source`/`destination` are `String` connection ids. `Limits::default()` returns the documented values. `validate` checks: version `1`; non-empty roots; source != destination; every filter column non-empty; `op == In` values `1..=100`; `max_depth <= limits.max_depth`; `max_rows_per_table >= 1`; `timeout_ms >= 1000`.

- [ ] **Step 8: Run the spec tests to verify they pass**

Run: `cargo test -p dbpinball-core --lib safesync::spec`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/dbpinball-core/src/safesync crates/dbpinball-core/src/security crates/dbpinball-core/src/lib.rs
git commit -m "feat(safesync): add M4 spec types and Safe Sync policy arms"
```

**Failure modes:** a permissive deserializer that ignores unknown keys (breaks fail-closed); a `SafeSyncExecute` arm that still returns `Unsupported`.
**Acceptance:** policy tests pass; unknown spec fields are rejected; `dbpinball-core` still builds without `tauri`/`clap`.
**Review checkpoint:** confirm the `SafeSyncPrepare` arm does not require enrollment of the source (reading is not writing) and does not allow equal identities.
**Depends on:** nothing.

---

### Task 2 (M4.1): Bound sessions and identity binding

**Prerequisites:** Task 1.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/session.rs`
- Modify: `crates/dbpinball-core/src/security/verify.rs` (extract a reusable `identity_query` returning the same fields), `crates/dbpinball-core/Cargo.toml` if a direct `sqlx` connection type needs importing.

**Interfaces:**
- Consumes: `DbConnection`, `CredentialProvider`, `Limits`, `CoreError`.
- Produces:
  - `SessionRole::{Source, Destination}`.
  - `SessionIdentity { engine, identity_key, system_identifier, database_oid, database_name, current_user, backend_pid, backend_start, server_addr, server_port, transaction_read_only }` with `Serialize`.
  - `BoundSession::open(role, connection, credentials, limits) -> Result<BoundSession, CoreError>`.
  - `BoundSession::identity(&self) -> &SessionIdentity`.
  - `BoundSession::assert_binding(&self, expected: &SessionIdentity) -> Result<(), CoreError>`.
  - `BoundSession::connection(&mut self) -> &mut sqlx::PgConnection`.

- [ ] **Step 1: Write the failing unit tests**

```rust
#[test] fn a_mysql_profile_is_refused() { /* engine Mysql -> Unsupported */ }
#[test] fn a_destination_session_is_never_read_only() { /* identity.transaction_read_only == false */ }
#[test] fn a_source_session_sets_read_only() { /* transaction_read_only == true */ }
#[test] fn assert_binding_rejects_a_different_identity_key() { /* -> PolicyRefusal/Conflict */ }
#[test] fn assert_binding_ignores_backend_pid_and_start() { /* same key, different pid -> Ok */ }
```

Real-database tests in `crates/dbpinball-core/tests/safesync_integration.rs`:

```rust
#[tokio::test] async fn source_session_is_database_enforced_read_only() {
    // INSERT on the source session must fail with SQLSTATE 25006.
}
#[tokio::test] async fn a_superuser_source_role_is_refused() {
    // rolsuper true -> source_privileged.
}
#[tokio::test] async fn a_source_role_with_insert_privilege_is_refused() {
    // has_table_privilege(..., 'INSERT') true on a selected table -> source_privileged.
}
#[tokio::test] async fn a_reconnected_session_with_a_new_cluster_fails_binding() {
    // second cluster identity key != first -> destination_changed / source_changed.
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball-core --lib safesync::session` and `DBPINBALL_TEST_PG_SOURCE_URL=... DBPINBALL_TEST_PG_DEST_URL=... cargo test -p dbpinball-core --features integration --test safesync_integration source_session`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `BoundSession`**

Open one `sqlx::PgConnection` with `PgConnectOptions` built from the profile and the provider's secret. On open:

1. if `connection.engine != DbEngine::Postgres` → `CoreError::new(ErrorCode::Unsupported, "M4 supports PostgreSQL only")`;
2. `SET default_transaction_read_only = on` (source only), `statement_timeout`, `lock_timeout`, `idle_in_transaction_session_timeout`, `client_encoding`;
3. read `session_identity()` on the same connection;
4. for the source, assert `SHOW transaction_read_only == on`, `SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user` both false, and no `INSERT/UPDATE/DELETE/TRUNCATE` privilege on the tables supplied by the caller (the caller passes `&[QualifiedName]` to `open`, or a separate `assert_source_role(&[QualifiedName])`).

`assert_binding` compares `identity_key` only.

- [ ] **Step 4: Run to verify pass**

Run the same commands. Expected: PASS, and the real-database tests report the SQLSTATE 25006 read-only failure.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/session.rs crates/dbpinball-core/src/security/verify.rs crates/dbpinball-core/tests/safesync_integration.rs
git commit -m "feat(safesync): bind identity to one owned session per role"
```

**Failure modes:** using `PgPool` (substitution returns); computing identity from the profile instead of the session; treating `default_transaction_read_only` as sufficient without `BEGIN READ ONLY`.
**Acceptance:** a write on the source session fails at the engine; a privileged source role is refused; a reconnect to another cluster fails binding.
**Review checkpoint:** confirm no code path hands the connection back to a pool.
**Depends on:** Task 1.

---

### Task 3 (M4.2): Schema graph discovery

**Prerequisites:** Task 2.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/graph.rs`

**Interfaces:**
- Consumes: `BoundSession` (source), `QualifiedName` from `spec`.
- Produces:
  - `ColumnInfo`, `TableNode`, `ForeignKeyEdge`, `SchemaGraph` exactly as the spec §7.2.
  - `SchemaGraph::discover(session: &mut BoundSession, tables: &[QualifiedName]) -> Result<SchemaGraph, CoreError>`.
  - `SchemaGraph::schema_hash(&self) -> String`.

- [ ] **Step 1: Write the failing real-database tests**

In `safesync_integration.rs`, create a synthetic schema per test with two clusters. Cases:

```rust
#[tokio::test] async fn discovers_a_composite_foreign_key_in_column_order() {}
#[tokio::test] async fn discovers_a_self_reference() {}
#[tokio::test] async fn discovers_a_cycle() {}
#[tokio::test] async fn refuses_a_view() {}
#[tokio::test] async fn refuses_a_foreign_table() {}
#[tokio::test] async fn records_generated_and_identity_columns() {}
#[tokio::test] async fn reports_a_table_without_a_primary_key() {}
#[tokio::test] async fn the_schema_hash_is_stable_across_two_reads() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `DBPINBALL_TEST_PG_SOURCE_URL=... cargo test -p dbpinball-core --features integration --test safesync_integration graph`
Expected: FAIL — function does not exist.

- [ ] **Step 3: Implement discovery**

Run `SET LOCAL search_path = pg_catalog` at the start of the discovery transaction. Query `pg_namespace`, `pg_class`, `pg_attribute`, `pg_constraint`, `pg_index`, `pg_attrdef`, `pg_type` restricted to `current_database()` and the requested schemas. Build `SchemaGraph` from the rows. Refuse `relkind` outside `{r, p}` with `ErrorCode::Unsupported` and a `safesync_code` of `unsupported_relation`. `schema_hash` = SHA-256 over the canonical JSON of the ordered table/column/constraint structures.

- [ ] **Step 4: Run to verify pass**

Run the same command. Expected: PASS, including composite FK order, cycles, generated/identity flags, and view refusal.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/graph.rs crates/dbpinball-core/tests/safesync_integration.rs
git commit -m "feat(safesync): discover the PostgreSQL foreign-key graph from pg_catalog"
```

**Failure modes:** using `information_schema` (loses column order and generated/identity detail); a `search_path` that can be shadowed; including views.
**Acceptance:** every listed catalog case passes against real PostgreSQL; the hash is stable.
**Review checkpoint:** confirm the catalog queries cannot be redirected by the caller's `search_path`.
**Depends on:** Task 2.

---

### Task 4 (M4.3): Selection planner with bounded traversal

**Prerequisites:** Task 3.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/planner.rs`, `crates/dbpinball-core/src/safesync/extract.rs`
- Modify: `crates/dbpinball-core/src/safesync/spec.rs` (add `RowIdentity` resolution helper).

**Interfaces:**
- Consumes: `SchemaGraph`, `SafeSyncSpec`, `Limits`.
- Produces:
  - `SelectionPlan { tables: Vec<PlannedTable>, order: Vec<QualifiedName>, deferred_cycles: Vec<Vec<QualifiedName>>, edges: usize, max_depth_used: u8, complete: bool, failures: Vec<PlanFailure> }`.
  - `PlannedTable { name, row_identity: Vec<String>, direction, estimated_rows: Option<u64>, rls_active }`.
  - `SelectionPlanner::plan(graph, spec) -> Result<SelectionPlan, CoreError>`.
  - `Extractor::extract(session, plan, spec, masker, limits) -> Result<Vec<TableRows>, CoreError>` where `TableRows` is masked rows only.

- [ ] **Step 1: Write the failing tests**

Unit tests (`planner.rs`):

```rust
#[test] fn breadth_first_traversal_stops_at_max_depth() {}
#[test] fn a_cycle_terminates_by_identity() {}
#[test] fn a_table_without_a_row_identity_is_refused() {}
#[test] fn a_cycle_without_deferrable_edges_is_refused() {}
#[test] fn a_cycle_with_all_edges_deferrable_is_ordered() {}
#[test] fn topological_order_places_parents_before_children() {}
```

Real-database tests (`safesync_integration.rs`):

```rust
#[tokio::test] async fn a_nullable_fk_with_a_filtered_parent_still_selects_the_parent() {}
#[tokio::test] async fn exceeding_max_total_rows_marks_the_plan_incomplete() {}
#[tokio::test] async fn a_filter_value_is_bound_not_interpolated() {}
#[tokio::test] async fn an_unknown_filter_column_is_refused() {}
#[tokio::test] async fn a_sql_identifier_cannot_break_out_of_quote_ident() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball-core --lib safesync::planner` and the integration filter.
Expected: FAIL.

- [ ] **Step 3: Implement the planner and extractor**

Validate every identifier against `SchemaGraph` (exists, allowed relkind, filter type comparable) then `quote_ident`. Build `SELECT <columns> FROM <schema>.<table> WHERE <col> = $1` with bound values. Traverse BFS, dedupe by the resolved row identity, count rows and bytes, and stop with `complete: false` plus a `PlanFailure { limit, observed }` when a limit is hit. Extraction returns masked rows only and never writes plaintext anywhere.

- [ ] **Step 4: Run to verify pass**

Run the same commands. Expected: PASS, including the nullable-parent case and the binding test.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/planner.rs crates/dbpinball-core/src/safesync/extract.rs crates/dbpinball-core/tests/safesync_integration.rs
git commit -m "feat(safesync): plan a bounded, parameterized, FK-aware selection"
```

**Failure modes:** string-concatenated identifiers; unbounded traversal; silent truncation instead of `complete: false`.
**Acceptance:** limits produce an incomplete plan and no import; injection probes cannot alter SQL; parents are always included.
**Review checkpoint:** read every SQL construction site and confirm only `quote_ident`-ed identifiers and `$n` placeholders appear.
**Depends on:** Task 3.

---

### Task 5 (M4.4): Deterministic masking engine

**Prerequisites:** Task 4.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/mask.rs`, `crates/dbpinball-core/src/safesync/key.rs`
- Modify: `crates/dbpinball-core/Cargo.toml` (`sha2`, `hmac`, `base32`).

**Interfaces:**
- Consumes: `SchemaGraph`, `TableRows`, `CredentialProvider` (for the key), `Limits`.
- Produces:
  - `MaskStrategy { Keep, Redact, Null, DeterministicPseudonym, DeterministicEmail, DeterministicPhone, DateShift, AmountJitter, Fixed }`.
  - `ColumnClass { PrimaryKey, ForeignKey, Masked, Keep, Unknown }`.
  - `MaskingPolicy { name, version, hash, rules }`, `MaskingPolicy::builtin_default()` (`development-default@1`), `MaskingPolicy::load(path)`.
  - `Masker::new(policy, key) -> Masker`, `Masker::filter_digest(...) -> String`, `Masker::coverage(graph) -> CoverageReport`, `Masker::mask_row(...)`.
  - `MaskKey { id, bytes }` with `MaskKeyProvider::current()` / `rotate()` backed by the keyring, and `MaskKeyProvider::ephemeral(bytes)` under `cfg(any(test, feature = "test-keys"))`.
  - `development-default@1` rules for `public.accounts`: `id → keep` (explicit), `email → deterministic_email`, `full_name → deterministic_pseudonym`; any other table or column → `unmapped_column` refusal.

- [ ] **Step 1: Write the failing tests**

Unit tests:

```rust
#[test] fn the_same_identity_and_key_produce_the_same_output() {}
#[test] fn a_different_key_produces_a_different_output() {}
#[test] fn the_same_value_in_two_columns_produces_different_output() {} // domain separation
#[test] fn the_email_local_part_is_16_base32_chars() {}
#[test] fn the_email_domain_is_example_invalid() {}
#[test] fn the_name_output_is_anon_space_10_base32() {}
#[test] fn the_uuid_id_is_kept_only_by_an_explicit_rule() {}
#[test] fn a_policy_without_the_id_rule_refuses_accounts() {}
#[test] fn a_column_shorter_than_the_output_refuses_without_max_len() {}
#[test] fn a_column_with_max_len_truncates_deterministically() {}
#[test] fn a_unicode_value_is_normalised_before_hashing() {}
#[test] fn a_missing_key_is_refused() {}
#[test] fn an_unknown_policy_version_is_refused() {}
#[test] fn date_shift_stays_within_the_policy_bound() {}
#[test] fn an_unmapped_column_is_refused() {}
#[test] fn a_jsonb_column_without_a_rule_is_refused() {}
#[test] fn a_bytea_column_without_a_rule_is_refused() {}
#[test] fn a_keep_rule_on_a_password_hash_is_rejected_at_load() {}
#[test] fn a_tiny_output_domain_is_rejected() {}
#[test] fn the_audit_record_contains_no_masked_value() {}
#[test] fn the_policy_hash_changes_when_a_rule_changes() {}
```

Real-database tests:

```rust
#[tokio::test] async fn masking_preserves_a_masked_pk_to_fk_join() {}
#[tokio::test] async fn a_unique_key_collision_after_masking_is_refused() {}
#[tokio::test] async fn no_plaintext_email_appears_in_the_run_artifacts() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball-core --lib safesync::mask` and the integration filter.
Expected: FAIL.

- [ ] **Step 3: Implement the masking engine**

`deterministic_pseudonym` = `base32(HMAC-SHA256(key, "dbpinball/safesync/v1" || 0x1F || schema || 0x1F || table || 0x1F || column || 0x1F || identity))`, NFC-normalised input, ASCII output. Email local part = `token[..16]`, domain fixed `example.invalid`. Name output = `"Anon " + token[..10]`. Foreign-key columns inherit the referenced column's strategy so joins survive. Collisions on any unique key are `mask_collision`. The built-in policy is exactly the `public.accounts` table in spec §8.4A; any other table or column refuses. The key is 32 CSPRNG bytes in the keyring; `MaskKeyProvider::ephemeral` exists only under `cfg(any(test, feature = "test-keys"))`. The plan records `key_id` only, and `filter_digest` uses the same key with the `dbpinball/safesync/filter/v1` domain.

- [ ] **Step 4: Run to verify pass**

Run the same commands. Expected: PASS, including the join-preservation and no-plaintext-artifacts tests.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/mask.rs crates/dbpinball-core/src/safesync/key.rs crates/dbpinball-core/Cargo.toml crates/dbpinball-core/tests/safesync_integration.rs
git commit -m "feat(safesync): add a fail-closed deterministic masking engine"
```

**Failure modes:** a default `keep`; the mask key in the plan or a log; non-deterministic output breaking FK joins; deduping instead of refusing a collision.
**Acceptance:** unmapped columns and risky types refuse; the same input maps to the same output; the destination join survives; no value appears in any artifact.
**Review checkpoint:** confirm the key never leaves `key.rs`, and that `CoverageReport` is computed before extraction.
**Depends on:** Task 4.

---

### Task 6 (M4.5): Dry-run plan, validation, and preview

**Prerequisites:** Tasks 1–5.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/plan.rs`, `crates/dbpinball-core/src/safesync/audit.rs`, `crates/dbpinball-core/src/safesync/service.rs`
- Modify: `crates/dbpinball-core/src/safesync/mod.rs`, `crates/dbpinball-core/src/lib.rs`.

**Interfaces:**
- Consumes: everything above; `security::{policy_check, evaluate, OperationContext, DestinationCategory}`.
- Produces:
  - `DryRunPlan` exactly as spec §10.2, including `selection.roots[].filter_digest`, `import.{conflict_policy, mode, sequence_adjustment}`, and `selection.contains_values` (always `false` in M4).
  - `DryRunPlan::fingerprint() -> String`, `DryRunPlan::is_expired(now_ms) -> bool`.
  - `service::{inspect, plan, validate, execute, status, rotate_key}` signatures:
    - `plan(spec, source, destination, credentials, store, now_ms, estimator) -> Result<DryRunPlan, CoreError>`
    - `validate(plan, spec, source, destination, credentials, store, now_ms) -> Result<ValidationReport, CoreError>`
    - `execute(plan, spec, acknowledgement, source, destination, credentials, store, progress, now_ms) -> Result<RunReport, CoreError>`
  - `AuditRecord { run_id, plan_fingerprint, source_key, destination_key, tables: Vec<TableAudit>, policy, key_id, outcome, started_at, finished_at }` with `TableAudit { name, rows, masked_columns }` and **no values**.
  - `IdempotencyOutcome { AlreadyCompleted, DestinationDrifted, DestinationNotEmpty, Proceed }`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn the_fingerprint_is_stable_for_the_same_plan() {}
#[test] fn the_fingerprint_changes_when_the_destination_changes() {}
#[test] fn the_fingerprint_changes_when_the_mask_key_id_changes() {}
#[test] fn the_fingerprint_changes_when_the_conflict_policy_changes() {}
#[test] fn the_fingerprint_changes_when_the_import_mode_changes() {}
#[test] fn the_fingerprint_is_unchanged_by_run_id_and_timestamps() {}
#[test] fn the_plan_json_contains_no_row_values() {}
#[test] fn the_plan_json_contains_no_filter_values() {}
#[test] fn a_spec_that_does_not_match_the_filter_digest_is_refused() {}
#[test] fn an_expired_plan_is_refused() {}
#[test] fn a_mismatched_acknowledgement_is_refused() {}
#[test] fn an_acknowledgement_for_another_destination_is_refused() {}
#[test] fn an_incomplete_plan_cannot_be_executed() {}
#[test] fn metadata_only_plan_reads_no_rows() {}
#[test] fn an_estimate_plan_reads_but_never_returns_values() {}
#[test] fn the_audit_record_has_counts_and_no_values() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball-core --lib safesync::plan safesync::service`.
Expected: FAIL.

- [ ] **Step 3: Implement plan, validate, execute, status**

Canonical JSON = sorted keys, no insignificant whitespace, `run_id`/`created_at`/`expires_at`/`plan_fingerprint` excluded from the fingerprint document. `filter_digest = HMAC-SHA256(mask_key, "dbpinball/safesync/filter/v1" || 0x1F || schema || 0x1F || table || 0x1F || canonical(filter))`. `execute` requires both the plan and the spec; it recomputes `filter_digest` and refuses a mismatch. Execution order: reopen source, `assert_binding(plan.source)`, mask and extract, reopen destination, `assert_binding(plan.destination)`, `policy_check(SafeSyncExecute)` with the session identity, begin the destination transaction, evaluate idempotency from the run table, import, commit, write the run row and the audit record. Acknowledgement is compared to the fingerprint and both identity keys.

- [ ] **Step 4: Run to verify pass**

Run the same command. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/plan.rs crates/dbpinball-core/src/safesync/audit.rs crates/dbpinball-core/src/safesync/service.rs crates/dbpinball-core/src/safesync/mod.rs crates/dbpinball-core/src/lib.rs
git commit -m "feat(safesync): add the fingerprinted dry-run plan and run service"
```

**Failure modes:** a plan that embeds a value; an acknowledgement that is not bound to the destination; executing an incomplete plan; a default plan that reads production rows.
**Acceptance:** fingerprints are stable and sensitive to the destination; an expired or mismatched plan is refused; the metadata-only plan issues no row query.
**Review checkpoint:** diff the plan serializer and confirm no `TableRows` field can reach it.
**Depends on:** Tasks 1–5.

---

### Task 7 (M4.6): Transactional destination import

**Prerequisites:** Task 6.

**Files:**
- Create: `crates/dbpinball-core/src/safesync/import.rs`
- Modify: `crates/dbpinball-core/src/safesync/service.rs`.

**Interfaces:**
- Consumes: masked `TableRows`, `SelectionPlan`, `SchemaGraph`, `BoundSession` (destination).
- Produces:
  - `ConflictPolicy { Refuse, Replace }`.
  - `DestinationImporter { policy: ConflictPolicy }`.
  - `DestinationImporter::run(session, plan, rows, graph, limits) -> Result<ImportReport, CoreError>`.
  - `ImportReport { tables: Vec<TableImport>, committed: bool, already_completed: bool }`.

- [ ] **Step 1: Write the failing real-database tests**

```rust
#[tokio::test] async fn imports_parents_before_children() {}
#[tokio::test] async fn a_cyclic_deferrable_schema_imports() {}
#[tokio::test] async fn a_non_deferrable_cycle_is_refused_before_any_write() {}
#[tokio::test] async fn a_non_empty_destination_is_refused_by_default() {}
#[tokio::test] async fn replace_deletes_only_the_selected_tables() {}
#[tokio::test] async fn running_the_same_plan_twice_is_a_noop() {}
#[tokio::test] async fn a_completed_run_with_changed_destination_rows_is_destination_drifted() {}
#[tokio::test] async fn an_interrupted_import_leaves_the_destination_unchanged() {}
#[tokio::test] async fn schema_drift_is_refused() {}
#[tokio::test] async fn a_generated_column_is_not_inserted() {}
#[tokio::test] async fn the_default_import_mode_touches_no_sequence() {}
#[tokio::test] async fn destination_contents_are_actually_masked() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `DBPINBALL_TEST_PG_SOURCE_URL=... DBPINBALL_TEST_PG_DEST_URL=... cargo test -p dbpinball-core --features integration --test safesync_integration import`
Expected: FAIL.

- [ ] **Step 3: Implement the importer**

Open one destination transaction. Create `dbpinball_safe_sync_runs` if absent (inside the transaction; a DDL in a transaction is fine in PostgreSQL). Check the run table for the fingerprint: if `completed` and the destination still matches the recorded state, return `already_completed` without writing; if `completed` but the selected tables drifted, refuse `destination_drifted`. Check schema drift against the plan's `schema_hash`. Check the selected tables are empty unless `Replace`. Insert in plan order; `SET CONSTRAINTS ALL DEFERRED` only when the plan says every cycle edge is deferrable. Omit generated columns. **Slice 1 and the default import mode perform no sequence adjustment** (`sequence_adjustment: false`); an identity column is out of scope for Slice 1, and a later milestone must run any `setval` as a separate idempotent step after commit, never claiming it was transactional. Write the run row, then commit. Never use `session_replication_role` or `ALTER TABLE ... DISABLE TRIGGER`.

- [ ] **Step 4: Run to verify pass**

Run the same command. Expected: PASS, including destination-content assertions and the interruption rollback.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-core/src/safesync/import.rs crates/dbpinball-core/src/safesync/service.rs crates/dbpinball-core/tests/safesync_integration.rs
git commit -m "feat(safesync): import inside one destination transaction"
```

**Failure modes:** `ON CONFLICT DO NOTHING` treated as idempotency; `session_replication_role`; dropping constraints; importing into a non-empty destination by default.
**Acceptance:** order is correct; cycles are refused or deferred correctly; a second identical run is a no-op; an interruption rolls back; destination rows are masked.
**Review checkpoint:** confirm there is exactly one `BEGIN`/`COMMIT` pair and no `ALTER TABLE ... DISABLE TRIGGER`.
**Depends on:** Task 6.

---

### Task 8 (M4.7): CLI commands and contract tests

**Prerequisites:** Task 7.

**Files:**
- Create: `crates/dbpinball-cli/src/commands/safe_sync.rs`, `crates/dbpinball-cli/tests/safesync_cli_integration.rs`
- Modify: `crates/dbpinball-cli/src/cli.rs`, `crates/dbpinball-cli/src/commands/mod.rs`, `crates/dbpinball-cli/src/main.rs`.

**Interfaces:**
- Consumes: `dbpinball_core::safesync::service::{...}`.
- Produces: the six `safe-sync` subcommands, arguments and exit codes from spec §13.

- [ ] **Step 1: Write the failing CLI tests**

Unit (`cli.rs`):

```rust
#[test] fn parses_safe_sync_plan_with_spec() {}
#[test] fn parses_safe_sync_execute_with_ack_and_spec() {}
#[test] fn execute_without_spec_is_rejected() {}
#[test] fn rejects_a_password_option() {}
#[test] fn every_subcommand_has_a_help_line() {}
```

Integration (`safesync_cli_integration.rs`, real databases):

```rust
#[test] fn plan_emits_one_json_envelope() {}
#[test] fn plan_without_a_tty_and_without_json_is_refused() {}
#[test] fn execute_without_ack_exits_2() {}
#[test] fn execute_without_spec_exits_2() {}
#[test] fn a_spec_that_does_not_match_the_plan_exits_2() {}
#[test] fn an_unenrolled_destination_exits_4() {}
#[test] fn an_incomplete_plan_exits_5_with_limit_exceeded() {}
#[test] fn an_expired_plan_exits_8() {}
#[test] fn a_completed_repeat_exits_0_with_already_completed() {}
#[test] fn stderr_progress_is_ndjson_and_stdout_has_one_object() {}
#[test] fn no_secret_appears_in_any_output() {}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball-cli --lib cli` and the integration filter.
Expected: FAIL — subcommand missing.

- [ ] **Step 3: Implement the CLI**

Add `Command::SafeSync { command: SafeSyncCommand }` with `Inspect`, `Plan`, `Validate`, `Execute`, `Status`, `KeyRotate`. Each arm calls the core service and reproduces exactly one envelope via `print_success`/`print_error`. `validate` and `execute` take `--plan` and `--spec` (the plan holds no filter values). Add `--ack`, `--replace`, `--strict-idempotency`, `--yes`, `--estimate`, `--out`. No synchronization logic in the CLI.

- [ ] **Step 4: Run to verify pass**

Run the same commands. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/dbpinball-cli/src crates/dbpinball-cli/tests/safesync_cli_integration.rs
git commit -m "feat(cli): add safe-sync plan, validate, execute and status"
```

**Failure modes:** two JSON objects on stdout; a new exit code; a prompt when there is no TTY; a secret in an error.
**Acceptance:** every documented exit code is exercised; stdout is exactly one envelope; no prompt without a TTY.
**Review checkpoint:** confirm the CLI has no SQL and no policy logic.
**Depends on:** Task 7.

---

### Task 9 (M4.8): Desktop adapter

**Prerequisites:** Task 8, plus an approved CLI contract.

**Files:**
- Create: `src-tauri/src/safesync.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/src/models.rs`

**Interfaces:**
- Consumes: the same core services.
- Produces: `safesync_inspect`, `safesync_plan`, `safesync_validate`, `safesync_execute`, `safesync_status`, `safesync_key_rotate`, each `Result<T, String>` and each a thin pass-through.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn every_safesync_command_is_registered() {}
#[test] fn the_adapter_adds_no_policy_decision() {} // source scan: no `evaluate(` in safesync.rs
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p dbpinball --lib safesync`
Expected: FAIL.

- [ ] **Step 3: Implement the adapter**

Each command deserializes arguments, calls the core service, maps `CoreError` through the existing `safe_error`, and returns. Register all six in `generate_handler!`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p dbpinball --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src
git commit -m "feat(desktop): add thin Safe Sync Tauri adapters"
```

**Failure modes:** a UI-side decision; a second credential read; a command that bypasses the core.
**Acceptance:** commands are registered and are pass-throughs; no `evaluate` call in the adapter.
**Review checkpoint:** confirm the adapter cannot construct an `OperationContext`.
**Depends on:** Task 8.

---

### Task 10 (M4.9): End-to-end PostgreSQL integration, CI, and documentation

**Prerequisites:** Tasks 1–9.

**Files:**
- Create: `crates/dbpinball-core/tests/safesync_e2e.rs`
- Modify: `.github/workflows/ci.yml`, `website/security/safe-sync-design.md`, `website/roadmap.md`, `website/.vitepress/config.mts`, `website/index.md`

**Interfaces:**
- Consumes: the CLI binary and both databases.
- Produces: the acceptance evidence and the planned-only documentation.

- [ ] **Step 1: Write the failing end-to-end tests**

Two disposable clusters, synthetic data only:

```rust
#[tokio::test] async fn a_bug_report_subset_round_trips_masked() {}
#[tokio::test] async fn production_is_never_a_write_destination() {}
#[tokio::test] async fn an_unknown_destination_is_refused() {}
#[tokio::test] async fn a_reversed_source_and_destination_is_refused() {}
#[tokio::test] async fn a_source_identity_change_between_plan_and_execute_is_refused() {}
#[tokio::test] async fn a_destination_identity_change_between_plan_and_execute_is_refused() {}
#[tokio::test] async fn an_interrupted_run_leaves_no_rows() {}
#[tokio::test] async fn a_second_identical_run_creates_no_duplicates() {}
#[tokio::test] async fn no_sensitive_value_appears_in_any_output() {}
```

Each asserts **actual destination table contents**, not just exit codes.

- [ ] **Step 2: Run to verify failure**

Run: `DBPINBALL_TEST_PG_SOURCE_URL=... DBPINBALL_TEST_PG_DEST_URL=... cargo test -p dbpinball-core --features integration --test safesync_e2e`
Expected: FAIL until the assertions and fixtures are complete.

- [ ] **Step 3: Wire CI**

Add a `safesync-integration` job with two PostgreSQL service containers, exporting `DBPINBALL_TEST_PG_SOURCE_URL` and `DBPINBALL_TEST_PG_DEST_URL`. Use `tags: [bw-tools]`. Do not mark it `allow_failure`. Assert the suite reports a non-zero test count (a skip is not acceptance).

- [ ] **Step 4: Write the planned-only documentation**

Create `website/security/safe-sync-design.md` describing M4 as **planned**, with the trust boundaries, the source protection rules, the masking guarantees and the non-goals. Add it to the Security sidebar. Update `website/roadmap.md` to mark M3 merged and M4 "design approved, not implemented". Do not mark Safe Sync available anywhere.

- [ ] **Step 5: Run documentation checks**

Run: `cd website && npm run build && node scripts/check-links.mjs && node scripts/check-contrast.mjs && node scripts/check-layout.mjs`
Expected: build succeeds; links, contrast and layout pass.

- [ ] **Step 6: Run the full verification**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && node scripts/check-boundaries.cjs`
Expected: all pass; the integration suites report a non-zero count.

- [ ] **Step 7: Commit**

```bash
git add crates/dbpinball-core/tests/safesync_e2e.rs .github/workflows/ci.yml website
git commit -m "test(safesync): prove the end-to-end masked subset and document M4 as planned"
```

**Failure modes:** an end-to-end test that only checks the exit code; a CI job that skips silently; documentation that marks M4 available.
**Acceptance:** destination contents are asserted; the CI job runs the suite; the docs say planned.
**Review checkpoint:** confirm the acceptance suite would fail if masking were removed.
**Depends on:** Tasks 1–9.

---

## First Vertical Slice (Slice 1)

Slice 1 is specified in `docs/superpowers/plans/2026-11-14-dbpinball-m4-slice-1-brief.md`
and in spec §16, which carries the 20-row acceptance matrix and 12 negative tests.
In short: one table with no foreign keys, `public.accounts(id uuid primary key,
email text, full_name text)`, two isolated disposable clusters, a restricted source
role, `id → keep` by explicit rule, `email → deterministic_email`,
`full_name → deterministic_pseudonym`, one destination transaction, and a repeat run
that is a no-op.

Slice 1 exercises Task 1 (policy), Task 2 (session binding), Task 4 (single-table
extraction), Task 5 (masking), Task 6 (plan/acknowledgement/idempotency), Task 7
(transactional import) and Task 8 (CLI), in that order. It does **not** require the
graph work (Task 3) or the cycle work, which follow on the same spine.

## Work-package dependency graph

```
M4.0 (Task 1) ──► M4.1 (Task 2) ──► M4.2 (Task 3) ──► M4.3 (Task 4) ──► M4.4 (Task 5)
                                                              │
                                                              ▼
                                                     M4.5 (Task 6) ──► M4.6 (Task 7) ──► M4.7 (Task 8) ──► M4.8 (Task 9) ──► M4.9 (Task 10)
```

Tasks 3 and 5 can run in parallel once Task 2 is merged. Task 9 must not start before Task 8's contract tests are green.

## Self-review notes

- Every spec section maps to a task: §4→Task 2, §5→Tasks 2/6, §6→Tasks 1/4, §7→Task 3, §8→Task 5, §9→Tasks 6/7, §10→Task 6, §11/§12→Tasks 6/7, §13→Task 8, §14→Task 9, §2→Tasks 2/4/5/7, acceptance→Task 10.
- The twelve non-negotiable acceptance criteria are each covered: production never a write destination (Tasks 1, 10), read-only source (Task 2), destination identity on the execution session (Tasks 2, 6), mandatory masking (Task 5), unknown fields fail closed (Tasks 1, 5), referential integrity or refusal (Tasks 3, 4, 7), dry-run distinct from execution (Task 6), retries cannot duplicate (Task 7), no sensitive values in artifacts (Tasks 5, 6, 8, 10), all writes need a verified destination (Tasks 1, 6, 7), CLI-first testable (Task 8), PostgreSQL-only (Tasks 1, 2).
