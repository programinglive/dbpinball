# DBPinball M4 Slice 1 — Implementation Brief

Status: **implementation brief only.** No runtime code exists yet.

Parent documents:

- Specification: `docs/superpowers/specs/2026-11-14-dbpinball-m4-safe-sync-architecture.md` (§16 is the acceptance contract)
- Plan: `docs/superpowers/plans/2026-11-14-dbpinball-m4-safe-sync-plan.md`

Approved design baseline: M4 planning PR (merge SHA recorded in the planning PR).

---

## 1. Scope

Build the security and correctness spine of Safe Sync, end to end, on one table
with no foreign keys. Do not build relationship traversal, cycles, deferred
constraints, masking strategies beyond the three named here, desktop UI, or
`--batch`.

### In scope

| Item | Included |
| --- | --- |
| Source authorization (`SafeSyncPrepare`) | yes |
| Destination authorization (`SafeSyncExecute`) | yes |
| `BoundSession` for source and destination | yes |
| Spec parsing and validation | yes |
| Catalog discovery limited to one table | yes |
| Single-table bounded extraction | yes |
| `development-default@1` with three column rules | yes |
| Mask key provider (keyring + test ephemeral) | yes |
| Plan, fingerprint, acknowledgement, expiry | yes |
| One destination transaction | yes |
| Run table and idempotency | yes |
| CLI `inspect`, `plan`, `validate`, `execute`, `status` | yes |
| `key rotate` | yes |
| Real PostgreSQL acceptance tests | yes |

### Out of scope (do not build in Slice 1)

- foreign-key traversal, composite keys, cycles, deferrable constraints;
- partitioned tables, generated columns, identity columns, sequences;
- `redact`/`null`/`date_shift`/`amount_jitter`/`fixed` strategies;
- `--batch`, `--strict-idempotency` beyond the recorded flag;
- desktop commands and UI;
- MySQL, MariaDB;
- background or scheduled sync.

---

## 2. Exact Core modules

Create:

```text
crates/dbpinball-core/src/safesync/mod.rs
crates/dbpinball-core/src/safesync/spec.rs
crates/dbpinball-core/src/safesync/session.rs
crates/dbpinball-core/src/safesync/graph.rs        // single-table discovery subset
crates/dbpinball-core/src/safesync/planner.rs      // no traversal; depth must be 0
crates/dbpinball-core/src/safesync/mask.rs
crates/dbpinball-core/src/safesync/key.rs
crates/dbpinball-core/src/safesync/extract.rs
crates/dbpinball-core/src/safesync/plan.rs
crates/dbpinball-core/src/safesync/import.rs
crates/dbpinball-core/src/safesync/audit.rs
crates/dbpinball-core/src/safesync/service.rs
```

Modify:

```text
crates/dbpinball-core/src/lib.rs                  pub mod safesync;
crates/dbpinball-core/src/security/policy.rs      SafeSyncPrepare / SafeSyncExecute arms
crates/dbpinball-core/src/security/mod.rs         re-export new PolicyCode variants
crates/dbpinball-core/Cargo.toml                  serde_yaml, sha2, hmac, base32, getrandom
```

Boundary: `dbpinball-core` must not depend on `tauri` or `clap`.

---

## 3. CLI contracts

```bash
dbpinball safe-sync inspect --source <id> --json
dbpinball safe-sync plan    --spec <file> [--estimate] [--out <file>] --json
dbpinball safe-sync validate --plan <file> --spec <file> --json
dbpinball safe-sync execute --plan <file> --spec <file> --ack <fingerprint> [--replace] [--yes] --json
dbpinball safe-sync status  --run <id> --json
dbpinball safe-sync key rotate --json
```

- One JSON envelope on stdout with `--json`; progress is NDJSON on stderr.
- Exit codes reuse `crates/dbpinball-cli/src/exit.rs`: 0, 2, 3, 4, 5, 7, 8, 9.
- `validate` and `execute` take both `--plan` and `--spec`; the plan holds no filter
  values.
- No `--password`; credentials come from `CredentialProvider`.
- No prompt without a TTY; `--ack` and `--yes` are required there.

---

## 4. Dependency order

```text
1. Policy arms + spec types            (no DB)
2. BoundSession + source role checks   (needs 1)
3. Masking engine + key provider       (needs 1)
4. Single-table graph + extraction     (needs 2)
5. Plan + fingerprint + acknowledgement(needs 1,4)
6. Transactional import + idempotency  (needs 5)
7. CLI commands                        (needs 6)
8. End-to-end acceptance + CI + docs   (needs 7)
```

Steps 2 and 3 may run in parallel. Step 7 must not start until step 6's tests are
green.

---

## 5. Test-first implementation sequence

Each step is a red → green → commit cycle. The tests are the ones named in the plan;
the acceptance matrix is spec §16.2.

| Step | First failing test group | Passes when |
| --- | --- | --- |
| 1 | `security::policy` Safe Sync arms; `safesync::spec` validation | policy returns `SourceAuthorized`/`MissingSource`/`NotEnrolled` correctly; unknown spec fields rejected |
| 2 | `safesync::session`; real DB source read-only and privilege checks | `INSERT` on source fails 25006; superuser/`UPDATE` role refused; identity bound |
| 3 | `safesync::mask` unit tests | determinism, domain separation, `example.invalid`, `Anon <10>`, unmapped refusal, missing key refusal |
| 4 | `safesync::planner` single-table + extraction binding | one root, depth 0, filter bound as `$1`, limits enforced |
| 5 | `safesync::plan` fingerprint + expiry + ack | stable fingerprint, no values in JSON, spec/digest mismatch refused |
| 6 | `safesync::import` transaction + idempotency | one transaction, repeat no-op, drift refused, rollback on failure |
| 7 | CLI unit + integration | one envelope, documented exit codes, no secrets |
| 8 | spec §16.2 and §16.3 | all 20 acceptance rows and 12 negative tests pass on two clusters |

Verification for every step:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node scripts/check-boundaries.cjs
```

Real-database step:

```bash
DBPINBALL_TEST_PG_SOURCE_URL=... DBPINBALL_TEST_PG_DEST_URL=... \
  cargo test -p dbpinball-core --features integration --test safesync_integration
```

A skipped run is not a pass; the CI job must report a non-zero test count.

---

## 6. Security gates (all must hold before Slice 1 is accepted)

1. **Source is read-only at the engine.** `default_transaction_read_only=on`,
   `BEGIN READ ONLY`, and a probe that an `INSERT` fails with SQLSTATE 25006.
2. **Source role is restricted.** Not superuser, not `rolbypassrls`, no
   `INSERT/UPDATE/DELETE/TRUNCATE` on `public.accounts`, not the owner.
3. **Identity is bound to the session that executes.** `assert_binding` compares the
   identity read on the same connection; no pool.
4. **Destination is enrolled, verified and bound.** `policy_check(SafeSyncExecute)`
   runs with the execution session's identity inside the import transaction.
5. **Masking is mandatory.** Every selected column has an explicit rule; an unmapped
   column, missing key, invalid policy or unsupported type refuses.
6. **No plaintext leaves the extractor.** No file, log, plan, audit record or CLI
   argument contains a source value.
7. **The plan has no values.** `contains_values=false`, `filter_digest` only.
8. **One destination transaction.** No constraint disabling, no `setval`, no
   batch mode.
9. **Acknowledgement is not authorization.** The policy still decides.
10. **PostgreSQL only.** A MySQL source or destination is `unsupported_engine`.

If any gate cannot be demonstrated by a test, Slice 1 is not done.

---

## 7. Real PostgreSQL fixtures

Two isolated disposable clusters, never production:

| Fixture | Contents |
| --- | --- |
| Source cluster A | `public.accounts(id uuid pk, email text not null, full_name text not null)`, 3 synthetic rows, emails at `example.com` only |
| Destination cluster B | identical `public.accounts`, empty |
| Source role | `LOGIN`, `CONNECT`, `USAGE` on `public`, `SELECT` on `public.accounts`; **no** write privilege; not superuser; not `rolbypassrls` |
| Negative roles | `accounts_writer` with `UPDATE`; `accounts_owner` as owner; `superuser` — each used only by a refusal test |
| Destination enrollment | `verified_development` via the existing M3 `security enroll` |
| Mask key | `MaskKeyProvider::ephemeral` with a fixed test key, compiled under `cfg(any(test, feature = "test-keys"))` |
| CI | two PostgreSQL service containers, `tags: [bw-tools]`, no `allow_failure` |

All data is synthetic. No production endpoint, credential or dump is used.

---

## 8. Expected PR boundaries

Slice 1 should land as **three focused PRs**, not one:

| PR | Contents | Must be green |
| --- | --- | --- |
| **S1-a** | Policy arms + spec types + masking engine + key provider (steps 1 and 3) | unit tests, fmt, clippy, boundary |
| **S1-b** | `BoundSession` + single-table extraction + plan/fingerprint (steps 2, 4, 5) | unit + real-DB source tests |
| **S1-c** | Transactional import + idempotency + CLI + end-to-end acceptance + CI (steps 6, 7, 8) | all real-DB acceptance and negative tests |

Each PR is independently reviewable and independently green. No PR may merge with a
skipped integration suite.

---

## 9. Stop conditions

Stop and escalate instead of working around any of these:

1. The source cannot be made database-enforced read-only in the test cluster.
2. `pg_control_system()` or the identity fields are unavailable to the restricted
   role, so the source cannot be identified. Do **not** weaken identity.
3. The mask key cannot be stored outside the plan/spec/logs on the target platform.
4. The destination cannot be enrolled as `verified_development` without touching a
   non-disposable database.
5. A required acceptance test can only pass by disabling a constraint, truncating,
   or writing to the source.
6. A behaviour would require MySQL support to satisfy the slice.
7. CI cannot run two clusters on `bw-tools` without a new runner.

Each stop condition is a design or environment blocker to report, not a reason to
relax a security gate.

---

## 10. Definition of done

- spec §16.2 (20 rows) and §16.3 (12 negative tests) pass against two real
  disposable PostgreSQL clusters in CI;
- `cargo fmt`, `clippy -D warnings`, the workspace tests and the boundary check
  pass;
- the destination contains masked rows only, and the source is byte-identical before
  and after;
- the plan contains no values;
- a repeat run is a no-op;
- the documentation still describes Safe Sync as planned;
- no tag, release, deploy or production access.
