# Repository structure

```text
dbpinball/
├── crates/
│   ├── dbpinball-core/        # Headless domain, store, keyring port, services
│   └── dbpinball-cli/         # The `dbpinball` binary
├── src-tauri/                 # Tauri shell and commands
├── src/                       # React UI
├── packages/
│   └── laravel-adapter/       # Separate PHP package
├── website/                   # This documentation site (VitePress)
├── docs/                      # Design notes, plans and audits
├── scripts/                   # Boundary check and tooling
└── .github/workflows/         # CI
```

## Key files

| File | Why it matters |
| --- | --- |
| `crates/dbpinball-core/src/store/connection_store.rs` | The store and its mutations |
| `crates/dbpinball-core/src/store/lock.rs` | Cross-process lock |
| `crates/dbpinball-core/src/store/record.rs` | Store schema |
| `crates/dbpinball-cli/src/cli.rs` | Every CLI command and flag |
| `src-tauri/src/connections.rs` | The desktop adapter |
| `src-tauri/src/lib.rs` | Tauri command registration |
| `src/pages/connections.tsx` | The connection vault UI |
| `scripts/check-boundaries.cjs` | The headless-core rule |

## Related

- [Architecture](/architecture/)
- [Testing](/development/testing)
