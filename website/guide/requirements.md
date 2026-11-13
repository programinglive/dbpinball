# Requirements

## To run the desktop application

| Requirement | Version |
| --- | --- |
| Node.js | 20.19+ or 22.12+ (required by Vite 7) |
| Rust | 1.77.2+ (Tauri 2 minimum supported Rust version) |
| Platform build dependencies | See the [Tauri prerequisites](https://tauri.app/start/prerequisites/) |
| A database server | MySQL 5.7+/8.0+ or PostgreSQL 11+ |

## To run the CLI

| Requirement | Version |
| --- | --- |
| Rust | 1.77.2+ |
| A database server | MySQL 5.7+/8.0+ or PostgreSQL 11+ |

The CLI does not need Node.js.

## Optional tools

Backup, restore and copy use the vendor client tools when they are available on
`PATH`:

- `mysqldump`, `mysql` for MySQL;
- `pg_dump`, `psql` for PostgreSQL.

The desktop backup, restore and copy pages are **not routed in the current UI**,
so these tools are not needed for the documented workflows.

## Supported platforms

| Platform | Status |
| --- | --- |
| Windows | Supported; the desktop and CLI are developed and tested here |
| Linux | Supported; CI runs the Rust and frontend suites on Linux |
| macOS | Supported by the code; not exercised in CI |

See [Platforms](/security/platforms) for the exact verification status.

## Database engines

| Engine | Versions | Status |
| --- | --- | --- |
| MySQL | 5.7+, 8.0+ | Connection, listing and query available |
| PostgreSQL | 11+ | Connection, listing and query available |

DBPinball connects to database engines, not to application frameworks, so any
project using MySQL or PostgreSQL is compatible.
