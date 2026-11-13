# Connection store

The connection store is the single source of truth for profiles. The desktop and
the CLI both read and write it.

## Location

`dirs::config_dir()/dbpinball/connections.json`

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\dbpinball\connections.json` |
| macOS | `~/Library/Application Support/dbpinball/connections.json` |
| Linux | `~/.config/dbpinball/connections.json` |

## File format

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
  "connections": [
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
  ]
}
```

- `version` is the only supported schema version.
- `migratedFrom` records provenance and is written once.
- `migrationReport` keeps the last migration result so unresolved items survive a
  restart.
- `revision` is a monotonic counter used for optimistic concurrency.
- **No password field exists.** A test asserts the key is absent.

## Writes

Every mutation:

1. acquires the cross-process lock;
2. re-reads the file inside the lock;
3. checks the expected revision, if one was supplied;
4. writes a temp file and renames it into place;
5. bumps the revision.

On Unix the temp file is restricted to `0600`.

## Reads

Reads do not take the lock. A read may see the previous revision while a write is
in progress; the atomic rename means it never sees a half-written file.

## Related

- [Revision consistency](/architecture/revision-consistency)
- [Profile schema](/reference/profile-schema)
