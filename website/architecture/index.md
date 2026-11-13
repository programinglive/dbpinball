# System overview

DBPinball is a Rust core with two front ends and one shared store.

```text
Desktop (React)                CLI (dbpinball)
      │                              │
      │ invoke(...)                  │ connections / schema
      ▼                              ▼
Tauri adapter (thin)          ──►  dbpinball-core
      │                              │
      └──────────► ConnectionStore ◄─┘
                   (single source of truth)
                        │
                        ├── connections.json  (versioned, atomic, locked)
                        └── OS keyring        (service "dbpinball", account = id)
```

## Crates and packages

| Path | Role |
| --- | --- |
| `crates/dbpinball-core` | Headless domain: connection model, store, keyring adapter, inspection services |
| `crates/dbpinball-cli` | The `dbpinball` binary |
| `src-tauri` | The Tauri desktop shell and its commands |
| `src` | The React UI |
| `packages/laravel-adapter` | A separate PHP package for Laravel backups |

## The rule that keeps it honest

The core must stay headless. `scripts/check-boundaries.cjs` fails the build if
`dbpinball-core` depends on `tauri` or `clap`, directly or transitively. That is
what stops database logic from leaking back into the desktop shell.

## Two capability families

The design separates two things that are easy to confuse:

- **Administrative operations** — raw backup, restore, copy and database
  management. Byte-faithful and unsanitized.
- **Safe Sync** — policy-controlled, selective, sanitized reproduction of
  production data into a verified development target.

A raw database copy is not sanitized and is not safe to share. Safe Sync is the
only capability intended to produce shareable development data, and it is **not
implemented**.

## Read next

- [Rust core](/architecture/core)
- [Tauri desktop](/architecture/desktop)
- [CLI](/architecture/cli)
- [Connection store](/architecture/connection-store)
