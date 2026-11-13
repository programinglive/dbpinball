# Adapter boundaries

The core is headless. Everything that touches a UI, a terminal or a specific
runtime lives in an adapter.

## The boundary check

`scripts/check-boundaries.cjs` fails the build if `dbpinball-core` declares or
resolves a dependency on `tauri` or `clap`. It checks both the manifest and the
resolved dependency graph.

```bash
node scripts/check-boundaries.cjs
```

This is what stops database logic from leaking back into the desktop shell.

## What each layer may do

| Layer | May | May not |
| --- | --- | --- |
| `dbpinball-core` | Domain types, store, keyring port, inspection services | Depend on Tauri, Clap, or terminal I/O |
| `dbpinball-cli` | Parse arguments, print output, map exit codes | Contain store or validation logic |
| `src-tauri` | Resolve the shared directory, call core, map errors | Duplicate validation or deduplication |
| `src` (React) | Render, call Tauri commands | Read or write the store directly |

## Database adapters

The core talks to MySQL and PostgreSQL through `sqlx`. The engine is part of the
connection model, so a new engine is a new adapter behind the same services, not
a new store.

## Related

- [Rust core](/architecture/core)
- [CI checks](/development/ci)
