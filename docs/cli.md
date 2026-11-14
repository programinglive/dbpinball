# dbpinball CLI

The `dbpinball` binary is a headless, read-only interface over the shared
`dbpinball-core` services. The desktop app and the CLI run the same inspection
code.

## Build and run

```bash
cargo build -p dbpinball-cli
./target/debug/dbpinball --help
```

The release build is a single binary with no Tauri, React, or webview
dependency.

## Commands

### `dbpinball doctor`

Checks the local environment: version, configuration directory, connection
store, keyring probe, and whether `mysqldump` / `pg_dump` are on `PATH`.

```bash
dbpinball doctor
dbpinball doctor --json
```

A `warn` does not fail the command. A `fail` exits 5.

### `dbpinball connections list`

Lists stored profiles. Passwords are never stored in the profile file and are
never printed.

```bash
dbpinball connections list
dbpinball connections list --json
```

### `dbpinball connections add`

Creates a profile. The password is read from **stdin**; there is no
`--password` flag.

```bash
printf '%s' "$DB_PASSWORD" | dbpinball connections add \
  --name local-dev \
  --engine postgres \
  --host 127.0.0.1 \
  --port 5432 \
  --database app \
  --username postgres \
  --target local
```

On a terminal, the password prompt hides input. When stdin is not a terminal,
exactly one line is read.

### `dbpinball connections set-password <id>`

Replaces the stored password. Reads stdin the same way as `add`.

### `dbpinball connections test <id>`

Opens the connection once through the core service.

```bash
dbpinball connections test local-dev --json
```

### `dbpinball connections remove <id>`

Removes the profile and its keyring credential.

- Interactive: prints the connection name and asks for confirmation.
- Non-interactive: requires `--yes`.
- Missing `--yes` in non-interactive mode exits 4 (`policy_refusal`).
- The database is never contacted, dropped, or modified.

### `dbpinball connections import <file>`

Imports profiles from a JSON array or a store-shaped object. Invalid records are
dropped with an index and reason; duplicates by
`(engine, host, port, database, username)` are skipped.

### `dbpinball schema inspect`

```bash
# Databases on the server
dbpinball schema inspect --connection <id> --json

# Tables in one database
dbpinball schema inspect --connection <id> --database app --json
```

## Global options

| Option | Meaning |
| --- | --- |
| `--json` | Emit exactly one JSON object on stdout |
| `--config-dir <path>` | Override the configuration directory (tests, advanced use) |
| `--credential-source <keyring\|env>` | Where the password comes from |

## JSON envelope

```json
{ "ok": true,  "data": { }, "error": null }
{ "ok": false, "data": null, "error": { "code": "connection", "message": "..." } }
```

- Exactly one object on stdout with `--json`.
- Diagnostics and warnings go to stderr.
- Error messages are redacted; a password never appears.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | Usage error |
| 3 | Connection or credential failure |
| 4 | Policy refusal |
| 5 | Operation or environment failure |
| 6 | Safe Sync committed the rows but the identity sequence needs reconciliation; re-run execute |
| 7 | Unknown connection profile |
| 8 | Store conflict: another writer changed the store since it was read |
| 9 | Store busy: the cross-process lock was not acquired within the bounded wait |

## Configuration and credentials

| Purpose | Location |
| --- | --- |
| Configuration root | `dirs::config_dir()/dbpinball/` |
| Profiles | `<config root>/connections.json` |
| Keyring service | `dbpinball` |
| Keyring account | connection id |

`connections.json` contains metadata only. The file is written atomically and
restricted to `0600` on Unix. It has no password field.

The desktop application uses the same file and the same path resolver. A profile
created in the CLI appears in the desktop and vice versa. Every mutation takes a
cross-process lock, so the two cannot silently overwrite each other. See
[connection-store.md](connection-store.md).

### Credential providers

- **Keyring** (default): the OS keyring. If it is unavailable, the command fails
  with exit 3; there is no plaintext fallback.
- **Env** (`--credential-source env`): reads `DBPINBALL_PASSWORD`. Intended for
  CI and automation. It cannot store or delete passwords.

## Read-only guarantee

M2 does not run arbitrary SQL, write data, back up, restore, copy, or run Safe
Sync. The only SQL the CLI executes is the fixed inspection set in
`dbpinball-core`:

- `SELECT 1`
- `SELECT schema_name FROM information_schema.schemata ...`
- `SELECT table_name FROM information_schema.tables ...`
- `SELECT datname FROM pg_database ...`
- `SELECT table_name FROM information_schema.tables ...`

## Target classification

`--target` records a user-provided label: `production`, `staging`, `local`, or
`unknown` (default). It is metadata only. It is **not** proof that a database is
local or safe to write to, and M2 never uses it to grant privileges.

## Desktop profile migration (M2.1)

The desktop app still stores profiles in webview `localStorage`. The CLI uses
`connections.json`. M2.1 will add a desktop command that imports the
`localStorage` profiles into the shared store, idempotently and with a rollback
path. Until then, use `connections add` or `connections import` for CLI
profiles.

## Limitations

- Read-only: no query execution, writes, backup, restore, or copy.
- Linux keyring requires a Secret Service; use `--credential-source env` where
  it is unavailable.
