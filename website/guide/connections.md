# Managing connections

Connection profiles are shared by the CLI and the desktop. This page covers the
operations both front ends expose.

## List

```bash
dbpinball connections list
dbpinball connections list --json
```

The list never prints passwords. The JSON form is a single object:

```json
{ "connections": [ { "id": "...", "name": "...", "engine": "postgres", "host": "...", "port": 5432, "database": "...", "username": "...", "useSsl": false, "target": "unknown" } ] }
```

## Add

```bash
dbpinball connections add --name <name> --engine <mysql|postgres> \
  --host <host> --port <port> --database <db> --username <user> \
  [--target <production|staging|local|unknown>] [--use-ssl]
```

The password is read from stdin. There is no `--password` flag.

## Test

```bash
dbpinball connections test <id>
```

## Change the password

```bash
dbpinball connections set-password <id>
```

## Remove

```bash
dbpinball connections remove <id> --yes
```

`--yes` is required when stdin is not a terminal. Without it, the command refuses
with exit code 4 (`policy_refusal`).

## Import

```bash
dbpinball connections import <file>
```

The file may be a JSON array of profiles or a full store object. The importer
validates each record, deduplicates by `(engine, host, port, database, username)`
and reports what it skipped.

## Desktop operations

The desktop exposes the same operations through Tauri commands:

| Operation | Command |
| --- | --- |
| List | `connections_list` |
| Create | `connections_create` |
| Update | `connections_update` |
| Delete | `connections_delete` |
| Import | `connections_import` |
| Migration status | `connections_migration_status` |
| Migrate legacy profiles | `connections_migrate_local_storage` |

The desktop passes the revision it last read on update and delete. If another
process changed the store in the meantime, the operation is refused with a
conflict instead of overwriting the other change.

## What removal does not do

Removing a profile deletes the metadata and the keyring credential. It does not
connect to the database, drop anything, or delete data. DBPinball has no
destructive database operation in the connection-management path.
