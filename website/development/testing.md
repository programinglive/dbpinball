# Testing

## Rust

```bash
cargo test --workspace
```

This runs:

- core unit tests (store, lock, import, migration, credential lifecycle);
- CLI unit and contract tests;
- cross-process store tests that spawn the real binary;
- Tauri adapter tests.

## Frontend

```bash
npm test
```

Vitest runs the React tests, including the connection vault and the migration
report.

## Database integration

The `Database integration` CI job runs the core and CLI integration tests against
disposable PostgreSQL and MySQL service containers. It is not part of the default
local run.

## Documentation site

```bash
cd website
npm run build
```

The build fails on a broken internal link.

## What the tests pin

| Area | Representative tests |
| --- | --- |
| Store round trip | `save_and_load_round_trip` |
| No password in the store | `serialized_store_never_contains_a_password` |
| Atomic write | `atomic_write_leaves_no_temporary_file` |
| Import provenance | `import_sets_provenance_and_unknown_target` |
| Idempotent migration | `repeated_migration_is_idempotent` |
| Conflict reporting | `a_conflicting_migration_is_not_reported_as_complete` |
| Revision conflict | `a_stale_revision_is_a_conflict` |
| Lock liveness | `a_live_owner_is_not_stolen` |
| Cross-process | `two_processes_do_not_lose_each_others_import` |
| Credential coordination | `a_stale_revision_does_not_touch_the_credential` |

## Related

- [CI checks](/development/ci)
- [Contributing](/development/contributing)
