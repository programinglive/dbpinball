# Profile schema

## `DbConnection`

```json
{
  "id": "0f0c2f7e-...",
  "name": "Analytics",
  "engine": "postgres",
  "host": "analytics.example.com",
  "port": 5432,
  "database": "analytics",
  "username": "analyst",
  "useSsl": true,
  "target": "unknown"
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string | Stable; the keyring account |
| `name` | string | Display name |
| `engine` | `mysql` \| `postgres` | Lowercase |
| `host` | string | Hostname or IP |
| `port` | number | 1–65535 |
| `database` | string | Database name |
| `username` | string | Database user |
| `useSsl` | boolean | Stored; not currently applied to connections |
| `target` | `production` \| `staging` \| `local` \| `unknown` | User hint only |

## `StoreFile`

```json
{
  "version": 1,
  "migratedFrom": "localStorage@v1",
  "migrationReport": {
    "imported": 2,
    "skippedInvalid": [],
    "skippedDuplicates": 0,
    "skippedConflicts": []
  },
  "revision": 7,
  "connections": []
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `version` | number | Must be 1 |
| `migratedFrom` | string? | Provenance, written once |
| `migrationReport` | object? | Last migration result |
| `revision` | number | Monotonic write counter |
| `connections` | array | The profiles |

## `ImportReport`

| Field | Type | Meaning |
| --- | --- | --- |
| `imported` | number | Records added |
| `skippedInvalid` | array | `{ index, reason }` |
| `skippedDuplicates` | number | Records already present |
| `skippedConflicts` | array | `{ index, id, reason }` |

## Validation rules

A record is rejected when:

- `id` is empty;
- `name` is empty;
- `host` is empty;
- `database` is empty;
- `username` is empty;
- `port` is 0.

## Deduplication

Two records are the same profile when `engine`, `host`, `port`, `database` and
`username` all match. A record with an existing `id` but different details is a
**conflict**: the shared record wins and the import is reported.

## Related

- [Connection store](/architecture/connection-store)
- [Configuration reference](/reference/configuration)
