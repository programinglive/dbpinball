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

Verified by the frontend test suite (10 tests across 3 files) and manual use.

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
| `dbpinball` CLI | M2 |
| Shared security policy and destination verification | M3 |
| PostgreSQL Safe Sync (sanitized production → development) | M4 |
| Desktop Safe Sync integration | M5 |
| MySQL Safe Sync | M6 |
| Advanced snapshots and relational dependency resolution | M7 |

The CLI and Safe Sync are **not available**. No command in this README should be assumed to exist until the corresponding milestone ships.

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

DBPinball is framework-agnostic. It connects to database engines, not to application frameworks, so any project using MySQL or PostgreSQL is compatible.

No framework-specific adapter ships in this repository. There is no Laravel, Rails, Django, or Prisma integration here, and none is claimed.

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

**Not available yet.** The `dbpinball` binary is planned for milestone M2. There is no CLI in this repository today, and no command below should be assumed to work.

When it ships, the intended shape is:

```bash
dbpinball doctor
dbpinball connections list --json
dbpinball schema inspect --connection <id> --json
dbpinball query run --connection <id> --database <db> --sql "select 1" --read-only --json
```

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

The Rust test suite currently contains **0 tests**. The frontend suite contains 10.

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
├── docs/                     architecture, audit, and release notes
└── scripts/                  pre-commit test verification
```

## Testing and validation

| Suite | Command | Current result |
| --- | --- | --- |
| Frontend | `npm test` | 3 files, 10 tests passing |
| Rust | `cd src-tauri && cargo test` | 0 tests |
| Type check | `npm run lint` | passing |
| Rust format | `cd src-tauri && cargo fmt --all -- --check` | passing |
| Rust lint | `cd src-tauri && cargo clippy --all-targets -- -D warnings` | passing |
| Pre-commit gate | `node scripts/verify-tests.cjs` | passing |

CI runs on every push and pull request: frontend lint, tests, and build; Rust format, clippy, and tests; `npm audit`; `cargo audit`; and a secret scan. See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

Dependency status is tracked in [`docs/security/DEPENDENCY_VULNERABILITY_REPORT.md`](docs/security/DEPENDENCY_VULNERABILITY_REPORT.md). Release readiness is tracked in [`docs/RELEASE_READINESS.md`](docs/RELEASE_READINESS.md).

## Roadmap

| Milestone | Scope | State |
| --- | --- | --- |
| M0 | Audit and baseline verification | Complete |
| M1 | Incremental core extraction with desktop compatibility | Planned |
| M2 | CLI foundation and contract tests | Planned |
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

MIT. See [`LICENSE`](LICENSE).

Third-party attributions are listed in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
