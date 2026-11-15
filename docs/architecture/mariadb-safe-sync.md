# MariaDB Safe Sync — Architecture & Threat Model (MDB-0)

**Status:** Approved Design Baseline (Implementation begins with MDB-1)  
**Scope:** Extending DBPinball M4 Safe Sync from PostgreSQL-only to MariaDB 10.6+ / 10.11+ InnoDB workloads while preserving all PostgreSQL Safe Sync guarantees.

---

## 1. Engine Differences & Threat Model

| Security / Correctness Dimension | PostgreSQL M4 Baseline | MariaDB 10.11+ Reality | Required MariaDB Contract |
|---|---|---|---|
| **Server Identity** | `pg_control_system().system_identifier` + `pg_database.oid` | No `@@server_uuid` (`1193` error); `@@server_id` defaults to `1` | Detect `MariaDB` via `VERSION()`; compute `mariadb-fp:<sha256>` from `@@hostname`, `@@port`, `@@datadir`, `@@server_id`; fail closed if any component is empty. |
| **Read-Only Source** | `SET default_transaction_read_only = on`, `BEGIN READ ONLY`, `statement_timeout` (ms) | `SET SESSION TRANSACTION READ ONLY`, `START TRANSACTION READ ONLY`, `max_statement_time` (s), `lock_wait_timeout` (s), `SET NAMES utf8mb4` | Enforce session read-only mode and inspect effective grants (`information_schema.*_PRIVILEGES` / `SHOW GRANTS`) when source policy requires read-only roles. |
| **Storage Engine** | Heap / partitioned tables (`relkind IN ('r','p')`) | Mixed engines (InnoDB, MyISAM, Aria, MEMORY) | Require `ENGINE = 'InnoDB'` and `TABLE_TYPE = 'BASE TABLE'`; refuse non-transactional tables fail-closed. |
| **Foreign-Key Cycles** | `DEFERRABLE` FKs supported for cycles | **No `DEFERRABLE` FKs** in InnoDB | Refuse any FK cycle (including self-references) unless an explicit non-cyclic root plan exists; never run `SET FOREIGN_KEY_CHECKS=0`. |
| **Catalog & Quoting** | `public` default schema, `"ident"`, `$n::type` | Schema = database name, `` `ident` ``, `?` placeholders | Abstract identifier quoting and `QualifiedName` default-schema resolution per engine. |
| **DDL & Locking** | Transactional DDL, `pg_advisory_xact_lock`, `LOCK TABLE` | DDL (`CREATE/ALTER`, `ALTER ... AUTO_INCREMENT`, `LOCK TABLES`) **implicitly commits** | Isolate `dbpinball_safe_sync_runs` DDL before data tx; use session-scoped `GET_LOCK`/`RELEASE_LOCK` for cross-process concurrency. |
| **Triggers & RLS** | `pg_class.relrowsecurity` checked | No PG-style RLS; triggers in `information_schema.TRIGGERS` | Inspect `information_schema.TRIGGERS` on destination tables; refuse or require explicit policy when triggers exist. |
| **Plan Envelope** | `PlanIdentity.engine = "postgres"` | Multi-engine plans | Bind actual engine (`"postgres"` / `"mariadb"`) into `PlanIdentity`, `spec_hash`, `plan_fingerprint`, and v3 HMAC. |

---

## 2. Milestone Breakdown (`MDB-0` – `MDB-9`)

- **MDB-0 (This Document):** Architecture, engine abstraction boundaries, and MariaDB threat model.
- **MDB-1:** MariaDB identity verification (`security/verify.rs` `@@server_uuid` fix), read-only session enforcement, and privilege checks.
- **MDB-2:** `information_schema` schema/FK/trigger/InnoDB graph inspection and non-deferrable cycle refusal.
- **MDB-3:** Bounded extraction, backtick quoting, `?` parameter binding, and MariaDB `cast_for` deterministic masking.
- **MDB-4:** Engine-aware `PlanIdentity` / `DryRunPlan` / signed selection validation (`spec_hash` + v3 MAC).
- **MDB-5:** Transactional InnoDB destination import without `"public"` schema assumptions.
- **MDB-6:** `AUTO_INCREMENT` reconciliation (handling implicit DDL commits) and `GET_LOCK` crash/concurrency recovery.
- **MDB-7:** CLI integration (`safe-sync` MariaDB enablement + `security inspect` MariaDB support).
- **MDB-8:** Two-instance MariaDB 10.11 integration test suite (mirroring PG S1/S2-a/S2-b/S2-c coverage).
- **MDB-9:** Documentation updates and Desktop (`src-tauri`) integration readiness.

## Implemented Architecture (MDB-0 through MDB-9)

1. **Identity & Enrollment (`security` + `safesync::session`)**:
   - `mysql` profiles are inspected via `SELECT VERSION(), DATABASE(), CURRENT_USER(), @@hostname, @@port, @@read_only, @@super_read_only`.
   - MariaDB servers (`is_mariadb_version`) bind `PlanIdentity` with `engine: "mariadb"`.
   - Source connections enforce `SET NAMES utf8mb4`, `SET SESSION time_zone = '+00:00'`, `SET SESSION TRANSACTION READ ONLY`, `max_statement_time`, `lock_wait_timeout`, `@@SESSION.tx_read_only == 1`, and `SHOW GRANTS FOR CURRENT_USER()` write-privilege refusal.
   - Destination connections must be enrolled as `local` (production/staging destinations are refused).
2. **Graph & Triggers (`safesync::mariadb_graph`)**:
   - Discovers `BASE TABLE` metadata, primary keys, columns, and composite foreign keys from `information_schema`.
   - Refuses non-InnoDB engines and tables with `BEFORE`/`AFTER` triggers unless explicitly allowed by policy.
3. **Bounded Extraction & Deterministic Masking (`safesync::planner`, `safesync::mask`, `safesync::typed`)**:
   - Uses backtick-quoted identifiers, `?` positional placeholders, and UTC casts (`CAST(? AS ...)`).
   - Supports `eq`, `in` (up to 100 values), `gte`, and `lte` (up to 8 predicates) on MariaDB integer (`tinyint`, `smallint`, `mediumint`, `int`, `bigint`), `decimal`/`numeric`, `date`, `datetime`, and `timestamp` types.
   - Refuses risky unmasked/binary/spatial types (`blob`, `tinyblob`, `mediumblob`, `longblob`, `binary`, `varbinary`, `geometry`, `point`, `linestring`, `polygon`) unless configured as `omit` or `null`.
4. **Transactional Import, Locking & Recovery (`safesync::import_mariadb`)**:
   - Acquires a session-scoped `GET_LOCK` per destination database/schema to prevent concurrent imports.
   - Executes `DELETE FROM` in reverse topological order (when `replace: true`) and parameterized `INSERT` inside a single InnoDB transaction.
   - Records run state and digests in `dbpinball_safe_sync_runs` for crash recovery and idempotent retry.
   - Reconciles `AUTO_INCREMENT` sequences **post-commit** via `ALTER TABLE ... AUTO_INCREMENT = <next>` so DDL never triggers an implicit commit mid-transaction.
5. **CLI & Desktop (`dbpinball-cli`, `src-tauri`, `src/pages/safesync.tsx`)**:
   - `dbpinball safe-sync` and the Desktop Safe Sync view (`safe_sync_inspect`, `safe_sync_plan`, `safe_sync_validate`, `safe_sync_execute`, `safe_sync_status`) share the exact same `dbpinball_core::safesync::service` pipeline and require explicit destination acknowledgement before mutation.

## Known Scope & Limitations

- MariaDB 10.11+ InnoDB `BASE TABLE` targets are supported; MyISAM/MEMORY/ARCHIVE tables and partitioned or cross-schema views are refused.
- MySQL (non-MariaDB) servers are not enabled for Safe Sync in this phase.
