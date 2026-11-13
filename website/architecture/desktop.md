# Tauri desktop

The desktop is a Tauri 2 shell around a React UI. It is a thin adapter: it
resolves the shared directory, calls the core, and maps errors to safe messages.

## Connection commands

| Command | Input | Output |
| --- | --- | --- |
| `connections_list` | — | `DbConnection[]` |
| `connections_create` | `connection`, `password?` | `DbConnection` |
| `connections_update` | `connection`, `password?`, `expectedRevision?` | — |
| `connections_delete` | `id`, `expectedRevision?` | `boolean` |
| `connections_import` | `records` | `ImportReport` |
| `connections_migrate_local_storage` | `records` | `ImportReport` |
| `connections_migration_status` | — | `MigrationStatus` |

## Database commands

| Command | Purpose |
| --- | --- |
| `test_conn` | Test a connection |
| `list_databases_cmd` | List databases |
| `list_tables_cmd` | List tables in a database |
| `run_query_cmd` | Run SQL and return a result grid |
| `backup`, `restore`, `copy_db` | Dump and copy operations (pages not routed) |

## Credential commands

| Command | Purpose |
| --- | --- |
| `store_password` | Store a password in the keyring |
| `get_password` | Read a password from the keyring |
| `delete_password` | Delete a password from the keyring |

The connection commands use the core's coordinated mutations, so the credential
and the metadata are written under one store lock. The older credential commands
remain for compatibility.

## Error mapping

The adapter never forwards a raw core message. It maps the error code to a safe
message:

| Code | Message |
| --- | --- |
| `Conflict` | The connection list changed in another window. Reload and try again. |
| `Busy` | The connection store is busy. Try again in a moment. |
| `NotFound` | That connection no longer exists. |
| `Connection` | The operating system keyring is unavailable for this connection. |
| `Unsupported` | This connection store version is not supported by this build. |
| `Operation` | The connection store could not be updated. |

This keeps filesystem paths and credentials out of the UI.

## Related

- [Desktop basics](/guide/desktop)
- [Credential lifecycle](/architecture/credential-lifecycle)
