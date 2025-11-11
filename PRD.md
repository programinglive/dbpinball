# dbpinball Product Requirements Document (PRD)

## 1. Overview

**Product Name:** dbpinball  
**Goal:** Deliver a one-click backup, restore, and copy solution for MySQL and PostgreSQL databases (local or remote) with live progress, logging, and resumable jobs.  
**Platforms:** Windows, macOS, Linux (Tauri desktop app).  
**Primary Users:** Developers and operators who need reliable database dumps without launching heavy GUI tools.  
**Release Stage:** Initial desktop release (v0.1.0).

## 2. Core Use Cases

### 2.1 Quick Backup
- Select a saved connection or enter credentials.
- Choose backup type: **Structure + Data**, **Structure only**, or **Data only**.
- Optional toggles:
  - Include views, routines, triggers.
  - Bulk inserts (MySQL).
  - Ignore DEFINER (MySQL).
  - Compression: None or ZIP.
- Generate `.sql` output (optionally zipped) with live progress.

### 2.2 Restore Existing Dump
- Select target connection.
- Provide `.sql` or `.dump` file path.
- Options:
  - Drop schema/database first.
  - Stop on first error or continue.
- Stream results with live progress and error reporting.

### 2.3 Copy Database
- Pick source and destination connections.
- Options:
  - Copy structure only or structure + data.
  - Drop objects if they exist in target.
- Same-engine copy supported in MVP (MySQL→MySQL, PostgreSQL→PostgreSQL).
- Cross-engine copy (best effort type mapping) earmarked for Phase 2.

### 2.4 Job History
- View list of executed jobs (backup, restore, copy) with:
  - Status (running, succeeded, failed).
  - Start/end time & duration.
  - Log entries and messages.
  - Downloadable artifact path for backups.
  - Re-run option for repeatable jobs (Phase 2).

### 2.5 Connection Vault
- Store named connections with metadata:
  - Engine, host, port, database, username, SSL toggle.
- Passwords stored securely via OS keyring (Keychain / Credential Manager / libsecret).
- Quick actions: test connection, edit, delete.

## 3. Functional Requirements

### 3.1 Database Compatibility
- **MySQL:** 5.7+, 8.0+.
- **PostgreSQL:** 11+.
- TLS support for secure connections.
- SSH tunneling support deferred to Phase 2.

### 3.2 Backup Features
- Structure + data, structure only, data only.
- Include views, routines, triggers.
- Bulk INSERT batching for MySQL.
- Optional "Ignore DEFINER" for MySQL.
- Compression: none or ZIP.
- Native tool usage when available:
  - MySQL: `mysqldump`.
  - PostgreSQL: `pg_dump`.
- Fallback to internal dumper (sqlx streaming) when native tools missing.

### 3.3 Restore Features
- Support `.sql` and `.dump` files.
- Options:
  - Drop schema first.
  - Stop on first error or continue.
- Use vendor tools when available (`mysql`, `psql`).
- Fallback to internal streaming execution.

### 3.4 Copy Features
- Source → Target same engine copy using dump + restore pipeline.
- Options for structure/data selection and target drop.
- Phase 2: Cross-engine copy with type mapping (e.g., MySQL↔PostgreSQL conversions).

### 3.5 Progress & Logging
- Stream messages via `tauri::Window::emit("job-progress", payload)`.
- UI displays real-time log feed with timestamps.
- Record job history to local storage (Phase 1 minimal viability using localStorage; future enhancements can migrate to SQLite/log store).

### 3.6 Artifacts
- Default output directory: `~/dbpinball/backups/<job-id>/`.
- Files named using pattern `{engine}_{database}_{timestamp}.sql[.zip]`.

## 4. Non-Functional Requirements

| Category      | Requirements |
|---------------|--------------|
| **Reliability** | Prefer vendor CLI tools for proven behavior; fallback internal dumper must handle large datasets calmly. |
| **Security**   | Passwords stored only in OS keyring, never plain text; no logging of full connection strings; TLS support. |
| **Performance**| Parallel table dumps (configurable) planned for Phase 2; chunked fetch for large tables; streaming to avoid memory spikes. |
| **DX**         | All backend operations exposed via Tauri commands with type-safe TypeScript bindings; integration tests using dockerized DBs (Phase 2). |

## 5. Architecture Overview

```
Tauri (Rust) Core
 ├─ dbpinball-core/
 │   ├─ drivers/
 │   │   ├─ mysql.rs (sqlx + mysqldump fallback)
 │   │   └─ postgres.rs (sqlx + pg_dump fallback)
 │   ├─ backup.rs / restore.rs / copy.rs
 │   ├─ jobs.rs (progress events, job registry)
 │   ├─ secrets.rs (OS keyring)
 │   └─ config.rs (saved connections / prefs)
 ├─ src-tauri/
 │   ├─ main.rs (Tauri bootstrap & commands)
 │   └─ schema/ (optional TS bindings, Phase 2)
 └─ ui/ React + shadcn/ui interface
     ├─ app/connections, backup, restore, copy, jobs
     └─ components/ (forms, pickers, log view)
```

### 5.1 Backend
- `models.rs`: shared structs (`DbConnection`, `DbEngine`, options types).
- `runners/mysql.rs` and `runners/postgres.rs`: vendor tool invocation, fallbacks, logging.
- `secrets.rs`: keyring integration.
- Job orchestration functions in `runners/mod.rs`.

### 5.2 Frontend
- React 18 + TypeScript + Vite.
- shadcn/ui for consistent components.
- Pages: Connections, Backup, Restore, Copy, Jobs.
- Tauri event listeners for progress logs.

## 6. UI & UX Requirements

### 6.1 Navigation
- Tabbed interface with the following tabs: Connections, Backup, Restore, Copy, Jobs.
- Header includes product name and tagline.

### 6.2 Form Interactions
- Validate required fields (connection details, file paths).
- Show inline feedback for connection testing.
- Disable actions while jobs running.
- Provide success/error toasts or log entries.

### 6.3 Job Progress Panel
- Indeterminate progress bar during operations.
- Scrollable log viewer (monospaced font) with appended entries.
- Display success message with artifact path when complete.

## 7. Data & Storage

| Data Type         | Storage Mechanism | Notes |
|-------------------|-------------------|-------|
| Connections       | localStorage (Phase 1) | Extended config storage planned for Phase 2 (SQLite/local file). |
| Passwords         | OS keychain via `keyring` crate | Identified by connection ID. |
| Jobs history      | localStorage (Phase 1); future persistence TBD. |
| Backup artifacts  | File system (`~/dbpinball/backups/<job-id>/`). |

## 8. Security Considerations
- No plaintext credential storage.
- Avoid logging sensitive information.
- Provide TLS flag in connection settings.
- Future enhancement: support custom CA certificates.

## 9. Testing Strategy

### 9.1 Automated (Phase 2)
- Integration tests with Docker Compose for MySQL & PostgreSQL.
- Validate backup/restore/copy round trips on large datasets (~1M rows).
- Compare row counts & checksums post-operation.
- Mock Tauri event emitter for unit testing progress logs.

### 9.2 Manual Testing (Phase 1)
- Verify vendor tool invocation on each OS.
- Ensure backup artifacts created and accessible.
- Validate restore options (drop schema, stop on error).
- Confirm connection testing works and handles invalid credentials gracefully.

## 10. Roadmap & Phases

| Phase | Scope |
|-------|-------|
| **Phase 1 (Current)** | Core backup/restore/copy, connection vault, job history, vendor tool fallback. |
| **Phase 2** | SSH tunneling, cross-engine copy with type mapping, parallel dumps, resumable jobs, automated tests. |
| **Phase 3** | Scheduled backups, cloud storage targets, web UI companion, database diff tooling. |

## 11. Success Metrics
- Time to complete a typical backup < 1 minute for small DBs.
- Restore success rate ≥ 99% on standard dumps.
- Users can set up first backup within 5 minutes (measured via onboarding tests).
- Zero critical security incidents (password leaks).

## 12. Open Questions
- Should job history persist beyond localStorage (e.g., SQLite) in early phases?
- Do we prioritize SSH tunneling or cross-engine copy first?
- How should backup scheduling integrate with system cron or remain manual in MVP?

---
_Last updated: 2025-11-11_
