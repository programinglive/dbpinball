# DBPinball

**CLI-First Database Developer Workspace**

Inspect, manage, and reproduce database issues safely across development environments.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## What is DBPinball?

DBPinball is an open-source desktop workspace for MySQL and PostgreSQL. It gives you a TablePlus-style interface for browsing databases, inspecting tables, and running queries, with a connection vault that keeps passwords in the operating system keyring instead of on disk.

The project is being rebuilt around a **core-first, CLI-first** architecture: a framework-agnostic Rust core, a `dbpinball` command-line interface as the primary contract, and the desktop app as a thin adapter over the same services. That architecture is a **target**, not a finished state. This README separates what works today from what is planned.

## Why DBPinball?

- **Local-first.** Connections and credentials stay on your machine. Passwords go to the OS keyring.
- **Engine-level, not framework-level.** DBPinball talks to MySQL and PostgreSQL directly, so it works with any application built on them.
- **Headless by design.** The roadmap moves all database logic into a reusable Rust core so the same capability can run from a CLI, a desktop app, or CI.
- **Safe Sync as a first-class goal.** Reproducing production data for debugging should be selective, sanitized, and impossible to point at production by accident. That capability is planned, not shipped.

## Current features

### Available and verified

| Capability | Status | Notes |
| --- | --- | --- |
| Connection vault | Available | Create, edit, test, and delete named MySQL/PostgreSQL connections |
| Credential storage | Available | Passwords stored in the OS keyring (Keychain, Credential Manager, libsecret) |
| Workspace overlay | Available | Full-screen TablePlus-style workspace for a selected connection |
| Database listing | Available | Lists databases on the connected server |
| Table listing | Available | Lists tables in the selected database |
| Query execution | Available | Runs SQL and renders results in a grid with column resizing and horizontal scroll |
| Window controls | Available | Custom title bar with minimize, maximize, close, and fullscreen |
| `dbpinball` CLI | Available | `doctor`, `connections list/add/set-password/test/remove/import`, `schema inspect`; JSON output and stable exit codes |

Verified by the frontend test suite (11 tests across 4 files), the Rust workspace tests (23 core, 19 CLI), and real PostgreSQL/MySQL integration tests.

### In development

These exist in the codebase but are **not reachable from the current UI**, or are known to be incomplete. Do not rely on them.

| Capability | State |
| --- | --- |
| Backup | Tauri command exists; the Backup page is not routed in `App.tsx` |
| Restore | Tauri command exists; the Restore page is not routed |
| Copy | Tauri command exists; the Copy page is not routed |
| Job history | Page exists; not routed |
| Internal dumper fallback | **Incomplete.** It creates an empty file and reports success when `mysqldump`/`pg_dump` is unavailable |
| `drop_existing` / `stop_on_error` | Accepted but ignored |
| Compression | Uses deflate (`.sql.deflate`), not ZIP, despite the UI label |
| TLS toggle | `useSsl` is stored but not applied to connections |

### Planned

| Capability | Milestone |
| --- | --- |
| Shared security policy and destination verification | M3 |
| PostgreSQL Safe Sync (sanitized production → development) | M4 |
| Desktop Safe Sync integration | M5 |
| MySQL Safe Sync | M6 |
| Advanced snapshots and relational dependency resolution | M7 |

Safe Sync is **not available**. No command in this README should be assumed to exist until the corresponding milestone ships.

## Monorepo Packages

- **Desktop Workspace (`apps/desktop` / root)**: Tauri + React interactive database manager and local dump runners.
- **Laravel Adapter ([`packages/laravel-adapter`](docs/monorepo-laravel-adapter.md))**: Server-side Laravel package (formerly `laravel-simple-backup`) providing the `backup:database` Artisan command and a `BackupManager` for MySQL and PostgreSQL dumps.

## Architecture

### Current implementation

```text
React (src/)  ──invoke()──>  Tauri commands (src-tauri/src/lib.rs)
                                      │
                                      ▼
                          runners/ (mysql.rs, postgres.rs)
                                      │
                                      ├── OS keyring (secrets.rs)
                                      ├── sqlx connection pools
                                      └── mysqldump / pg_dump / mysql / psql
```

All database logic currently lives inside the Tauri crate and imports `tauri::Window`, so it cannot run headless.

### Target architecture

```text
DBPinball Core (Rust)
        │
        ├── CLI
        │
        ├── Desktop (Tauri + React)
        │
        └── Future integrations

Database adapters: PostgreSQL, MySQL, and future engines
```

The target is documented in [`docs/superpowers/specs/2026-11-12-dbpinball-cli-first-architecture-design.md`](docs/superpowers/specs/2026-11-12-dbpinball-cli-first-architecture-design.md). It is a design, not a description of the current code.

### Two distinct capability families

The design separates two things that are easy to confuse:

- **Administrative operations** — raw backup, restore, copy, and database management. Byte-faithful, unsanitized.
- **Safe Sync** — policy-controlled, selective, sanitized reproduction of production data into a verified development target.

**A raw database copy is not sanitized and is not safe to share.** Safe Sync is the only capability intended to produce shareable development data, and it is not implemented yet.

## Supported database engines

| Engine | Versions | Status |
| --- | --- | --- |
| MySQL | 5.7+, 8.0+ | Connection, listing, and query available |
| PostgreSQL | 11+ | Connection, listing, and query available |

Backup and restore additionally require the vendor client tools (`mysqldump`, `mysql`, `pg_dump`, `psql`) on `PATH`.

## Framework compatibility

DBPinball is framework-agnostic at its core. It connects to database engines, not to application frameworks, so any project using MySQL or PostgreSQL is compatible.

A **Laravel adapter** ships in this repository at [`packages/laravel-adapter`](docs/monorepo-laravel-adapter.md). It provides the `backup:database` Artisan command and a `BackupManager` for MySQL and PostgreSQL dumps. It is a separate package with its own `composer.json` and PHP test suite. Retention pruning and cloud storage drivers are **not implemented**.

No other framework-specific adapter ships here. There is no Rails, Django, or Prisma integration, and none is claimed.

## Desktop quick start

### Prerequisites

- **Node.js 20.19+ or 22.12+** (required by Vite 7)
- **Rust 1.77.2+** (Tauri 2 minimum supported Rust version)
- A MySQL or PostgreSQL server for testing
- Platform build dependencies for Tauri: see <https://tauri.app/start/prerequisites/>

### Run from source

```bash
git clone https://github.com/programinglive/dbpinball.git
cd dbpinball
npm install
npm run tauri dev
```

### Build installers

```bash
npm run tauri build
```

Installers are written to `src-tauri/target/release/bundle/`.

### First connection

1. Launch the app.
2. Use the **+** toolbar button and choose **Add MySQL Connection** or **Add PostgreSQL Connection**.
3. Enter host, port, database, username, and password.
4. Test the connection, then save it.
5. Double-click the connection to open the workspace.

## CLI quick start

The `dbpinball` CLI is available for read-only inspection. It shares the same Rust core as the desktop app.

```bash
# Build and run from the workspace
cargo run -p dbpinball-cli -- --help

# Check the local environment
cargo run -p dbpinball-cli -- doctor --json

# Add a profile; the password is read from stdin and stored in the OS keyring
cargo run -p dbpinball-cli -- connections add \
  --name local-dev --engine postgres --host 127.0.0.1 --port 5432 \
  --database app --username postgres --target local

# List profiles (never prints passwords) and test one
cargo run -p dbpinball-cli -- connections list --json
cargo run -p dbpinball-cli -- connections test <id> --json

# Inspect databases, or tables in one database
cargo run -p dbpinball-cli -- schema inspect --connection <id> --json
cargo run -p dbpinball-cli -- schema inspect --connection <id> --database app --json
```

Global options: `--json`, `--config-dir <path>`, `--credential-source <keyring|env>`.
For CI, `--credential-source env` reads `DBPINBALL_PASSWORD`; it never falls back to plaintext storage.

The CLI is **read-only at the database layer**. It does not run arbitrary SQL, write data, back up, restore, or copy. See [`docs/cli.md`](docs/cli.md) for the full contract.

## Security and data-handling principles

- Passwords are stored in the OS keyring, never in the repository or in plaintext configuration.
- Connection metadata is stored locally; it is not transmitted anywhere.
- The app does not phone home, and there is no telemetry.
- Raw backups contain real data. Treat them as sensitive.

### Known limitations

These are documented defects, not guarantees:

- The workspace query runner executes arbitrary SQL, including writes. There is no read-only guard yet.
- Passwords are interpolated into connection URLs internally, which can break on special characters and risks appearing in error text.
- The TLS toggle is stored but not applied.
- The internal dumper fallback produces an empty file instead of failing.

Do not use DBPinball against production data until the security milestones land. See [`SECURITY.md`](SECURITY.md) for the full boundary description.

## Development

### Setup

```bash
npm install
```

### Run

```bash
npm run tauri dev     # desktop app with hot reload
npm run dev           # frontend only, in a browser
```

### Test

```bash
npm test                          # frontend (Vitest)
cd src-tauri && cargo test        # Rust
```

The Rust test suite currently contains **0 tests**. The frontend suite contains 11.

### Lint and type-check

```bash
npm run lint                      # tsc --noEmit
cd src-tauri && cargo clippy
```

### Project layout

```text
dbpinball/
├── src/                      React + TypeScript frontend
│   ├── pages/                connections, backup, restore, copy, jobs
│   ├── components/           workspace and shadcn/ui components
│   └── hooks/                workspace query and window hooks
├── src-tauri/                Rust backend
│   ├── src/lib.rs            Tauri commands
│   ├── src/models.rs         shared data models
│   ├── src/secrets.rs        OS keyring access
│   └── src/runners/          MySQL and PostgreSQL implementations
├── crates/dbpinball-core/    Headless Rust core (no Tauri, no CLI parser)
├── docs/                     architecture, audit, and release notes
└── scripts/                  pre-commit test verification
└── packages/laravel-adapter/ Laravel adapter package (PHP)
```

## Testing and validation

| Suite | Command | Current result |
| --- | --- | --- |
| Frontend | `npm test` | 4 files, 11 tests passing |
| Rust core | `cargo test -p dbpinball-core` | 12 tests passing |
| Rust workspace | `cargo test --workspace` | passing |
| Type check | `npm run lint` | passing |
| Rust format | `cargo fmt --all -- --check` | passing |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` | passing |
| Dependency boundary | `node scripts/check-boundaries.cjs` | passing |
| Pre-commit gate | `node scripts/verify-tests.cjs` | passing |

Database integration tests are gated behind `--features integration` and a disposable stack:

```bash
docker compose -f docker-compose.test.yml up -d --wait
DBPINBALL_TEST_PG_URL=postgres://postgres:dbpinball@127.0.0.1:55432/dbpinball_test \
DBPINBALL_TEST_MYSQL_URL=mysql://root:dbpinball@127.0.0.1:53306/dbpinball_test \
  cargo test -p dbpinball-core --features integration
docker compose -f docker-compose.test.yml down -v
```

CI runs on every push and pull request: frontend lint, tests, and build; Rust format, clippy, and workspace tests; a dependency-boundary check; the Laravel adapter PHP suite; a dedicated `database-integration` job against disposable PostgreSQL 16 and MySQL 8 services; `npm audit`; `cargo audit`; and a secret scan. See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

Dependency status is tracked in [`docs/security/DEPENDENCY_VULNERABILITY_REPORT.md`](docs/security/DEPENDENCY_VULNERABILITY_REPORT.md). Release readiness is tracked in [`docs/RELEASE_READINESS.md`](docs/RELEASE_READINESS.md).

## Roadmap

| Milestone | Scope | State |
| --- | --- | --- |
| M0 | Audit and baseline verification | Complete |
| M1 | Incremental core extraction with desktop compatibility | Complete |
| M2 | CLI foundation and contract tests | Complete |
| M2.1 | Desktop import of `localStorage` profiles into the shared store | Planned |
| M3 | Shared security policy and destination verification | Planned |
| M4 | PostgreSQL Safe Sync through the CLI | Planned |
| M5 | Desktop Safe Sync integration | Planned |
| M6 | MySQL Safe Sync | Planned |
| M7 | Advanced snapshots and relational dependency resolution | Planned |

Deferred: cross-engine copy, SSH tunnels, scheduled backups, cloud storage.

## Contributing

Contributions are welcome. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a pull request.

Two rules matter more than the rest:

1. **Never use real production data** in issues, pull requests, fixtures, or test artifacts. Use synthetic datasets and disposable databases.
2. **Keep the desktop app working.** Command names and payloads are part of the compatibility contract.

## Security reporting

Do not open a public issue for a vulnerability. See [`SECURITY.md`](SECURITY.md) for the disclosure process and the current reporting channel.

## License

This project is licensed under the [MIT License](LICENSE).

Third-party attributions are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
