# Requirements

## To run DBPinball

| Requirement | Notes |
| --- | --- |
| Windows 10/11 (x64) or Linux (x64) | macOS is supported by the code but not packaged yet |
| A database server | MySQL 5.7+/8.0+, MariaDB 10.6+/10.11+, or PostgreSQL 11+ |

Installing DBPinball does **not** require Rust, Cargo, Node.js or a source
checkout. See [Installation](/guide/installation).

## To build DBPinball from source

| Requirement | Version |
| --- | --- |
| Node.js | 20.19+ or 22.12+ (required by Vite 7) |
| Rust | 1.77.2+ (Tauri 2 minimum supported Rust version) |
| Platform build dependencies | See the [Tauri prerequisites](https://tauri.app/start/prerequisites/) |

See [Building from source](/development/building).

## Optional tools

Backup, restore and copy use the vendor client tools when they are available on
`PATH`:

- `mysqldump`, `mysql` for MySQL and MariaDB;
- `pg_dump`, `psql` for PostgreSQL.

The desktop backup, restore and copy pages are **not routed in the current UI**,
so these tools are not needed for the documented workflows. `dbpinball doctor`
reports whether they are present.

## Supported platforms

| Platform | Status |
| --- | --- |
| Windows | Supported; the desktop installer and CLI archive are built and verified |
| Linux | Supported; CI builds the CLI archive and the `.deb`/AppImage packages |
| macOS | Supported by the code; not packaged or exercised in CI |

See [Platforms](/security/platforms) for the exact verification status.

## Database engines

| Engine | Versions | Status |
| --- | --- | --- |
| MySQL | 5.7+, 8.0+ | Connection, listing and query available |
| MariaDB | 10.6+, 10.11+ | Connection, listing, query and Safe Sync available |
| PostgreSQL | 11+ | Connection, listing, query and Safe Sync available |

DBPinball connects to database engines, not to application frameworks, so any
project using MySQL, MariaDB or PostgreSQL is compatible.
