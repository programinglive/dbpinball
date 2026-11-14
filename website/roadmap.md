# Roadmap

This page separates what is implemented and verified from what is planned. Do not
assume a planned feature exists.

## Implemented and verified

| Capability | Status |
| --- | --- |
| Connection vault (create, edit, test, delete) | <span class="status status-available">Available</span> |
| Credential storage in the OS keyring | <span class="status status-available">Available</span> |
| Database and table listing | <span class="status status-available">Available</span> |
| Query execution in the desktop workspace | <span class="status status-available">Available</span> |
| `dbpinball` CLI (`doctor`, `connections`, `schema inspect`) | <span class="status status-available">Available</span> |
| Shared connection store between desktop and CLI | <span class="status status-available">Available</span> |
| Cross-process lock and revision consistency | <span class="status status-available">Available</span> |
| Desktop migration from `localStorage` | <span class="status status-available">Available</span> |
| Shared security policy and destination verification (M3) | <span class="status status-available">Available</span> |

## M2.1 status

M2.1 is the desktop connection store migration. It is merged.

What it does:

- the desktop reads and writes the shared store instead of `localStorage`;
- legacy `localStorage` profiles are imported once, with a report;
- `localStorage` is kept as a read-only rollback artifact for one release;
- create, update and delete coordinate the keyring and the metadata under one
  store lock.

## Planned

| Capability | Milestone |
| --- | --- |
| Shared security policy and destination verification | M3 — **implemented and merged** (see [Security policy](/security/policy)) |
| PostgreSQL Safe Sync (sanitized production → development) | M4 — **design approved, not implemented** (see [Safe Sync design](/security/safe-sync-design)) |
| Desktop Safe Sync integration | M5 |
| MySQL Safe Sync | M6 |
| Advanced snapshots and relational dependency resolution | M7 |

## Known limitations

- Backup, restore and copy commands exist, but their pages are not routed in the
  desktop UI.
- The internal dumper fallback is incomplete: it can create an empty file and
  report success when the vendor tool is missing.
- `drop_existing` and `stop_on_error` are accepted but ignored.
- Compression uses deflate (`.sql.deflate`), not ZIP, despite the UI label.
- `useSsl` is stored but not applied to connections.
- macOS is not exercised in CI.
- Cross-system crash atomicity between the store and the keyring is not claimed.

## Not available

Safe Sync is **not available**. No command or page in this documentation should
be assumed to exist until the corresponding milestone ships.

## Related

- [Limitations](/security/limitations)
- [Architecture](/architecture/)
